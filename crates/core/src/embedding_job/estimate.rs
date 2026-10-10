use std::time::{Duration, Instant};

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::{db::Database, error::CoreError};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingEstimate {
    pub model: String,
    pub elapsed_seconds: u64,
    pub estimated_remaining_seconds: Option<u64>,
    pub chunks_per_second: Option<f64>,
    /// `history` uses the same endpoint/model/dimension space on this machine;
    /// `measured` incorporates actual batches from this job.
    pub basis: String,
}

pub(super) struct EmbeddingEstimator {
    space_id: String,
    model: String,
    remaining_chars: usize,
    rate: Option<f64>,
    measured: bool,
    started: Instant,
    active_seconds: f64,
    chunks: usize,
}

impl EmbeddingEstimator {
    pub(super) fn new(
        db: &Database,
        space_id: &str,
        model: &str,
        remaining_chars: usize,
    ) -> Result<Self, CoreError> {
        // Recent local measurements are useful; old hardware/network conditions
        // should not masquerade as current measurements after a long absence.
        let rate = db.conn().query_row(
            "SELECT chars_per_second FROM embedding_performance WHERE space_id=?1 AND updated_at > datetime('now','-7 days')",
            [space_id], |row| row.get::<_,f64>(0),
        ).optional()?.filter(|rate| rate.is_finite() && *rate > 0.0);
        Ok(Self {
            space_id: space_id.into(),
            model: model.into(),
            remaining_chars,
            rate,
            measured: false,
            started: Instant::now(),
            active_seconds: 0.0,
            chunks: 0,
        })
    }

    pub(super) fn observe(
        &mut self,
        db: &Database,
        chars: usize,
        chunks: usize,
        duration: Duration,
    ) {
        self.remaining_chars = self.remaining_chars.saturating_sub(chars);
        self.active_seconds += duration.as_secs_f64();
        self.chunks += chunks;
        if chars == 0 || duration.is_zero() {
            return;
        }
        let sample = chars as f64 / duration.as_secs_f64();
        // First live sample replaces the historical prior; later batches smooth
        // short latency spikes without hiding changes such as single-input recovery.
        let rate = if self.measured {
            self.rate.map_or(sample, |rate| rate * 0.65 + sample * 0.35)
        } else {
            sample
        };
        self.rate = Some(rate);
        self.measured = true;
        if let Err(error) = db.conn().execute(
            "INSERT INTO embedding_performance(space_id,chars_per_second) VALUES (?1,?2)
             ON CONFLICT(space_id) DO UPDATE SET chars_per_second=excluded.chars_per_second,updated_at=datetime('now')",
            rusqlite::params![self.space_id,rate],
        ) {
            tracing::warn!("Could not save embedding timing sample: {error}");
        }
    }

    pub(super) fn snapshot(&self, completed: bool) -> EmbeddingEstimate {
        EmbeddingEstimate {
            model: self.model.clone(),
            elapsed_seconds: self.started.elapsed().as_secs(),
            estimated_remaining_seconds: if completed || self.remaining_chars == 0 {
                Some(0)
            } else {
                self.rate
                    .map(|rate| (self.remaining_chars as f64 / rate).ceil() as u64)
            },
            chunks_per_second: (self.active_seconds > 0.0)
                .then(|| self.chunks as f64 / self.active_seconds),
            basis: if self.measured {
                "measured"
            } else if self.rate.is_some() {
                "history"
            } else {
                "calibrating"
            }
            .into(),
        }
    }

    pub(super) fn refresh_remaining(&mut self, chars: usize) {
        self.remaining_chars = chars;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_use_work_and_exact_model_space_then_adapt_to_live_speed() {
        let db = Database::open_memory().unwrap();
        let mut estimate =
            EmbeddingEstimator::new(&db, "endpoint:model:1024", "Model", 10000).unwrap();
        assert_eq!(estimate.snapshot(false).estimated_remaining_seconds, None);
        estimate.observe(&db, 2000, 2, Duration::from_secs(4));
        assert_eq!(
            estimate.snapshot(false).estimated_remaining_seconds,
            Some(16)
        );
        assert_eq!(estimate.snapshot(false).basis, "measured");
        let next = EmbeddingEstimator::new(&db, "endpoint:model:1024", "Model", 5000).unwrap();
        assert_eq!(next.snapshot(false).estimated_remaining_seconds, Some(10));
        assert_eq!(next.snapshot(false).basis, "history");
        let other =
            EmbeddingEstimator::new(&db, "other-endpoint:model:1024", "Model", 5000).unwrap();
        assert_eq!(other.snapshot(false).estimated_remaining_seconds, None);
        estimate.observe(&db, 2000, 2, Duration::from_secs(20));
        assert!(
            estimate
                .snapshot(false)
                .estimated_remaining_seconds
                .unwrap()
                > 12
        );
        assert_eq!(estimate.snapshot(true).estimated_remaining_seconds, Some(0));
    }
}
