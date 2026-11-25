//! Only the agent dispatcher supplies this channel. The VM has no registry execution handle.
use crate::llm::{ToolCallRequest, ToolDefinition};
use crate::tools::ToolRegistry;
use mlua::{Lua, LuaSerdeExt, LuaString, Table, Value};
use std::{collections::BTreeMap, sync::Arc, time::Instant};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// Discover from the scoped registry and recheck the actual arguments before
/// dispatch. A text read and a native image disclosure have different profiles.
pub(crate) fn supports(registry: &ToolRegistry, name: &str, args: &serde_json::Value) -> bool {
    let Some(tool) = registry.get(name) else {
        return false;
    };
    if tool.canonical_identity_for_arguments(args).is_some() || tool.ownership_selector() != name {
        return false;
    }
    if name == "workspace_rules" && args.get("action").is_some_and(|action| action != "read") {
        return false;
    }
    let access = tool.access_profile(args);
    access.can_read
        && !access.can_write
        && !access.can_execute
        && !access.can_access_network
        && matches!(
            access.category.as_str(),
            "filesystem" | "document_analysis" | "knowledge" | "knowledge_health" | "memory"
        )
}

pub(crate) struct Request {
    pub calls: Vec<ToolCallRequest>,
    pub cancel: CancellationToken,
    pub reply: oneshot::Sender<Result<Vec<String>, String>>,
}

#[derive(Clone)]
pub(crate) struct Bridge {
    sender: mpsc::Sender<Request>,
    definitions: Arc<BTreeMap<String, ToolDefinition>>,
}

tokio::task_local! { pub(crate) static BRIDGE: Bridge; }

impl Bridge {
    pub(crate) fn channel(registry: &ToolRegistry) -> (Self, mpsc::Receiver<Request>) {
        let (sender, receiver) = mpsc::channel(2);
        let definitions = registry
            .tool_names()
            .into_iter()
            .filter(|name| supports(registry, name, &serde_json::Value::Null))
            .filter_map(|name| registry.definition(&name))
            .map(|definition| (definition.name.clone(), definition))
            .collect();
        (
            Self {
                sender,
                definitions: Arc::new(definitions),
            },
            receiver,
        )
    }

    pub(crate) fn admits(&self, name: &str) -> bool {
        self.definitions.contains_key(name)
    }

    pub(super) fn bind(
        self,
        lua: &Lua,
        env: &Table,
        cancel: CancellationToken,
        deadline: Instant,
    ) -> mlua::Result<()> {
        let names = self.definitions.keys().cloned().collect::<Vec<_>>();
        env.raw_set(
            "list_tools",
            lua.create_function(move |lua, ()| lua.to_value(&names))?,
        )?;
        let schemas = self.definitions.clone();
        env.raw_set(
            "tool_schema",
            lua.create_function(move |lua, name: LuaString| {
                let name = name.to_str()?;
                let definition = schemas
                    .get(name.as_ref())
                    .ok_or_else(|| mlua::Error::runtime("tool unavailable in this cell"))?;
                lua.to_value(&definition.parameters)
            })?,
        )?;
        let single = self.clone();
        let single_cancel = cancel.clone();
        env.raw_set(
            "call_tool",
            lua.create_function(move |lua, (name, args): (LuaString, Option<Table>)| {
                let call = single.prepare(name, args)?;
                let mut result = single.execute(vec![call], &single_cancel, deadline)?;
                lua.create_string(
                    result
                        .pop()
                        .ok_or_else(|| mlua::Error::runtime("missing live tool result"))?,
                )
            })?,
        )?;
        env.raw_set(
            "call_tools",
            lua.create_function(move |lua, batch: Table| {
                let output = lua.create_table()?;
                // Bound simultaneously admitted reads on phones. Larger batches
                // continue in groups and copy directly into the limited VM heap.
                let mut calls = Vec::new();
                for i in 1..=batch.raw_len() {
                    let entry: Table = batch.raw_get(i)?;
                    calls.push(self.prepare(entry.raw_get("name")?, entry.raw_get("arguments")?)?);
                    if calls.len() == 4 || i == batch.raw_len() {
                        for text in self.execute(std::mem::take(&mut calls), &cancel, deadline)? {
                            output.raw_push(lua.create_string(text)?)?;
                        }
                    }
                }
                Ok(output)
            })?,
        )
    }

