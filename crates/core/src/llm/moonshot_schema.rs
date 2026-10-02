//! Tool-schema projection for Moonshot's restricted JSON Schema dialect.
//!
//! This is a wire description, not an argument validator. The registered tool
//! keeps its original schema and execution checks. In particular, root unions
//! must become an object, while conditional/exclusive rules that MFJS cannot
//! express remain guidance for the model and are enforced by the tool owner.
//! See https://github.com/MoonshotAI/walle/blob/main/docs/mfjs-spec.md.

use serde_json::{json, Map, Value};

use super::provider_boundary::{
    is_alibaba_chat_endpoint, is_alibaba_coding_chat_endpoint, is_moonshot_public_endpoint,
};
use super::ProviderType;

pub(super) fn uses_moonshot_schema(
    provider: ProviderType,
    base_url: Option<&str>,
    model: &str,
) -> bool {
    if is_moonshot_public_endpoint(provider, base_url) {
        return true;
    }
    // Explicit OpenAI-compatible connections to the same public service have
    // the same schema dialect. Never infer it from a model name on a proxy.
    let alibaba = is_alibaba_chat_endpoint(provider, base_url)
        || (base_url.is_some()
            && is_alibaba_chat_endpoint(ProviderType::AlibabaModelStudio, base_url));
    let model = model.to_ascii_lowercase();
    if alibaba {
        return matches!(
            model.strip_prefix("kimi/").unwrap_or(&model),
            "kimi-k3"
                | "kimi-k2.7-code"
                | "kimi-k2.7-code-highspeed"
                | "kimi-k2.6"
                | "kimi-k2.5"
                | "kimi-k2-thinking"
                | "moonshot-kimi-k2-instruct"
        );
    }
    // Coding Plan is a separate contract; do not grant its other models the
    // capabilities of the pay-as-you-go or third-party direct-supply routes.
    model == "kimi-k2.5" && is_alibaba_coding_chat_endpoint(provider, base_url)
}

pub(super) fn project_tool_parameters(schema: &Value) -> Value {
    Projection {
        root: schema,
        references: Vec::new(),
        remaining: 4096,
    }
    .project(schema, true, 0)
}

struct Projection<'a> {
    root: &'a Value,
    references: Vec<String>,
    remaining: usize,
}

