//! Bounded execution lanes for synchronous SQLite work.
//!
//! `rusqlite` is intentionally synchronous. Async hosts must enqueue work on
//! these dedicated threads instead of blocking Tokio workers on the global
//! connection mutex.

use std::time::{Duration, Instant};

use tokio::sync::{
    mpsc::{self, error::TrySendError, Sender},
    oneshot,
};

use crate::db::Database;
use crate::error::CoreError;

type DatabaseJob = Box<dyn FnOnce(&Database) + Send + 'static>;

enum Admission {
    Immediate,
    Wait,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseExecutionMetrics {
    pub queue_wait: Duration,
    pub execution: Duration,
}

#[derive(Debug)]
pub struct DatabaseExecution<T> {
    pub value: T,
    pub metrics: DatabaseExecutionMetrics,
}

#[derive(Clone)]
pub struct DatabaseExecutor {
    reader: Sender<DatabaseJob>,
    writer: Sender<DatabaseJob>,
    control_reader: Sender<DatabaseJob>,
    control_writer: Sender<DatabaseJob>,
}

impl DatabaseExecutor {
    pub fn new(database: Database, capacity: usize) -> Result<Self, CoreError> {
        if capacity == 0 {
            return Err(CoreError::InvalidInput(
                "Database executor capacity must be greater than zero".to_string(),
            ));
        }

        let reader_database = database.read_only_lane()?;
        let control_reader = spawn_lane("nexa-db-control-reader", database.read_only_lane()?, 16)?;
        // The same Database connection still serializes all writes. Separate
        // admission prevents UI settings/history traffic from rejecting run
        // ledger commits or checkpoints before they reach that connection.
        let control_writer = spawn_lane("nexa-db-control-writer", database.clone(), 16)?;
        let reader = spawn_lane("nexa-db-reader", reader_database, capacity)?;
        let writer = spawn_lane("nexa-db-writer", database, capacity)?;
        Ok(Self {
            reader,
            writer,
            control_reader,
            control_writer,
        })
    }

    pub async fn read<T, F>(&self, operation: F) -> Result<DatabaseExecution<T>, CoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Database) -> Result<T, CoreError> + Send + 'static,
    {
        execute(&self.reader, "reader", Admission::Immediate, operation).await
    }

    pub async fn write<T, F>(&self, operation: F) -> Result<DatabaseExecution<T>, CoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Database) -> Result<T, CoreError> + Send + 'static,
    {
        execute(&self.writer, "writer", Admission::Immediate, operation).await
    }

    pub async fn read_control<T, F>(&self, operation: F) -> Result<DatabaseExecution<T>, CoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Database) -> Result<T, CoreError> + Send + 'static,
    {
        execute(
            &self.control_reader,
            "control reader",
            Admission::Wait,
            operation,
        )
        .await
    }

    pub async fn write_control<T, F>(&self, operation: F) -> Result<DatabaseExecution<T>, CoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Database) -> Result<T, CoreError> + Send + 'static,
    {
        execute(
            &self.control_writer,
            "control writer",
            Admission::Wait,
            operation,
        )
        .await
    }
}

fn spawn_lane(
    name: &str,
    database: Database,
    capacity: usize,
) -> Result<Sender<DatabaseJob>, CoreError> {
    let (sender, mut receiver) = mpsc::channel::<DatabaseJob>(capacity);
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            while let Some(job) = receiver.blocking_recv() {
                job(&database);
            }
        })
        .map_err(|error| CoreError::Internal(format!("Failed to start {name}: {error}")))?;
    Ok(sender)
}