    fn prepare(&self, name: LuaString, args: Option<Table>) -> mlua::Result<ToolCallRequest> {
        let name = name.to_str()?;
        if !self.admits(name.as_ref()) {
            return Err(mlua::Error::runtime("tool unavailable in this cell"));
        }
        let mut remaining = super::code_mode::VALUE_BYTES;
        let args = match args {
            Some(table) => json_argument(Value::Table(table), 0, &mut remaining)?,
            None => serde_json::json!({}),
        };
        if !args.is_object() {
            return Err(mlua::Error::runtime("tool arguments must be an object"));
        }
        let arguments = args.to_string();
        if arguments.len() > super::code_mode::VALUE_BYTES {
            return Err(mlua::Error::runtime("tool arguments exceed 1 MiB"));
        }
        Ok(ToolCallRequest {
            id: format!("cell_{}", uuid::Uuid::new_v4().simple()),
            name: name.to_string(),
            arguments,
            thought_signature: None,
        })
    }

    fn execute(
        &self,
        calls: Vec<ToolCallRequest>,
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> mlua::Result<Vec<String>> {
        let (reply, result) = oneshot::channel();
        let call_cancel = cancel.child_token();
        let _guard = call_cancel.clone().drop_guard();
        let request = Request {
            calls,
            cancel: call_cancel,
            reply,
        };
        tokio::runtime::Handle::current().block_on(async {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => Err("cell cancelled".to_owned()),
                _ = tokio::time::sleep_until(deadline.into()) => Err("cell deadline reached".to_owned()),
                value = async {
                    self.sender.send(request).await.map_err(|_| "tool dispatcher closed".to_owned())?;
                    result.await.map_err(|_| "tool dispatcher closed".to_owned())?
                } => value,
            }
        }).map_err(mlua::Error::runtime)
    }
}

// Bound expansion as we walk, before shared/cyclic Luau tables can allocate
// their expanded JSON representation. Only strings, scalars and plain tables cross.
fn json_argument(
    value: Value,
    depth: usize,
    budget: &mut usize,
) -> mlua::Result<serde_json::Value> {
    use serde_json::{Map, Value as Json};
    let mut charge = |bytes| -> mlua::Result<()> {
        *budget = budget
            .checked_sub(bytes)
            .ok_or_else(|| mlua::Error::runtime("tool argument tree exceeds 1 MiB"))?;
        Ok(())
    };
    charge(8)?;
    if depth > 48 {
        return Err(mlua::Error::runtime("tool arguments are nested too deeply"));
    }
    match value {
        Value::Boolean(v) => Ok(v.into()),
        Value::Integer(v) => Ok(v.into()),
        Value::Number(v) if v.is_finite() => Ok(v.into()),
        Value::String(v) => {
            charge(v.as_bytes().len())?;
            Ok(v.to_str()?.to_string().into())
        }
        Value::Nil => Ok(Json::Null),
        Value::LightUserData(v) if v.0.is_null() => Ok(Json::Null),
        Value::Table(t) if t.raw_len() > 0 => {
            let mut values = Vec::new();
            for i in 1..=t.raw_len() {
                values.push(json_argument(t.raw_get(i)?, depth + 1, budget)?);
            }
            for pair in t.pairs::<Value, Value>() {
                if !matches!(pair?.0, Value::Integer(i) if i > 0 && (i as usize) <= values.len()) {
                    return Err(mlua::Error::runtime(
                        "mixed or sparse array keys in tool arguments",
                    ));
                }
            }
            Ok(Json::Array(values))
        }
        Value::Table(t) => {
            let mut object = Map::new();
            for pair in t.pairs::<Value, Value>() {
                let (Value::String(key), value) = pair? else {
                    return Err(mlua::Error::runtime(
                        "tool argument objects require string keys",
                    ));
                };
                *budget = budget
                    .checked_sub(key.as_bytes().len())
                    .ok_or_else(|| mlua::Error::runtime("tool arguments exceed 1 MiB"))?;
                object.insert(
                    key.to_str()?.to_string(),
                    json_argument(value, depth + 1, budget)?,
                );
            }
            Ok(Json::Object(object))
        }
        _ => Err(mlua::Error::runtime(
            "tool arguments must contain JSON-compatible values",
        )),
    }
}