impl Projection<'_> {
    fn project(&mut self, value: &Value, root: bool, depth: usize) -> Value {
        // Bound both cyclic references and repeated expansion of a shared DAG.
        // An unconstrained wire node does not relax the real tool's contract.
        if depth >= 24 || self.remaining == 0 {
            return empty_schema(root);
        }
        self.remaining -= 1;
        let Some(schema) = value.as_object() else {
            return empty_schema(root);
        };
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            let target = reference
                .strip_prefix('#')
                .and_then(|pointer| self.root.pointer(pointer));
            if let Some(target) = target.filter(|_| !self.references.iter().any(|r| r == reference))
            {
                self.references.push(reference.to_owned());
                let mut projected = self.project(target, root, depth + 1);
                self.references.pop();
                if let Some(out) = projected.as_object_mut() {
                    append_description(out, schema.get("description").and_then(Value::as_str));
                }
                return projected;
            }
            // Remote references are never fetched. Recursive/unknown schemas
            // remain open on the wire, with validation owned by the MCP tool.
            return empty_schema(root);
        }

        let object = root
            || schema.get("type").and_then(Value::as_str) == Some("object")
            || (!schema.contains_key("type")
                && (schema.contains_key("properties") || schema.contains_key("required")));
        let union = schema
            .get("anyOf")
            .or_else(|| schema.get("oneOf"))
            .and_then(Value::as_array);
        if !object {
            if let Some(branches) = union.filter(|branches| !branches.is_empty()) {
                let variants = branches
                    .iter()
                    .map(|branch| {
                        let mut branch = self.project(branch, false, depth + 1);
                        if let Some(out) = branch.as_object_mut() {
                            append_description(
                                out,
                                schema.get("description").and_then(Value::as_str),
                            );
                        }
                        branch
                    })
                    .collect();
                // A union's validation siblings cannot coexist with anyOf in
                // MFJS. Keep them as guidance, rather than emitting another 400.
                let mut out = union_schema(variants);
                if let Some(out) = out.as_object_mut() {
                    let constraints: Map<_, _> = schema
                        .iter()
                        .filter(|(key, _)| {
                            !matches!(
                                key.as_str(),
                                "anyOf"
                                    | "oneOf"
                                    | "description"
                                    | "$defs"
                                    | "definitions"
                                    | "$schema"
                            )
                        })
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect();
                    if !constraints.is_empty() {
                        append_rules_to_branches(out, &Value::Object(constraints));
                    }
                    if schema.contains_key("oneOf") {
                        describe_branches(out, "Exactly one of the alternatives must match.");
                    }
                }
                return out;
            }
            if let Some(types) = schema.get("type").and_then(Value::as_array) {
                let variants = types
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|t| valid_type(t))
                    .map(|kind| {
                        let mut branch = schema.clone();
                        branch.insert("type".into(), json!(kind));
                        self.project(&Value::Object(branch), false, depth + 1)
                    })
                    .collect();
                return union_schema(variants);
            }
        }

        let mut out = Map::new();
        for key in ["description", "default"] {
            if let Some(value) = schema.get(key) {
                out.insert(key.into(), value.clone());
            }
        }
        let kind = if object {
            Some("object")
        } else {
            schema
                .get("type")
                .and_then(Value::as_str)
                .filter(|t| valid_type(t))
                .or_else(|| schema.contains_key("items").then_some("array"))
        };
        if let Some(kind) = kind {
            out.insert("type".into(), json!(kind));
        }
        if let Some(values) = schema
            .get("enum")
            .and_then(Value::as_array)
            .cloned()
            .or_else(|| schema.get("const").map(|value| vec![value.clone()]))
        {
            if !values.is_empty()
                && values
                    .iter()
                    .all(|v| primitive_type(v) == primitive_type(&values[0]))
            {
                let inferred = primitive_type(&values[0]);
                if let Some(inferred) = inferred.filter(|t| {
                    kind.is_none() || kind == Some(t) || (kind == Some("number") && *t == "integer")
                }) {
                    out.entry("type").or_insert_with(|| json!(inferred));
                    if inferred != "null" {
                        out.insert("enum".into(), json!(values));
                    }
                }
            } else if kind.is_none()
                && !values.is_empty()
                && values.iter().all(|v| primitive_type(v).is_some())
            {
                let variants = values
                    .into_iter()
                    .map(|v| self.project(&json!({"const":v}), false, depth + 1))
                    .collect();
                return union_schema(variants);
            }
        }

        match out.get("type").and_then(Value::as_str) {
            Some("object") => {
                let mut properties = Map::new();
                if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                    for (name, child) in props {
                        properties.insert(name.clone(), self.project(child, false, depth + 1));
                    }
                }
                out.insert("properties".into(), Value::Object(properties));
                if let Some(required) = schema.get("required").and_then(Value::as_array) {
                    add_required(&mut out, required);
                }
                if let Some(additional) = schema.get("additionalProperties") {
                    // Pattern properties cannot be expressed by this dialect;
                    // closing the object after dropping them would reject valid inputs.
                    if !schema.contains_key("patternProperties") {
                        out.insert(
                            "additionalProperties".into(),
                            if additional.is_boolean() {
                                additional.clone()
                            } else {
                                self.project(additional, false, depth + 1)
                            },
                        );
                    }
                }
                for key in ["anyOf", "oneOf", "allOf"] {
                    if let Some(branches) = schema.get(key).and_then(Value::as_array) {
                        let branches: Vec<_> = branches
                            .iter()
                            .map(|b| self.project(b, true, depth + 1))
                            .collect();
                        merge_object_branches(&mut out, &branches, key == "allOf");
                    }
                }
            }
            Some("array") => {
                let items = schema.get("items");
                let item_schema =
                    if schema.contains_key("prefixItems") || items.is_some_and(Value::is_array) {
                        // Positional tuple constraints are not homogeneous items.
                        // Preserve them as guidance; never apply the tail schema to every item.
                        json!({})
                    } else {
                        items
                            .map(|s| self.project(s, false, depth + 1))
                            .unwrap_or_else(|| json!({}))
                    };
                out.insert("items".into(), item_schema);
                copy_keys(schema, &mut out, &["minItems", "maxItems"]);
            }
            Some("string") => copy_keys(schema, &mut out, &["minLength", "maxLength", "pattern"]),
            Some("number" | "integer") => {
                copy_keys(schema, &mut out, &["minimum", "maximum"]);
            }
            _ => {}
        }
        let rules: Map<_, _> = schema
            .iter()
            .filter(|(key, _)| {
                matches!(
                    key.as_str(),
                    "anyOf"
                        | "oneOf"
                        | "allOf"
                        | "if"
                        | "then"
                        | "else"
                        | "not"
                        | "dependentRequired"
                        | "dependentSchemas"
                        | "patternProperties"
                        | "prefixItems"
                        | "exclusiveMinimum"
                        | "exclusiveMaximum"
                        | "minProperties"
                        | "maxProperties"
                )
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if !rules.is_empty() {
            append_rules(&mut out, &Value::Object(rules));
        }
        if schema.get("items").is_some_and(Value::is_array) {
            append_rules(&mut out, &json!({"items":schema["items"]}));
        }
        Value::Object(out)
    }
}

