//! Live-runtime callback receipts with bounded Rust memory. SQLite owns a
//! private temporary database, removes it on close, and atomically records each
//! completed response. Repeated IDs never require replaying a filesystem effect.
use crate::error::CoreError;
use rusqlite::{Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::VecDeque;

pub struct RuntimeReceipts {
    connection: Connection,
}
impl RuntimeReceipts {
    pub fn new() -> Result<Self, CoreError> {
        let connection = Connection::open("")?;
        connection.execute_batch("PRAGMA cache_size=-2048; PRAGMA synchronous=OFF; CREATE TABLE receipts (id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, response TEXT NOT NULL);")?;
        Ok(Self { connection })
    }
    pub fn get<T: DeserializeOwned>(&self, id: &str) -> Result<Option<(String, T)>, CoreError> {
        let row: Option<(String, String)> = self
            .connection
            .prepare_cached("SELECT fingerprint,response FROM receipts WHERE id=?1")?
            .query_row([id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;
        row.map(|(signature, response)| Ok((signature, serde_json::from_str(&response)?)))
            .transpose()
    }
    pub fn insert(
        &self,
        id: &str,
        fingerprint: &str,
        response: &impl Serialize,
    ) -> Result<(), CoreError> {
        self.insert_serialized(id, fingerprint, &serde_json::to_string(response)?)
    }
    fn insert_serialized(
        &self,
        id: &str,
        fingerprint: &str,
        response: &str,
    ) -> Result<(), CoreError> {
        self.connection
            .prepare_cached(
                "INSERT OR REPLACE INTO receipts(id,fingerprint,response) VALUES (?1,?2,?3)",
            )?
            .execute(rusqlite::params![id, fingerprint, response])?;
        Ok(())
    }
    pub fn remove(&self, id: &str) -> Result<(), CoreError> {
        self.connection
            .prepare_cached("DELETE FROM receipts WHERE id=?1")?
            .execute([id])?;
        Ok(())
    }
}

/// A lossless FIFO for live events. Normal streaming stays in a small memory
/// window; a paused consumer spills into the same private receipt store instead
/// of blocking the producer or imposing a lifetime event limit.
pub struct RuntimeEventQueue {
    receipts: RuntimeReceipts,
    memory: VecDeque<String>,
    memory_bytes: usize,
    written: u64,
    read: u64,
}

const EVENT_MEMORY_BYTES: usize = 256 * 1024;
const EVENT_MEMORY_ITEMS: usize = 256;

impl RuntimeEventQueue {
    pub fn new() -> Result<Self, CoreError> {
        Ok(Self {
            receipts: RuntimeReceipts::new()?,
            memory: VecDeque::new(),
            memory_bytes: 0,
            written: 0,
            read: 0,
        })
    }

    pub fn push(&mut self, value: &impl Serialize) -> Result<(), CoreError> {
        let serialized = serde_json::to_string(value)?;
        // Once spill starts, append there until its tail is consumed. Putting
        // a newer event back into memory earlier would overtake disk events.
        if self.read == self.written
            && self.memory.len() < EVENT_MEMORY_ITEMS
            && serialized.capacity() <= EVENT_MEMORY_BYTES.saturating_sub(self.memory_bytes)
        {
            self.memory_bytes += serialized.capacity();
            self.memory.push_back(serialized);
        } else {
            self.receipts
                .insert_serialized(&self.written.to_string(), "", &serialized)?;
            self.written += 1;
        }
        Ok(())
    }

    pub fn pop<T: DeserializeOwned>(&mut self) -> Result<Option<T>, CoreError> {
        if let Some(serialized) = self.memory.pop_front() {
            self.memory_bytes -= serialized.capacity();
            return Ok(Some(serde_json::from_str(&serialized)?));
        }
        if self.read == self.written {
            return Ok(None);
        }
        let id = self.read.to_string();
        let result = self.receipts.get(&id).and_then(|row| {
            let (_, event) = row.ok_or_else(|| {
                CoreError::Agent("Agent runtime: Native event inbox lost its next event".into())
            })?;
            self.receipts.remove(&id)?;
            Ok(Some(event))
        });
        self.read += 1;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumed_stream_events_do_not_write_sqlite() {
        let mut queue = RuntimeEventQueue::new().unwrap();
        for index in 0..10_000_u32 {
            queue.push(&index).unwrap();
            assert_eq!(queue.pop::<u32>().unwrap(), Some(index));
        }
        assert_eq!(queue.written, 0);
        assert_eq!(queue.memory_bytes, 0);
        assert!(queue.pop::<u32>().unwrap().is_none());
    }

    #[test]
    fn slow_consumer_spills_without_reordering_when_memory_frees() {
        let mut queue = RuntimeEventQueue::new().unwrap();
        for index in 0..EVENT_MEMORY_ITEMS + 10 {
            queue.push(&index).unwrap();
        }
        assert_eq!(queue.memory.len(), EVENT_MEMORY_ITEMS);
        assert_eq!(queue.written, 10);
        for index in 0..EVENT_MEMORY_ITEMS {
            assert_eq!(queue.pop::<usize>().unwrap(), Some(index));
        }
        // Memory is empty, but newer events must still follow the spill tail.
        queue.push(&(EVENT_MEMORY_ITEMS + 10)).unwrap();
        assert!(queue.memory.is_empty());
        for index in EVENT_MEMORY_ITEMS..=EVENT_MEMORY_ITEMS + 10 {
            assert_eq!(queue.pop::<usize>().unwrap(), Some(index));
        }
        queue.push(&"caught up").unwrap();
        assert_eq!(queue.memory.len(), 1);
        assert_eq!(queue.pop::<String>().unwrap().as_deref(), Some("caught up"));
    }

    #[test]
    fn event_memory_is_byte_bounded_without_truncating_large_unicode_events() {
        let mut queue = RuntimeEventQueue::new().unwrap();
        let payload = "大\n\"".repeat(EVENT_MEMORY_BYTES / 16);
        for _ in 0..5 {
            queue.push(&payload).unwrap();
        }
        assert!(queue.memory_bytes <= EVENT_MEMORY_BYTES);
        assert!(queue.written > 0);
        let oversized = "大".repeat(EVENT_MEMORY_BYTES);
        queue.push(&oversized).unwrap();
        for _ in 0..5 {
            assert_eq!(
                queue.pop::<String>().unwrap().as_deref(),
                Some(payload.as_str())
            );
        }
        assert_eq!(
            queue.pop::<String>().unwrap().as_deref(),
            Some(oversized.as_str())
        );
        assert!(queue.pop::<String>().unwrap().is_none());
        assert_eq!(queue.memory_bytes, 0);
    }
}
