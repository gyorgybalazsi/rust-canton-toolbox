//! Consumes a stream of [`WriteBatch`]es (one per ledger update), groups them
//! into larger batches and writes them to SurrealDB.

use std::time::{Duration, Instant};

use anyhow::Result;
use futures_util::Stream;
use tokio_stream::StreamExt;
use tracing::{debug, info, warn};

use crate::model::WriteBatch;
use crate::store::{AcsMarker, Store};

/// What the sync loop should do about the ACS load, derived from the marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcsLoadState {
    /// No marker: load the ACS fresh at the current begin_offset.
    Fresh,
    /// Marker present and incomplete: resume at its offset with its token.
    Resume {
        acs_offset: i64,
        continuation: Option<Vec<u8>>,
    },
    /// Marker present and complete: skip the ACS load entirely.
    Skip,
}

/// Pure decision: marker -> action. A complete marker always means Skip,
/// regardless of the current begin_offset (re-snapshotting only happens via
/// --fresh, which wipes all data incl. the marker).
pub fn decide_acs_load(marker: Option<AcsMarker>) -> AcsLoadState {
    match marker {
        None => AcsLoadState::Fresh,
        Some(m) if m.acs_complete => AcsLoadState::Skip,
        Some(m) => AcsLoadState::Resume {
            acs_offset: m.acs_offset,
            continuation: m.acs_continuation_token,
        },
    }
}