fn empty_schema(root: bool) -> Value {
    if root {
        json!({"type":"object","properties":{}})
    } else {
        json!({})
    }
}

fn valid_type(kind: &str) -> bool {
    matches!(
        kind,
        "object" | "array" | "string" | "number" | "integer" | "boolean" | "null"
    )
}

fn primitive_type(value: &Value) -> Option<&'static str> {
    match value {
        Value::Null => Some("null"),
        Value::Bool(_) => Some("boolean"),
        Value::String(_) => Some("string"),
        Value::Number(n) if n.is_i64() || n.is_u64() => Some("integer"),
        Value::Number(_) => Some("number"),
        _ => None,
    }
}

fn copy_keys(source: &Map<String, Value>, out: &mut Map<String, Value>, keys: &[&str]) {
    for key in keys {
        if let Some(value) = source.get(*key) {
            out.insert((*key).into(), value.clone());
        }
    }
}

fn append_description(out: &mut Map<String, Value>, text: Option<&str>) {
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        return;
    };
    if out.contains_key("anyOf") {
        describe_branches(out, text);
        return;
    }
    let current = out
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if current != text {
        out.insert(
            "description".into(),
            json!(if current.is_empty() {
                text.to_owned()
            } else {
                format!("{current}\n{text}")
            }),
        );
    }
}

fn append_rules(out: &mut Map<String, Value>, rules: &Value) {
    let rules = rules.to_string();
    let excerpt: String = rules.chars().take(2048).collect();
    let suffix = if excerpt.len() < rules.len() {
        "... (remaining rules validated by the tool)"
    } else {
        ""
    };
    append_description(
        out,
        Some(&format!(
            "Additional argument rules (validated by the tool): {excerpt}{suffix}"
        )),
    );
}

fn describe_branches(out: &mut Map<String, Value>, text: &str) {
    if let Some(branches) = out.get_mut("anyOf").and_then(Value::as_array_mut) {
        for branch in branches.iter_mut().filter_map(Value::as_object_mut) {
            append_description(branch, Some(text));
        }
    }
}

fn append_rules_to_branches(out: &mut Map<String, Value>, rules: &Value) {
    if let Some(branches) = out.get_mut("anyOf").and_then(Value::as_array_mut) {
        for branch in branches.iter_mut().filter_map(Value::as_object_mut) {
            append_rules(branch, rules);
        }
    }
}

fn union_schema(mut variants: Vec<Value>) -> Value {
    variants.dedup();
    if variants.is_empty() {
        json!({})
    } else {
        json!({"anyOf":variants})
    }
}

fn add_required(out: &mut Map<String, Value>, required: &[Value]) {
    let mut all = out
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let properties = out
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .expect("object properties");
    for field in required.iter().filter_map(Value::as_str) {
        properties.entry(field).or_insert_with(|| json!({}));
        let field = json!(field);
        if !all.contains(&field) {
            all.push(field);
        }
    }
    if !all.is_empty() {
        out.insert("required".into(), json!(all));
    }
}