async fn execute<T, F>(
    lane: &Sender<DatabaseJob>,
    lane_name: &str,
    admission: Admission,
    operation: F,
) -> Result<DatabaseExecution<T>, CoreError>
where
    T: Send + 'static,
    F: FnOnce(&Database) -> Result<T, CoreError> + Send + 'static,
{
    let queued_at = Instant::now();
    let (result_tx, result_rx) = oneshot::channel();
    let job: DatabaseJob = Box::new(move |database: &Database| {
        let started_at = Instant::now();
        let queue_wait = started_at.saturating_duration_since(queued_at);
        let value = operation(database);
        let execution = started_at.elapsed();
        let _ = result_tx.send((value, queue_wait, execution));
    });

    // A burst of durable event/checkpoint commits must wait for its reserved
    // lane instead of turning temporary contention into a failed agent run.
    // The queue stays bounded and waiting yields to async cancellation. Once
    // admitted, writes still finish even if their caller stops awaiting them.
    let admitted = match admission {
        Admission::Immediate => lane.try_send(job),
        Admission::Wait => lane
            .send(job)
            .await
            .map_err(|error| TrySendError::Closed(error.0)),
    };
    match admitted {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            return Err(CoreError::InvalidInput(format!(
                "Database {lane_name} queue is full; retry after current work completes"
            )))
        }
        Err(TrySendError::Closed(_)) => {
            return Err(CoreError::Internal(format!(
                "Database {lane_name} executor is unavailable"
            )))
        }
    }

    let (value, queue_wait, execution) = result_rx.await.map_err(|_| {
        CoreError::Internal(format!(
            "Database {lane_name} executor stopped before returning a result"
        ))
    })?;
    Ok(DatabaseExecution {
        value: value?,
        metrics: DatabaseExecutionMetrics {
            queue_wait,
            execution,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fill_lane(lane: &Sender<DatabaseJob>) -> std::sync::mpsc::Sender<()> {
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        lane.try_send(Box::new(move |_| {
            let _ = entered_tx.send(());
            let _ = release_rx.recv_timeout(Duration::from_secs(5));
        }))
        .unwrap();
        entered_rx.await.unwrap();
        while lane.capacity() > 0 {
            lane.try_send(Box::new(|_| {})).unwrap();
        }
        release_tx
    }

    #[tokio::test(flavor = "current_thread")]
    async fn full_control_lanes_wait_without_rejecting_or_blocking_async_work() {
        let executor = DatabaseExecutor::new(Database::open_memory().unwrap(), 1).unwrap();
        let read_release = fill_lane(&executor.control_reader).await;
        let write_release = fill_lane(&executor.control_writer).await;
        let mut read = Box::pin(executor.read_control(|_| Ok("read")));
        let mut write = Box::pin(executor.write_control(|_| Ok("checkpoint")));

        assert!(futures::poll!(&mut read).is_pending());
        assert!(futures::poll!(&mut write).is_pending());
        // Neither saturated control lane can occupy a Tokio worker or consume
        // the application lanes' capacity.
        executor.read(|_| Ok(())).await.unwrap();
        executor.write(|_| Ok(())).await.unwrap();
        read_release.send(()).unwrap();
        write_release.send(()).unwrap();
        assert_eq!(read.await.unwrap().value, "read");
        assert_eq!(write.await.unwrap().value, "checkpoint");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelling_control_admission_does_not_enqueue_a_write() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };

        let executor = DatabaseExecutor::new(Database::open_memory().unwrap(), 1).unwrap();
        let release = fill_lane(&executor.control_writer).await;
        let executed = Arc::new(AtomicBool::new(false));
        let observed = executed.clone();
        let mut pending = Box::pin(executor.write_control(move |_| {
            observed.store(true, Ordering::SeqCst);
            Ok(())
        }));
        assert!(futures::poll!(&mut pending).is_pending());
        drop(pending);
        release.send(()).unwrap();
        executor.write_control(|_| Ok(())).await.unwrap();
        assert!(!executed.load(Ordering::SeqCst));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn admitted_control_write_finishes_after_caller_cancellation() {
        let executor = DatabaseExecutor::new(Database::open_memory().unwrap(), 1).unwrap();
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (committed_tx, committed_rx) = oneshot::channel();
        let mut pending = Box::pin(executor.write_control(move |_| {
            let _ = entered_tx.send(());
            let _ = release_rx.recv_timeout(Duration::from_secs(5));
            let _ = committed_tx.send(());
            Ok(())
        }));
        assert!(futures::poll!(&mut pending).is_pending());
        entered_rx.await.unwrap();
        drop(pending);
        release_tx.send(()).unwrap();
        committed_rx.await.unwrap();
    }

    #[tokio::test]
    async fn full_application_lanes_still_reject_excess_work() {
        let executor = DatabaseExecutor::new(Database::open_memory().unwrap(), 1).unwrap();
        let read_release = fill_lane(&executor.reader).await;
        let write_release = fill_lane(&executor.writer).await;
        assert!(matches!(
            executor.read(|_| Ok(())).await,
            Err(CoreError::InvalidInput(_))
        ));
        assert!(matches!(
            executor.write(|_| Ok(())).await,
            Err(CoreError::InvalidInput(_))
        ));
        read_release.send(()).unwrap();
        write_release.send(()).unwrap();
    }

    #[tokio::test]
    async fn bounded_control_admission_survives_full_application_lanes() {
        let directory = tempfile::tempdir().unwrap();
        let executor = DatabaseExecutor::new(
            Database::new(directory.path().join("control.db")).unwrap(),
            1,
        )
        .unwrap();
        let mut releases = Vec::new();
        for lane in [&executor.reader, &executor.writer] {
            let (entered_tx, entered_rx) = oneshot::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            lane.try_send(Box::new(move |_| {
                let _ = entered_tx.send(());
                let _ = release_rx.recv_timeout(Duration::from_secs(3));
            }))
            .unwrap();
            entered_rx.await.unwrap();
            lane.try_send(Box::new(|_| {})).unwrap();
            releases.push(release_tx);
        }
        let read = tokio::time::timeout(
            Duration::from_millis(500),
            executor.read_control(|db| db.stoppable_interaction_run_for_conversation("empty")),
        )
        .await;
        let write = tokio::time::timeout(
            Duration::from_millis(500),
            executor.write_control(|db| db.load_app_config()),
        )
        .await;
        for release in releases {
            let _ = release.send(());
        }
        assert!(
            read.expect("stop lookup must not wait behind application requests")
                .is_ok(),
            "stop lookup was rejected by a full application queue"
        );
        assert!(
            write
                .expect("checkpoint admission must have reserved capacity")
                .is_ok(),
            "checkpoint was rejected by a full application queue"
        );
    }

    #[tokio::test]
    async fn executes_reads_and_writes_on_dedicated_threads() {
        let database = Database::open_memory().unwrap();
        let executor = DatabaseExecutor::new(database, 4).unwrap();

        let writer = executor
            .write(|database| {
                let thread = std::thread::current()
                    .name()
                    .unwrap_or_default()
                    .to_string();
                database.load_app_config()?;
                Ok(thread)
            })
            .await
            .unwrap();
        let reader = executor
            .read(|database| {
                let thread = std::thread::current()
                    .name()
                    .unwrap_or_default()
                    .to_string();
                database.get_conversation_stats()?;
                Ok(thread)
            })
            .await
            .unwrap();

        assert_eq!(writer.value, "nexa-db-writer");
        assert_eq!(reader.value, "nexa-db-reader");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_writer_contention_does_not_stall_reads_or_async_cancellation() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::new(directory.path().join("contention.db")).unwrap();
        let executor = DatabaseExecutor::new(database.clone(), 4).unwrap();
        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let tool = std::thread::spawn(move || {
            let _connection = database.conn();
            let _ = locked_tx.send(());
            let _ = release_rx.recv_timeout(Duration::from_secs(3));
        });
        locked_rx.await.unwrap();
        let writer = executor.clone();
        let pending = tokio::spawn(async move { writer.write(Database::load_app_config).await });
        let read = tokio::time::timeout(
            Duration::from_millis(500),
            executor.read(Database::list_conversations),
        )
        .await;
        let cancelled = tokio::time::timeout(Duration::from_millis(500), async {
            tokio::task::yield_now().await;
            pending.abort();
            pending.await.unwrap_err().is_cancelled()
        })
        .await;
        let _ = release_tx.send(());
        tool.join().unwrap();
        assert!(read
            .expect("tool-side writer lock stalled a UI read")
            .is_ok());
        assert!(cancelled.expect("database work blocked the async cancellation executor"));
    }
}
