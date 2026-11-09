use anyhow::Result;
use futures_util::Stream;
use neo4rs::{Graph, query};
use std::time::{Duration, Instant};
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};

pub use crate::cypher::CypherQuery;

/// Durable ACS-load state stored on the singleton `(:SyncState)` node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcsMarker {
    pub acs_offset: i64,
    pub acs_continuation_token: Option<Vec<u8>>,
    pub acs_complete: bool,
}

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
/// --fresh, which wipes all nodes incl. the marker).
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
/// even when no Transaction nodes exist yet (G3).
pub fn acs_floor(neo4j_max: Option<i64>, marker_acs_offset: Option<i64>) -> Option<i64> {
    match (neo4j_max, marker_acs_offset) {
        (Some(m), Some(a)) => Some(m.max(a)),
        (Some(m), None) => Some(m),
        (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

/// Why `apply_cypher_vec_stream_to_neo4j` stopped consuming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamExit {
    /// The source stream closed or errored out (server abort, natural end).
    Ended,
    /// The proactive rotation deadline was reached at a batch boundary.
    Rotated,
}

/// The next poll timeout: the sooner of the flush deadline and (if a rotation
/// deadline is set) the time until rotation. With no rotation deadline, it's
/// just the flush remaining.
fn effective_timeout(
    flush_remaining: std::time::Duration,
    rotation_remaining: Option<std::time::Duration>,
) -> std::time::Duration {
    match rotation_remaining {
        Some(r) => flush_remaining.min(r),
        None => flush_remaining,
    }
}

const MAX_COMMIT_RETRIES: u32 = 3;
const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Commits a batch of queries to Neo4j with retry logic for transient errors (e.g., deadlocks).
async fn commit_with_retry(
    graph: &Graph,
    queries: Vec<neo4rs::Query>,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut delay = INITIAL_RETRY_DELAY;
    for attempt in 1..=MAX_COMMIT_RETRIES {
        let mut txn = graph.start_txn().await?;
        match txn.run_queries(queries.clone()).await {
            Ok(_) => match txn.commit().await {
                Ok(_) => return Ok(()),
                Err(e) if is_transient_error(&e) && attempt < MAX_COMMIT_RETRIES => {
                    warn!(
                        "Transient Neo4j error on commit (attempt {}/{}): {}. Retrying in {:?}",
                        attempt, MAX_COMMIT_RETRIES, e, delay
                    );
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
                Err(e) => return Err(Box::new(e)),
            },
            Err(e) if is_transient_error(&e) && attempt < MAX_COMMIT_RETRIES => {
                warn!(
                    "Transient Neo4j error on run_queries (attempt {}/{}): {}. Retrying in {:?}",
                    attempt, MAX_COMMIT_RETRIES, e, delay
                );
                // Transaction is already failed, just retry
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            Err(e) => return Err(Box::new(e)),
        }
    }
    error!(
        "Exhausted all {} retries for Neo4j commit",
        MAX_COMMIT_RETRIES
    );
    Err("Exhausted all retries for Neo4j commit".into())
}

/// Checks if a Neo4j error is transient and worth retrying.
fn is_transient_error(e: &neo4rs::Error) -> bool {
    let msg = format!("{}", e);
    msg.contains("DeadlockDetected") || msg.contains("TransientError")
}

/// Queries Neo4j for the maximum offset stored in the graph.
/// This is used to determine where to resume processing after a restart.
pub async fn get_last_processed_offset(uri: &str, user: &str, pass: &str) -> Result<Option<i64>> {
    debug!("Connecting to Neo4j at {} to query last offset", uri);
    let graph = Graph::new(uri, user, pass)?;

    // Use Transaction label to leverage the transaction_offset index (fast)
    let mut result = graph
        .execute(query(
            "MATCH (t:Transaction) RETURN max(t.offset) as max_offset",
        ))
        .await?;
    match result.next().await {
        Ok(Some(row)) => {
            let offset = row.get::<Option<i64>>("max_offset")?;
            info!("Last processed offset from Neo4j: {:?}", offset);
            Ok(offset)
        }
        Ok(None) => {
            info!("No offset found in Neo4j (empty database)");
            Ok(None)
        }
        Err(e) => {
            warn!("Failed to query last offset from Neo4j: {}", e);
            Err(e.into())
        }
    }
}

#[allow(clippy::too_many_arguments)] // Neo4j connection + stream params; a struct would not clarify.
pub async fn apply_cypher_vec_stream_to_neo4j<S>(
    uri: &str,
    user: &str,
    pass: &str,
    mut query_stream: S,
    batch_size: usize,
    flush_timeout_secs: u64,
    idle_timeout_secs: u64,
    rotation_deadline: Option<Instant>,
) -> Result<(Option<i64>, Option<i64>, u128, StreamExit), Box<dyn std::error::Error>>
where
    S: Stream<Item = Vec<CypherQuery>> + Unpin,
{
    let flush_timeout = Duration::from_secs(flush_timeout_secs);
    info!("Connecting to Neo4j at {}", uri);
    let graph = Graph::new(uri, user, pass)?;
    debug!(uri = %uri, user = %user, "Successfully connected to Neo4j");

    // Query max offset before update
    debug!("Querying max offset before update");
    let before_offset = {
        let mut result = graph
            .execute(query(
                "MATCH (t:Transaction) RETURN max(t.offset) as max_offset",
            ))
            .await?;
        match result.next().await {
            Ok(Some(row)) => row.get::<Option<i64>>("max_offset")?,
            Ok(None) => None,
            Err(e) => return Err(Box::new(e)),
        }
    };
    info!("Max offset before update: {:?}", before_offset);

    // Measure update time
    let start_time = Instant::now();
    info!(
        "Starting to process query stream (batch_size={}, flush_timeout={}s)",
        batch_size, flush_timeout_secs
    );

    // Batch multiple updates together for better Neo4j throughput
    let idle_timeout = Duration::from_secs(idle_timeout_secs);
    let mut batch_count = 0u64;
    let mut pending_queries: Vec<neo4rs::Query> = Vec::new();
    let mut updates_in_batch = 0usize;
    let mut batch_start_time: Option<Instant> = None;
    let mut last_update_time = Instant::now();

    let mut exit = StreamExit::Ended;

    loop {
        // Proactive rotation: stop at a batch boundary before the token hard-expires.
        // Checked at the top of every iteration so a saturated stream (where
        // query_stream.next() always wins the timeout race) still rotates.
        if let Some(deadline) = rotation_deadline
            && Instant::now() >= deadline
        {
            if !pending_queries.is_empty() {
                let queries_to_flush: Vec<neo4rs::Query> = std::mem::take(&mut pending_queries);
                commit_with_retry(&graph, queries_to_flush).await?;
                info!(
                    "Flushed {} pending queries before stream rotation",
                    updates_in_batch
                );
            }
            exit = StreamExit::Rotated;
            break;
        }

        // Calculate remaining time until flush timeout, bounded by the rotation deadline
        let flush_remaining = batch_start_time
            .map(|start| flush_timeout.saturating_sub(start.elapsed()))
            .unwrap_or(flush_timeout);
        let rotation_remaining =
            rotation_deadline.map(|d| d.saturating_duration_since(Instant::now()));
        let timeout_remaining = effective_timeout(flush_remaining, rotation_remaining);

        // Wait for next update with timeout
        let next_update = tokio::time::timeout(timeout_remaining, query_stream.next()).await;

        match next_update {
            Ok(Some(cypher_vec)) => {
                last_update_time = Instant::now();
                // Received an update
                batch_count += 1;
                let query_count = cypher_vec.len();
                debug!(
                    batch = batch_count,
                    query_count = query_count,
                    "Received update"
                );

                // Accumulate queries
                let query_count_this_update = cypher_vec.len();
                pending_queries.extend(cypher_vec.into_iter().map(|cq| cq.query));
                updates_in_batch += 1;

                // Start batch timer on first update
                if batch_start_time.is_none() {
                    batch_start_time = Some(Instant::now());
                }

                if updates_in_batch == 1 {
                    info!("First update received, {} queries", query_count_this_update);
                }

                // Commit when batch is full
                if updates_in_batch >= batch_size {
                    let total_queries = pending_queries.len();
                    info!(
                        "Starting batch commit (full): {} updates, {} queries",
                        updates_in_batch, total_queries
                    );
                    let commit_start = Instant::now();
                    commit_with_retry(&graph, pending_queries).await?;
                    let commit_time = commit_start.elapsed();
                    info!(
                        "Committed batch of {} updates ({} queries) in {:?} ({} total updates)",
                        updates_in_batch, total_queries, commit_time, batch_count
                    );
                    pending_queries = Vec::new();
                    updates_in_batch = 0;
                    batch_start_time = None;
                }
            }
            Ok(None) => {
                // Stream ended
                break;
            }
            Err(_) => {
                // Check for idle timeout (stale/dead stream detection)
                if last_update_time.elapsed() >= idle_timeout {
                    warn!(
                        "No updates received for {}s (idle_timeout={}s), stream appears stale. Triggering reconnect.",
                        last_update_time.elapsed().as_secs(),
                        idle_timeout_secs,
                    );
                    // Flush any pending queries before returning
                    if !pending_queries.is_empty() {
                        let queries_to_flush: Vec<neo4rs::Query> =
                            std::mem::take(&mut pending_queries);
                        commit_with_retry(&graph, queries_to_flush).await?;
                        info!(
                            "Flushed {} pending queries before idle disconnect",
                            updates_in_batch
                        );
                    }
                    break;
                }

                // Timeout - flush partial batch if any
                if !pending_queries.is_empty() {
                    let total_queries = pending_queries.len();
                    info!(
                        "Starting batch commit (timeout): {} updates, {} queries",
                        updates_in_batch, total_queries
                    );
                    let commit_start = Instant::now();
                    commit_with_retry(&graph, pending_queries).await?;
                    let commit_time = commit_start.elapsed();
                    info!(
                        "Committed batch of {} updates ({} queries) in {:?} ({} total updates)",
                        updates_in_batch, total_queries, commit_time, batch_count
                    );
                    pending_queries = Vec::new();
                    updates_in_batch = 0;
                    batch_start_time = None;
                }
            }
        }
    }

    // Commit any remaining queries
    if !pending_queries.is_empty() {
        debug!(
            updates = updates_in_batch,
            queries = pending_queries.len(),
            "Committing final batch"
        );
        commit_with_retry(&graph, pending_queries).await?;
        info!("Committed final batch of {} updates", updates_in_batch);
    }

    let update_time_ms = start_time.elapsed().as_millis();
    info!("Processed {} batches in {} ms", batch_count, update_time_ms);

    // Query max offset after update
    debug!("Querying max offset after update");
    let after_offset = {
        let mut result = graph
            .execute(query(
                "MATCH (t:Transaction) RETURN max(t.offset) as max_offset",
            ))
            .await?;
        match result.next().await {
            Ok(Some(row)) => row.get::<Option<i64>>("max_offset")?,
            Ok(None) => None,
            Err(e) => return Err(Box::new(e)),
        }
    };
    info!("Max offset after update: {:?}", after_offset);

    info!(
        "Neo4j update complete: offset {:?} -> {:?}, {} batches in {} ms",
        before_offset, after_offset, batch_count, update_time_ms
    );

    Ok((before_offset, after_offset, update_time_ms, exit))
}

/// True if an ACS-stream error indicates the participant was pruned past the
/// snapshot offset (continuation token no longer valid). Matches on the tonic
/// Status text surfaced by stream_active_contracts (which wraps it in anyhow).
pub fn is_pruning_error(err: &anyhow::Error) -> bool {
    let msg = format!("{}", err);
    msg.contains("FailedPrecondition")
        || msg.contains("FAILED_PRECONDITION")
        || msg.contains("PARTICIPANT_PRUNED_DATA_ACCESSED")
}

/// Clear the ACS marker and all from_acs Created nodes so a fresh ACS load can
/// start cleanly (used when a resume is rejected due to pruning).
pub async fn reset_acs_state(
    uri: &str,
    user: &str,
    pass: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let graph = Graph::new(uri, user, pass)?;
    graph
        .run(query("MATCH (s:SyncState {id: 'singleton'}) DELETE s"))
        .await?;
    graph
        .run(query("MATCH (c:Created {from_acs: true}) DETACH DELETE c"))
        .await?;
    Ok(())
}

/// Read the singleton ACS marker, if present.
pub async fn read_acs_marker(
    uri: &str,
    user: &str,
    pass: &str,
) -> Result<Option<AcsMarker>, Box<dyn std::error::Error>> {
    let graph = Graph::new(uri, user, pass)?;
    let mut result = graph
        .execute(query(
            "MATCH (s:SyncState {id: 'singleton'}) \
             RETURN s.acs_offset AS acs_offset, s.acs_continuation_token AS token, s.acs_complete AS complete",
        ))
        .await?;
    match result.next().await {
        Ok(Some(row)) => {
            let acs_offset: i64 = row.get("acs_offset")?;
            let acs_continuation_token: Option<Vec<u8>> = row.get("token").ok();
            let acs_complete: bool = row.get("complete").unwrap_or(false);
            Ok(Some(AcsMarker {
                acs_offset,
                acs_continuation_token,
                acs_complete,
            }))
        }
        Ok(None) => Ok(None),
        Err(e) => Err(Box::new(e)),
    }
}

/// Commit a batch of ACS Created queries AND update the marker's resume
/// position in ONE transaction, so the durable resume point never runs ahead
/// of committed contracts.
pub async fn commit_acs_batch_and_marker(
    graph: &Graph,
    batch: Vec<neo4rs::Query>,
    acs_offset: i64,
    continuation: Option<Vec<u8>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut txn = graph.start_txn().await?;
    if !batch.is_empty() {
        txn.run_queries(batch).await?;
    }
    let marker_q = match continuation {
        Some(tok) => query(
            "MERGE (s:SyncState {id: 'singleton'}) \
             SET s.acs_offset = $offset, s.acs_continuation_token = $token, s.acs_complete = false",
        )
        .param("offset", acs_offset)
        .param("token", tok),
        None => query(
            "MERGE (s:SyncState {id: 'singleton'}) \
             SET s.acs_offset = $offset, s.acs_continuation_token = null, s.acs_complete = false",
        )
        .param("offset", acs_offset),
    };
    txn.run_queries(vec![marker_q]).await?;
    txn.commit().await?;
    Ok(())
}

/// Mark the ACS load complete (final batch already committed or empty).
pub async fn mark_acs_complete(
    graph: &Graph,
    batch: Vec<neo4rs::Query>,
    acs_offset: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut txn = graph.start_txn().await?;
    if !batch.is_empty() {
        txn.run_queries(batch).await?;
    }
    txn.run_queries(vec![
        query(
            "MERGE (s:SyncState {id: 'singleton'}) \
         SET s.acs_offset = $offset, s.acs_continuation_token = null, s.acs_complete = true",
        )
        .param("offset", acs_offset),
    ])
    .await?;
    txn.commit().await?;
    Ok(())
}

#[cfg(test)]
mod rotation_tests {
    use super::*;
    use std::time::Duration;

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
    fn decide_acs_load_incomplete_without_token_resumes_from_start_at_offset() {
        let m = AcsMarker {
            acs_offset: 100,
            acs_continuation_token: None,
            acs_complete: false,
        };
        assert_eq!(
            decide_acs_load(Some(m)),
            AcsLoadState::Resume {
                acs_offset: 100,
                continuation: None
            }
        );
    }

    #[test]
    fn acs_floor_uses_marker_when_no_transactions() {
        assert_eq!(acs_floor(None, Some(500)), Some(500));
    }

    #[test]
    fn acs_floor_uses_max_when_transactions_ahead() {
        assert_eq!(acs_floor(Some(600), Some(500)), Some(600));
    }

    #[test]
    fn acs_floor_passthrough_when_no_marker() {
        assert_eq!(acs_floor(Some(600), None), Some(600));
        assert_eq!(acs_floor(None, None), None);
    }

    #[test]
    fn is_pruning_error_matches_failed_precondition() {
        let e = anyhow::anyhow!(
            "Error reading from active contracts stream: status: FailedPrecondition, message: \"PARTICIPANT_PRUNED_DATA_ACCESSED\""
        );
        assert!(is_pruning_error(&e));
    }

    #[test]
    fn is_pruning_error_ignores_unrelated() {
        let e = anyhow::anyhow!("Error reading from active contracts stream: status: Unavailable");
        assert!(!is_pruning_error(&e));
    }
}