fn merge_object_branches(out: &mut Map<String, Value>, branches: &[Value], all_of: bool) {
    let mut names = std::collections::BTreeSet::new();
    for branch in branches {
        if let Some(properties) = branch.get("properties").and_then(Value::as_object) {
            names.extend(properties.keys().cloned());
        }
    }
    let properties = out
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .expect("object properties");
    for name in names {
        if properties.contains_key(&name) {
            continue;
        }
        let mut variants = Vec::new();
        for branch in branches {
            let constraint = branch
                .get("properties")
                .and_then(|p| p.get(&name))
                .or_else(|| {
                    (!all_of)
                        .then(|| branch.get("additionalProperties"))
                        .flatten()
                });
            match constraint {
                Some(Value::Bool(false)) => {}
                Some(value) if value.is_object() => variants.push(value.clone()),
                _ if !all_of => variants.push(json!({})),
                _ => {}
            }
        }
        let merged = if variants
            .iter()
            .any(|v| v.as_object().is_some_and(Map::is_empty))
        {
            json!({})
        } else if all_of || variants.windows(2).all(|pair| pair[0] == pair[1]) {
            variants.into_iter().next().unwrap_or_else(|| json!({}))
        } else {
            union_schema(variants)
        };
        properties.insert(name, merged);
    }
    let mut required = Vec::new();
    for (index, branch) in branches.iter().enumerate() {
        let fields = branch
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if all_of {
            required.extend(fields);
        } else if index == 0 {
            required = fields;
        } else {
            required.retain(|field| fields.contains(field));
        }
    }
    add_required(out, &required);
}

#[cfg(test)]
mod tests {
    use super::*;

    // Contract oracle for the MFJS vocabulary emitted here. Inspect schema
    // slots only: defaults/enum values may themselves contain JSON Schema keys.
    fn assert_wire_schema(schema: &Value) {
        let object = schema.as_object().expect("schema must be an object");
        for key in object.keys() {
            assert!(
                [
                    "type",
                    "description",
                    "default",
                    "properties",
                    "required",
                    "additionalProperties",
                    "items",
                    "enum",
                    "minItems",
                    "maxItems",
                    "minimum",
                    "maximum",
                    "minLength",
                    "maxLength",
                    "pattern",
                    "anyOf"
                ]
                .contains(&key.as_str()),
                "unsupported keyword {key}"
            );
        }
        if let Some(branches) = object.get("anyOf") {
            assert_eq!(object.len(), 1, "union has conflicting siblings: {schema}");
            let branches = branches.as_array().unwrap();
            assert!(!branches.is_empty());
            for branch in branches {
                assert_wire_schema(branch);
            }
        }
        if let Some(kind) = object.get("type") {
            assert!(
                kind.as_str().is_some_and(valid_type),
                "invalid type: {schema}"
            );
            if kind == "array" {
                assert!(object["items"].is_object());
            }
        }
        if let Some(properties) = object.get("properties") {
            assert_eq!(object["type"], "object");
            for child in properties.as_object().unwrap().values() {
                assert_wire_schema(child);
            }
        }
        if let Some(required) = object.get("required") {
            assert_eq!(object["type"], "object");
            for field in required.as_array().unwrap() {
                assert!(object["properties"].get(field.as_str().unwrap()).is_some());
            }
        }
        if let Some(items) = object.get("items") {
            assert_wire_schema(items);
        }
        if let Some(additional) = object
            .get("additionalProperties")
            .filter(|v| !v.is_boolean())
        {
            assert_wire_schema(additional);
        }
    }

    #[test]
    fn built_in_tools_all_project_to_moonshot_vocabulary_without_mutating_the_source() {
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("prompts/tools");
        let mut count = 0;
        for file in std::fs::read_dir(directory).unwrap() {
            let path = file.unwrap().path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let definition: Value = serde_json::from_str(&text).unwrap();
            let Some(parameters) = definition.get("parameters") else {
                continue;
            };
            let projected = project_tool_parameters(parameters);
            assert_eq!(projected["type"], "object", "{}", path.display());
            assert_wire_schema(&projected);
            assert_eq!(definition, serde_json::from_str::<Value>(&text).unwrap());
            assert_eq!(
                projected,
                project_tool_parameters(parameters),
                "projection must be deterministic"
            );
            count += 1;
        }
        assert!(
            count >= 60,
            "expected the real built-in schema set, got {count}"
        );
    }

