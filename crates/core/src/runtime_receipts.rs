//! Live-runtime callback receipts with bounded Rust memory. SQLite owns a
//! private temporary database, removes it on close, and atomically records each
//! completed response. Repeated IDs never require replaying a filesystem effect.
use crate::error::CoreError;
use rusqlite::{Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};

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
            .query_row(
                "SELECT fingerprint,response FROM receipts WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
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
        self.connection.execute(
            "INSERT OR REPLACE INTO receipts(id,fingerprint,response) VALUES (?1,?2,?3)",
            rusqlite::params![id, fingerprint, serde_json::to_string(response)?],
        )?;
        Ok(())
    }
    pub fn remove(&self, id: &str) -> Result<(), CoreError> {
        self.connection
            .execute("DELETE FROM receipts WHERE id=?1", [id])?;
        Ok(())
    }
}
