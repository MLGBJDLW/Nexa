use super::LiveSnapshot;
use crate::{db::Database, error::CoreError};
use rusqlite::params;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveRecord {
    pub snapshot: LiveSnapshot,
    pub summary: Option<String>,
}

impl Database {
    pub fn save_live_record(&self, owner: &str, record: &LiveRecord) -> Result<(), CoreError> {
        if !record.snapshot.phase.is_terminal() {
            return Err(CoreError::InvalidInput("Stop Live before saving".into()));
        }
        let json = serde_json::to_string(record).map_err(|e| CoreError::Internal(e.to_string()))?;
        if json.len() > 6_000_000 {
            return Err(CoreError::InvalidInput("Live record is too large".into()));
        }
        let conn = self.conn();
        conn.execute("INSERT INTO live_records (id, owner, started_at, record_json) VALUES (?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET record_json=json_set(excluded.record_json,'$.summary',COALESCE(json_extract(excluded.record_json,'$.summary'),json_extract(live_records.record_json,'$.summary'))) WHERE owner=excluded.owner OR excluded.owner='desktop'",params![record.snapshot.id,owner,record.snapshot.started_at,json])?;
        Ok(())
    }
    pub fn load_live_record(&self, owner: &str, id: &str) -> Result<LiveRecord, CoreError> {
        let json: String = self.conn().query_row(
            "SELECT record_json FROM live_records WHERE id=?1 AND (owner=?2 OR ?2='desktop')",
            params![id, owner],
            |row| row.get(0),
        )?;
        serde_json::from_str(&json).map_err(|e| CoreError::Internal(e.to_string()))
    }
    pub fn list_live_records(&self, owner: &str) -> Result<Vec<serde_json::Value>, CoreError> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id,started_at,json_extract(record_json,'$.snapshot.model') FROM live_records WHERE owner=?1 OR ?1='desktop' ORDER BY started_at DESC LIMIT 50")?;
        let records=stmt.query_map([owner], |row| Ok(serde_json::json!({"id":row.get::<_,String>(0)?,"startedAt":row.get::<_,String>(1)?,"model":row.get::<_,String>(2)?})))?.collect::<Result<Vec<_>,_>>()?;
        Ok(records)
    }
}