    #[test]
    fn edit_file_keeps_fields_required_path_closed_object_and_content_guidance() {
        let original: Value =
            serde_json::from_str(include_str!("../../prompts/tools/edit_file.json")).unwrap();
        let schema = project_tool_parameters(&original["parameters"]);
        assert_eq!(schema["properties"], original["parameters"]["properties"]);
        assert_eq!(schema["required"], json!(["path"]));
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema.get("anyOf").is_none());
        for alias in ["new_str", "new_string", "content"] {
            assert!(schema["description"].as_str().unwrap().contains(alias));
        }
        assert_wire_schema(&schema);
    }

    #[test]
    fn mcp_root_variants_expose_every_field_without_requiring_every_variant() {
        let schema = project_tool_parameters(&json!({
            "type":"object", "anyOf":[
                {"type":"object", "properties":{"filename":{"type":"string"},"content":{"type":"string"}}, "required":["filename","content"], "additionalProperties":false},
                {"type":"object", "properties":{"filename":{"type":"string"},"source_url":{"type":"string"}}, "required":["filename","source_url"], "additionalProperties":false}
            ]
        }));
        assert_eq!(schema["required"], json!(["filename"]));
        for field in ["filename", "content", "source_url"] {
            assert_eq!(schema["properties"][field]["type"], "string");
        }
        assert_wire_schema(&schema);
    }

    #[test]
    fn open_or_pattern_branches_do_not_gain_constraints_from_other_variants() {
        for other in [
            json!({}),
            json!({"patternProperties":{"^value$":{"type":"number"}}, "additionalProperties":false}),
        ] {
            let schema = project_tool_parameters(&json!({
                "type":"object", "anyOf":[
                    {"properties":{"value":{"type":"string"}}, "required":["value"], "additionalProperties":false},
                    other
                ]
            }));
            assert_eq!(schema["properties"]["value"], json!({}));
            assert!(schema.get("required").is_none());
            assert_ne!(schema["additionalProperties"], false);
            assert_wire_schema(&schema);
        }
    }

    #[test]
    fn nested_unions_nullable_types_and_tuples_have_no_conflicting_keywords() {
        let schema = project_tool_parameters(&json!({
            "type":"object", "properties":{
                "choice":{"type":"string", "anyOf":[{"enum":["A"]},{"enum":["B"]}]},
                "size":{"description":"Paper size", "anyOf":[{"type":"string"},{"type":"array", "items":[{"type":"number"},{"type":"number"}]}]},
                "optional":{"type":["object","null"], "properties":{"name":{"type":"string"}}},
                "tuple":{"type":"array", "prefixItems":[{"type":"string"},{"type":"number"}], "items":false},
                "mode":{"oneOf":[{"const":"on"},{"const":null}]},
                "mixed":{"enum":["auto",true,3,null]},
                "free":true
            }
        }));
        assert_wire_schema(&schema);
        assert_eq!(schema["properties"]["optional"]["anyOf"][1]["type"], "null");
        assert_eq!(schema["properties"]["tuple"]["items"], json!({}));
        assert_eq!(schema["properties"]["free"], json!({}));
        assert_eq!(
            schema["properties"]["mode"]["anyOf"][0]["enum"],
            json!(["on"])
        );
    }

    #[test]
    fn local_references_are_bounded_and_literal_data_is_untouched() {
        let literal = json!({"type":"object","anyOf":[{"type":"string"}],"$ref":"literal"});
        let schema = project_tool_parameters(&json!({
            "$defs":{"node":{"type":"object","properties":{"next":{"$ref":"#/$defs/node"},"label":{"type":"string"}}}},
            "type":"object", "properties":{
                "first":{"$ref":"#/$defs/node"},
                "second":{"$ref":"#/$defs/node"},
                "opaque":{"default":literal},
                "missing":{"$ref":"#/missing"},
                "remote":{"$ref":"https://example.invalid/schema"}
            }
        }));
        assert_eq!(
            schema["properties"]["first"],
            schema["properties"]["second"]
        );
        assert_eq!(
            schema["properties"]["first"]["properties"]["label"]["type"],
            "string"
        );
        assert_eq!(schema["properties"]["opaque"]["default"], literal);
        assert_wire_schema(&schema);
    }

    #[test]
    fn all_of_properties_and_shared_required_fields_remain_visible() {
        let schema = project_tool_parameters(&json!({
            "type":"object", "allOf":[
                {"properties":{"path":{"type":"string"}}, "required":["path"]},
                {"properties":{"text":{"type":"string"}}, "required":["text"]},
                {"if":{"required":["mode"]},"then":{"required":["extra"]}}
            ]
        }));
        assert_eq!(schema["required"], json!(["path", "text"]));
        assert_eq!(schema["properties"]["text"]["type"], "string");
        assert!(schema["description"].as_str().unwrap().contains("extra"));
        assert_wire_schema(&schema);
    }
}