/// Update-stream resume floor: never start before the ACS snapshot offset,
/// even when no transactions exist yet.
pub fn acs_floor(db_max: Option<i64>, marker_acs_offset: Option<i64>) -> Option<i64> {
    match (db_max, marker_acs_offset) {
        (Some(m), Some(a)) => Some(m.max(a)),
        (Some(m), None) => Some(m),
        (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

/// Why [`apply_batch_stream`] stopped consuming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamExit {
    /// The source stream closed or errored out (server abort, natural end).
    Ended,
    /// The proactive rotation deadline was reached at a batch boundary.
    Rotated,
}

/// The next poll timeout: the sooner of the flush deadline and (if a rotation
/// deadline is set) the time until rotation.
fn effective_timeout(flush_remaining: Duration, rotation_remaining: Option<Duration>) -> Duration {
    match rotation_remaining {
        Some(r) => flush_remaining.min(r),
        None => flush_remaining,
    }
}

/// True if an ACS-stream error indicates the participant was pruned past the
/// snapshot offset (continuation token no longer valid).
pub fn is_pruning_error(err: &anyhow::Error) -> bool {
    let msg = format!("{}", err);
    msg.contains("FailedPrecondition")
        || msg.contains("FAILED_PRECONDITION")
        || msg.contains("PARTICIPANT_PRUNED_DATA_ACCESSED")
}

/// Settings for [`apply_batch_stream`].
#[derive(Debug, Clone, Copy)]
pub struct StreamSettings {
    /// Ledger updates per database transaction.
    pub batch_size: usize,
    /// Write a partial batch after this long.
    pub flush_timeout: Duration,
    /// Give up on a stream that has been silent this long.
    pub idle_timeout: Duration,
    /// Stop at a batch boundary once this instant passes (token rotation).
    pub rotation_deadline: Option<Instant>,
}

/// Result of [`apply_batch_stream`].
#[derive(Debug, Clone, Copy)]
pub struct StreamOutcome {
    pub before_offset: Option<i64>,
    pub after_offset: Option<i64>,
    pub elapsed: Duration,
    pub exit: StreamExit,
}

async fn flush(
    store: &Store,
    pending: &mut WriteBatch,
    updates: &mut usize,
    reason: &str,
) -> Result<()> {
    if *updates == 0 && pending.is_empty() {
        return Ok(());
    }
    let batch = std::mem::take(pending);
    let rows = batch.row_count();
    let start = Instant::now();
    store.write_batch(&batch).await?;
    info!(
        "Committed batch ({reason}): {} updates, {rows} rows in {:?}",
        *updates,
        start.elapsed()
    );
    *updates = 0;
    Ok(())
}

/// Writes the batches from `stream` to SurrealDB, grouping up to
/// `batch_size` updates per transaction. Returns when the stream ends, goes
/// idle, or the rotation deadline passes.
pub async fn apply_batch_stream<S>(
    store: &Store,
    mut stream: S,
    settings: StreamSettings,
) -> Result<StreamOutcome>
where
    S: Stream<Item = WriteBatch> + Unpin,
{
    let before_offset = store.last_processed_offset().await?;
    info!("Last written offset before update: {:?}", before_offset);

    let start_time = Instant::now();
    let mut pending = WriteBatch::default();
    let mut updates_in_batch = 0usize;
    let mut total_updates = 0u64;
    let mut batch_start: Option<Instant> = None;
    let mut last_update = Instant::now();
    let mut exit = StreamExit::Ended;

    loop {
        // Proactive rotation, checked every iteration so a saturated stream
        // (where next() always wins the timeout race) still rotates.
        if let Some(deadline) = settings.rotation_deadline
            && Instant::now() >= deadline
        {
            flush(store, &mut pending, &mut updates_in_batch, "rotation").await?;
            exit = StreamExit::Rotated;
            break;
        }

        let flush_remaining = batch_start
            .map(|s| settings.flush_timeout.saturating_sub(s.elapsed()))
            .unwrap_or(settings.flush_timeout);
        let rotation_remaining = settings
            .rotation_deadline
            .map(|d| d.saturating_duration_since(Instant::now()));
        let timeout = effective_timeout(flush_remaining, rotation_remaining);

        match tokio::time::timeout(timeout, stream.next()).await {
            Ok(Some(batch)) => {
                last_update = Instant::now();
                total_updates += 1;
                debug!(rows = batch.row_count(), "Received update");
                pending.extend(batch);
                updates_in_batch += 1;
                batch_start.get_or_insert_with(Instant::now);
                if updates_in_batch >= settings.batch_size {
                    flush(store, &mut pending, &mut updates_in_batch, "full").await?;
                    batch_start = None;
                }
            }
            Ok(None) => break,
            Err(_) => {
                if last_update.elapsed() >= settings.idle_timeout {
                    warn!(
                        "No updates received for {}s, stream appears stale. Triggering reconnect.",
                        last_update.elapsed().as_secs()
                    );
                    flush(store, &mut pending, &mut updates_in_batch, "idle").await?;
                    break;
                }
                flush(store, &mut pending, &mut updates_in_batch, "timeout").await?;
                batch_start = None;
            }
        }
    }

    flush(store, &mut pending, &mut updates_in_batch, "final").await?;

    let after_offset = store.last_processed_offset().await?;
    let elapsed = start_time.elapsed();
    info!(
        "SurrealDB update complete: offset {:?} -> {:?}, {} updates in {:?}",
        before_offset, after_offset, total_updates, elapsed
    );
    Ok(StreamOutcome {
        before_offset,
        after_offset,
        elapsed,
        exit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_timeout_no_rotation_uses_flush() {
        assert_eq!(
            effective_timeout(Duration::from_secs(5), None),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn effective_timeout_rotation_sooner_wins() {
        assert_eq!(
            effective_timeout(Duration::from_secs(5), Some(Duration::from_secs(2))),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn effective_timeout_flush_sooner_wins() {
        assert_eq!(
            effective_timeout(Duration::from_secs(1), Some(Duration::from_secs(30))),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn decide_acs_load_absent_is_fresh() {
        assert_eq!(decide_acs_load(None), AcsLoadState::Fresh);
    }

    #[test]
    fn decide_acs_load_complete_is_skip_regardless_of_offset() {
        let m = AcsMarker {
            acs_offset: 100,
            acs_continuation_token: None,
            acs_complete: true,
        };
        assert_eq!(decide_acs_load(Some(m)), AcsLoadState::Skip);
    }

    #[test]
    fn decide_acs_load_incomplete_resumes_at_marker_offset_with_token() {
        let m = AcsMarker {
            acs_offset: 100,
            acs_continuation_token: Some(vec![1, 2, 3]),
            acs_complete: false,
        };
        assert_eq!(
            decide_acs_load(Some(m)),
            AcsLoadState::Resume {
                acs_offset: 100,
                continuation: Some(vec![1, 2, 3])
            }
        );
    }

    #[test]
    fn acs_floor_cases() {
        assert_eq!(acs_floor(None, Some(500)), Some(500));
        assert_eq!(acs_floor(Some(600), Some(500)), Some(600));
        assert_eq!(acs_floor(Some(600), None), Some(600));
        assert_eq!(acs_floor(None, None), None);
    }

    #[test]
    fn is_pruning_error_matches_failed_precondition() {
        let e = anyhow::anyhow!(
            "Error reading from active contracts stream: status: FailedPrecondition, message: \"PARTICIPANT_PRUNED_DATA_ACCESSED\""
        );
        assert!(is_pruning_error(&e));
        let e = anyhow::anyhow!("Error reading from active contracts stream: status: Unavailable");
        assert!(!is_pruning_error(&e));
    }
}
