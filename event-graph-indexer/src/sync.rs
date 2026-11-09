use anyhow::{Context, Result};
use ledger_api::v2::Identifier;
use neo4rs::{ConfigBuilder, Graph, query};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};

use crate::config::TemplateFilterConfig;
use crate::cypher;
use crate::graph::{StreamExit, apply_cypher_vec_stream_to_neo4j, get_last_processed_offset};
use chrono::{Duration as ChronoDuration, Utc};
use client::active_contracts::stream_active_contracts;
use client::jwt::{TokenManager, TokenSource};
use client::ledger_end::{get_ledger_end, get_pruning_offset};
use client::stream_updates::stream_updates;

/// Configuration for the resilient sync process
pub struct SyncConfig {
    pub ledger_url: String,
    pub parties: Vec<String>,
    pub neo4j_uri: String,
    pub neo4j_user: String,
    pub neo4j_pass: String,
    /// Optional list of contract templates to filter on.
    /// If empty or None, all templates are included.
    pub template_filters: Option<Vec<TemplateFilterConfig>>,
    /// Starting offset when Neo4j has no data. If None, falls back to pruning offset.
    pub starting_offset: Option<i64>,
    /// Number of updates to batch before committing to Neo4j
    pub batch_size: usize,
    /// Flush timeout in seconds - commit even if batch isn't full after this duration
    pub flush_timeout_secs: u64,
    /// Idle timeout in seconds - reconnect if no updates received for this duration
    pub idle_timeout_secs: u64,
    /// Flatten create/choice arguments into dot-separated Neo4j node properties
    pub flatten_arguments: bool,
    /// Maximum recursion depth for flattening nested records
    pub flatten_max_depth: usize,
    /// Store raw JSON blob of arguments as Neo4j properties
    pub store_arguments_json: bool,
}

impl SyncConfig {
    /// Converts template_filters to a Vec<Identifier> for use with the client library.
    pub fn get_template_identifiers(&self) -> Option<Vec<Identifier>> {
        self.template_filters.as_ref().map(|filters| {
            filters
                .iter()
                .map(|f| Identifier {
                    package_id: f.package_name.clone(),
                    module_name: f.module_name.clone(),
                    entity_name: f.entity_name.clone(),
                })
                .collect()
        })
    }
}

/// Exponential backoff configuration
pub struct BackoffConfig {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub multiplier: f64,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            multiplier: 2.0,
        }
    }
}

/// Ensures required indexes exist in Neo4j for optimal query performance.
/// Creates indexes if they don't exist (idempotent).
async fn ensure_indexes(neo4j_uri: &str, neo4j_user: &str, neo4j_pass: &str) -> Result<()> {
    info!("Ensuring Neo4j indexes exist...");
    let graph = Graph::new(neo4j_uri, neo4j_user, neo4j_pass)?;

    let indexes = [
        "CREATE INDEX created_contract_id IF NOT EXISTS FOR (c:Created) ON (c.contract_id)",
        "CREATE INDEX created_offset_node IF NOT EXISTS FOR (c:Created) ON (c.offset, c.node_id)",
        "CREATE INDEX created_offset IF NOT EXISTS FOR (c:Created) ON (c.offset)",
        "CREATE INDEX created_template_name IF NOT EXISTS FOR (c:Created) ON (c.template_name)",
        "CREATE INDEX exercised_offset_node IF NOT EXISTS FOR (e:Exercised) ON (e.offset, e.node_id)",
        "CREATE INDEX exercised_choice_name IF NOT EXISTS FOR (e:Exercised) ON (e.choice_name)",
        "CREATE INDEX transaction_offset IF NOT EXISTS FOR (t:Transaction) ON (t.offset)",
        "CREATE INDEX transaction_id_idx IF NOT EXISTS FOR (t:Transaction) ON (t.transaction_id)",
        "CREATE INDEX party_id IF NOT EXISTS FOR (p:Party) ON (p.party_id)",
    ];

    for index_query in &indexes {
        match graph.run(query(index_query)).await {
            Ok(_) => debug!("Index ensured: {}", index_query),
            Err(e) => warn!(
                "Failed to create index (may already exist): {} - {}",
                index_query, e
            ),
        }
    }

    info!("Neo4j indexes ready");
    Ok(())
}

/// Loads the Active Contract Set (ACS) into Neo4j at a specific offset.
/// This ensures we have all contracts that were active at that offset before starting to stream updates.
///
/// The offset should be the pruning offset (or start offset for streaming) so that all contracts
/// that will be archived in the stream already exist as Created nodes.
///
/// Resumable: persists a continuation token + offset to the (:SyncState) marker after each batch
/// so that a crash mid-load can be resumed. Rotates the ACS stream at the token deadline.
#[allow(clippy::too_many_arguments)] // ledger + neo4j endpoints, auth, and resume params.
async fn load_acs_to_neo4j(
    ledger_url: &str,
    neo4j_uri: &str,
    neo4j_user: &str,
    neo4j_pass: &str,
    parties: &[String],
    token_manager: &TokenManager,
    acs_offset: i64,
    mut continuation: Option<Vec<u8>>,
    template_filters: Option<&[Identifier]>,
    flatten_config: cypher::FlattenConfig,
) -> Result<()> {
    info!(
        "Loading Active Contract Set (ACS) into Neo4j at offset {} (resume={})...",
        acs_offset,
        continuation.is_some()
    );
    let start_time = Instant::now();
    let mut contract_count = 0u64;
    let graph = Graph::new(neo4j_uri, neo4j_user, neo4j_pass)?;
    const BATCH_SIZE: usize = 500;

    loop {
        let (token, rotation_at) = token_manager.get_token_with_rotation_deadline().await?;
        let mut acs_stream = stream_active_contracts(
            Some(&token),
            acs_offset,
            continuation.clone(),
            parties.to_vec(),
            ledger_url.to_string(),
            template_filters,
        )
        .await?;
        let rotation_timer = tokio::time::sleep_until(rotation_at.into());
        tokio::pin!(rotation_timer);

        let mut batch: Vec<neo4rs::Query> = Vec::new();

        loop {
            tokio::select! {
                _ = &mut rotation_timer => { break; }
                item = acs_stream.next() => match item {
                    Some(Ok((contract, cont_tok))) => {
                        let queries = cypher::created_event_to_cypher(&contract.created_event, flatten_config);
                        batch.extend(queries.into_iter().map(|cq| cq.query));
                        contract_count += 1;
                        if cont_tok.is_some() { continuation = cont_tok; }
                        if batch.len() >= BATCH_SIZE {
                            let to_commit = std::mem::take(&mut batch);
                            crate::graph::commit_acs_batch_and_marker(&graph, to_commit, acs_offset, continuation.clone())
                                .await.map_err(|e| anyhow::anyhow!("ACS batch commit failed: {}", e))?;
                        }
                    }
                    Some(Err(e)) => return Err(e),
                    None => {
                        let to_commit = std::mem::take(&mut batch);
                        crate::graph::mark_acs_complete(&graph, to_commit, acs_offset)
                            .await.map_err(|e| anyhow::anyhow!("ACS completion commit failed: {}", e))?;
                        let elapsed = start_time.elapsed();
                        info!("ACS loading complete: {} contracts in {:.2}s at offset {}", contract_count, elapsed.as_secs_f64(), acs_offset);
                        return Ok(());
                    }
                }
            }
        }

        // Reaching here means the rotation timer fired (the None/Err arms return).
        let to_commit = std::mem::take(&mut batch);
        crate::graph::commit_acs_batch_and_marker(
            &graph,
            to_commit,
            acs_offset,
            continuation.clone(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("ACS rotation flush failed: {}", e))?;
        info!("Rotating ACS stream ahead of token expiry; resuming from continuation token");
        continue;
    }
}

/// Picks the stream resume offset for a `--fresh` retry.
///
/// On the first fresh iteration, the explorer commits to a starting offset
/// (used as the anchor for the ACS load). On any subsequent retry — token
/// refresh, transport error, etc. — the stream should resume from wherever
/// Neo4j actually is, not the original anchor; otherwise the explorer
/// re-streams offsets it already committed (idempotent `MERGE`s, but no
/// progress and `max(t.offset)` appears stalled).
fn resume_offset_after_fresh(initial: i64, neo4j_max: Option<i64>) -> i64 {
    match neo4j_max {
        Some(n) if n > initial => n,
        _ => initial,
    }
}

/// Narrows binary-search bounds when resolving a start offset by record time.
///
/// `probed` is the record time (whole seconds) of the first update at or after
/// `mid`, or `None` when no timed update exists at/after `mid` (i.e. near the
/// ledger end). `target` is the cutoff time in seconds. Returns the new
/// `(lo, hi)`. Caller maintains the invariant `lo <= mid < hi`.
///
/// Goal: find the smallest offset whose record time is `>= target`. If the
/// probe is strictly before the target, the answer is above `mid`; otherwise
/// (at/after target, or no time data) the answer is at or below `mid`.
fn bisect_narrow(lo: i64, hi: i64, mid: i64, probed: Option<i64>, target: i64) -> (i64, i64) {
    match probed {
        Some(t) if t < target => (mid + 1, hi),
        _ => (lo, mid),
    }
}

/// Extracts a record time (whole seconds) from an update, if it carries one.
/// Transactions carry `record_time` directly; offset checkpoints carry one per
/// synchronizer, so we take the max. Reassignments and topology events are not
/// used as time probes (return `None`).
fn record_time_seconds(update: &ledger_api::v2::get_updates_response::Update) -> Option<i64> {
    use ledger_api::v2::get_updates_response::Update;
    match update {
        Update::Transaction(tx) => tx.record_time.as_ref().map(|ts| ts.seconds),
        Update::OffsetCheckpoint(c) => c
            .synchronizer_times
            .iter()
            .filter_map(|st| st.record_time.as_ref().map(|ts| ts.seconds))
            .max(),
        _ => None,
    }
}

/// Maximum messages to read while probing for a record time before giving up
/// on a probe (treated as "no time at/after this offset").
const MAX_PROBE_MESSAGES: usize = 64;

/// Reads the record time (whole seconds) of the first update *strictly after*
/// `begin_exclusive`. Opens a short-lived update stream, reads until a message
/// carries a time, then drops the stream. Returns `None` if no timed update
/// arrives within `MAX_PROBE_MESSAGES` (e.g. the offset is at/after ledger end).
///
/// Using `begin_exclusive` here matches both the real sync stream (also
/// `begin_exclusive`) and the ACS load (`active_at_offset = resolved`), so the
/// resolved offset is consistent end-to-end.
async fn probe_record_time_after(
    ledger_url: &str,
    token: &str,
    parties: &[String],
    template_filters: Option<&[Identifier]>,
    begin_exclusive: i64,
) -> Result<Option<i64>> {
    let mut stream = stream_updates(
        Some(token),
        begin_exclusive,
        None,
        parties.to_vec(),
        ledger_url.to_string(),
        template_filters,
    )
    .await?;

    let mut seen = 0usize;
    while let Some(item) = stream.next().await {
        let resp = item?;
        if let Some(update) = resp.update.as_ref()
            && let Some(secs) = record_time_seconds(update)
        {
            return Ok(Some(secs));
        }
        seen += 1;
        if seen >= MAX_PROBE_MESSAGES {
            return Ok(None);
        }
    }
    Ok(None)
}

/// Resolves the ledger offset whose record time is the earliest one at or after
/// `now - days`. Binary searches `[pruning_offset, ledger_end]` using
/// `record_time` (monotonic with offset). Clamps to the pruning offset when the
/// requested window predates retained data (including when the whole ledger is
/// younger than `days`); converges to ledger end when the target is more recent
/// than the newest update.
///
/// Bias note: if a probe reads `MAX_PROBE_MESSAGES` updates without a usable
/// record time it returns `None`, which the search treats as "at/after target"
/// and narrows downward. This can only resolve to a slightly *earlier* offset
/// (more history), never skip data — a safe, over-inclusive bias.
async fn resolve_offset_days_ago(
    ledger_url: &str,
    token: &str,
    parties: &[String],
    template_filters: Option<&[Identifier]>,
    days: u32,
) -> Result<i64> {
    let target = (Utc::now() - ChronoDuration::days(days as i64)).timestamp();
    let pruning = get_pruning_offset(ledger_url, Some(token)).await?;
    let ledger_end = get_ledger_end(ledger_url, Some(token)).await?;
    info!(
        "Resolving --from-days-ago {} (target epoch {}s) over offset range [{}, {}]",
        days, target, pruning, ledger_end
    );

    let (mut lo, mut hi) = (pruning, ledger_end);
    if hi <= lo {
        info!("Ledger offset range is empty/tiny; starting from {}", lo);
        return Ok(lo);
    }

    // Precise clamp: if the oldest retained update is already at or after the
    // target, the requested window predates retained data — start at pruning.
    // (This also short-circuits the search.) Only warns in the genuine
    // predates-data case, not when the target merely happens to map to pruning.
    if let Some(oldest) =
        probe_record_time_after(ledger_url, token, parties, template_filters, pruning).await?
        && oldest >= target
    {
        warn!(
            "--from-days-ago {} predates retained data; clamped to pruning offset {}",
            days, pruning
        );
        return Ok(pruning);
    }

    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let probed =
            probe_record_time_after(ledger_url, token, parties, template_filters, mid).await?;
        let (nl, nh) = bisect_narrow(lo, hi, mid, probed, target);
        lo = nl;
        hi = nh;
    }

    info!("Resolved --from-days-ago {} to offset {}", days, lo);
    Ok(lo)
}

/// Clears all data from Neo4j by atomically dropping and recreating the `neo4j`
/// database via the `system` database. This is O(seconds) regardless of data
/// size and avoids the page-cache / undo-log pressure of a graph-level delete
/// (which on 44GB-scale datasets has been observed to hang for hours).
///
/// Requires Neo4j Enterprise (Neo4j Desktop is enterprise) and a connecting
/// user with the CREATE DATABASE privilege. The default `neo4j` user has it.
/// We fail fast with a clear message if the DDL is rejected — no silent
/// fallback to the slow graph-traversal delete.
async fn clear_neo4j_database(neo4j_uri: &str, neo4j_user: &str, neo4j_pass: &str) -> Result<()> {
    info!("Clearing Neo4j database via CREATE OR REPLACE DATABASE neo4j (fast, atomic)...");
    let config = ConfigBuilder::new()
        .uri(neo4j_uri)
        .user(neo4j_user)
        .password(neo4j_pass)
        .db("system")
        .build()
        .context("failed to build system-db config for CREATE OR REPLACE DATABASE")?;
    let graph = Graph::connect(config)
        .context("failed to connect to system database for CREATE OR REPLACE DATABASE")?;
    graph
        .run(query("CREATE OR REPLACE DATABASE neo4j WAIT"))
        .await
        .context(
            "CREATE OR REPLACE DATABASE neo4j failed. Requires Neo4j Enterprise and the \
             connecting user must have the CREATE DATABASE privilege. If you're on Community \
             Edition or a restricted user, drop and recreate the `neo4j` database via Neo4j \
             Desktop's UI instead.",
        )?;
    info!("Neo4j `neo4j` database recreated (empty)");
    Ok(())
}

/// Runs the sync process with automatic reconnection and token refresh.
///
/// This function will:
/// 1. Load the Active Contract Set (ACS) if not already loaded
/// 2. Query Neo4j for the last processed offset (resume point)
/// 3. Start streaming from that offset
/// 4. On stream errors, reconnect with exponential backoff
/// 5. Proactively refresh JWT tokens before they expire
///
/// If `fresh` is true, clears the database and starts from current ledger end.
/// If `from_days_ago` is `Some(n)`, it implies a fresh start but anchors at the
/// offset that was current ~`n` days ago (resolved via [`resolve_offset_days_ago`])
/// instead of ledger end.
pub async fn run_resilient_sync(
    sync_config: SyncConfig,
    token_source: TokenSource,
    backoff_config: BackoffConfig,
    fresh: bool,
    from_days_ago: Option<u32>,
) -> Result<()> {
    // If fresh start, clear the database first
    if fresh {
        clear_neo4j_database(
            &sync_config.neo4j_uri,
            &sync_config.neo4j_user,
            &sync_config.neo4j_pass,
        )
        .await?;
    }

    // Ensure indexes exist before starting sync
    ensure_indexes(
        &sync_config.neo4j_uri,
        &sync_config.neo4j_user,
        &sync_config.neo4j_pass,
    )
    .await?;

    let flatten_config = cypher::FlattenConfig {
        enabled: sync_config.flatten_arguments,
        max_depth: sync_config.flatten_max_depth,
        store_arguments_json: sync_config.store_arguments_json,
    };

    let token_manager = Arc::new(TokenManager::new(token_source));

    // Start background token refresh
    let token_manager_clone = Arc::clone(&token_manager);
    let _refresh_handle = token_manager_clone.start_background_refresh();
    info!("Started background JWT token refresh");

    // Start background offset progress logger with ETA
    let neo4j_uri_clone = sync_config.neo4j_uri.clone();
    let neo4j_user_clone = sync_config.neo4j_user.clone();
    let neo4j_pass_clone = sync_config.neo4j_pass.clone();
    let ledger_url_clone = sync_config.ledger_url.clone();
    let token_manager_for_progress = Arc::clone(&token_manager);
    let _progress_handle = tokio::spawn(async move {
        // `remaining <= CAUGHT_UP_THRESHOLD` => we've drained the backlog and are
        // tracking live mainnet. The transition is logged exactly once until
        // `remaining` grows past FELL_BEHIND_THRESHOLD (a big backlog appeared,
        // e.g. Neo4j was restarted), at which point the flag resets and the next
        // catch-up is logged again.
        const CAUGHT_UP_THRESHOLD: i64 = 1000;
        const FELL_BEHIND_THRESHOLD: i64 = CAUGHT_UP_THRESHOLD * 100;
        let mut prev_offset: Option<i64> = None;
        let mut prev_time: Option<Instant> = None;
        let mut caught_up = false;

        loop {
            tokio::time::sleep(Duration::from_secs(300)).await; // 5 minutes

            let current_offset = match get_last_processed_offset(
                &neo4j_uri_clone,
                &neo4j_user_clone,
                &neo4j_pass_clone,
            )
            .await
            {
                Ok(Some(offset)) => offset,
                Ok(None) => {
                    info!("[Progress] No offset data in Neo4j yet");
                    continue;
                }
                Err(e) => {
                    warn!("[Progress] Failed to query Neo4j offset: {}", e);
                    continue;
                }
            };

            // Get ledger end for ETA calculation
            let ledger_end = match token_manager_for_progress.get_token().await {
                Ok(token) => {
                    match client::ledger_end::get_ledger_end(&ledger_url_clone, Some(&token)).await
                    {
                        Ok(end) => Some(end),
                        Err(e) => {
                            warn!("[Progress] Failed to get ledger end: {}", e);
                            None
                        }
                    }
                }
                Err(_) => None,
            };

            // Calculate rate and ETA
            let rate_info = if let (Some(prev), Some(time)) = (prev_offset, prev_time) {
                let elapsed_secs = time.elapsed().as_secs_f64();
                let offsets_processed = current_offset - prev;
                let rate = offsets_processed as f64 / elapsed_secs;

                if rate > 0.0 {
                    if let Some(end) = ledger_end {
                        let remaining = end - current_offset;
                        let eta_secs = remaining as f64 / rate;
                        let eta_hours = eta_secs / 3600.0;
                        format!(
                            "rate: {:.1} offsets/s, remaining: {}, ETA: {:.1} hours",
                            rate, remaining, eta_hours
                        )
                    } else {
                        format!("rate: {:.1} offsets/s", rate)
                    }
                } else {
                    "rate: stalled".to_string()
                }
            } else if let Some(end) = ledger_end {
                format!("ledger end: {}, remaining: {}", end, end - current_offset)
            } else {
                "calculating...".to_string()
            };

            info!("[Progress] Neo4j offset: {}, {}", current_offset, rate_info);

            // Catch-up transition: log once when remaining first drops to within
            // CAUGHT_UP_THRESHOLD; allow re-logging if we fall significantly behind.
            if let Some(end) = ledger_end {
                let remaining = end - current_offset;
                if !caught_up && remaining <= CAUGHT_UP_THRESHOLD {
                    info!(
                        "[Progress] Caught up to ledger end at offset {} (remaining: {})",
                        current_offset, remaining
                    );
                    caught_up = true;
                } else if caught_up && remaining > FELL_BEHIND_THRESHOLD {
                    caught_up = false;
                }
            }

            prev_offset = Some(current_offset);
            prev_time = Some(Instant::now());
        }
    });
    info!("Started background progress logger (every 5 min)");

    let mut current_delay = backoff_config.initial_delay;
    let mut consecutive_failures = 0u32;
    let mut acs_loaded_checked = false;
    let mut fresh_start_offset: Option<i64> = None; // Used only on first iteration when fresh=true

    loop {
        // Get a fresh token + its rotation deadline
        let (token, rotation_at) = match token_manager.get_token_with_rotation_deadline().await {
            Ok(pair) => pair,
            Err(e) => {
                error!(
                    "Failed to get JWT token: {}. Retrying in {:?}",
                    e, current_delay
                );
                tokio::time::sleep(current_delay).await;
                current_delay = std::cmp::min(
                    Duration::from_secs_f64(
                        current_delay.as_secs_f64() * backoff_config.multiplier,
                    ),
                    backoff_config.max_delay,
                );
                continue;
            }
        };

        // First, determine the starting offset
        let begin_offset = if fresh && fresh_start_offset.is_none() {
            // Fresh start. --from-days-ago wins over starting_offset and ledger end.
            if let Some(days) = from_days_ago {
                let template_identifiers = sync_config.get_template_identifiers();
                match resolve_offset_days_ago(
                    &sync_config.ledger_url,
                    &token,
                    &sync_config.parties,
                    template_identifiers.as_deref(),
                    days,
                )
                .await
                {
                    Ok(resolved) => {
                        info!(
                            "FRESH START: --from-days-ago {} resolved to offset {}",
                            days, resolved
                        );
                        fresh_start_offset = Some(resolved);
                        resolved
                    }
                    Err(e) => {
                        error!(
                            "Failed to resolve --from-days-ago {}: {}. Retrying in {:?}",
                            days, e, current_delay
                        );
                        tokio::time::sleep(current_delay).await;
                        current_delay = std::cmp::min(
                            Duration::from_secs_f64(
                                current_delay.as_secs_f64() * backoff_config.multiplier,
                            ),
                            backoff_config.max_delay,
                        );
                        continue;
                    }
                }
            } else {
                // Existing behavior: use starting_offset if configured, otherwise ledger end
                match sync_config.starting_offset {
                    Some(configured_offset) => {
                        match get_ledger_end(&sync_config.ledger_url, Some(&token)).await {
                            Ok(ledger_end) => {
                                let resolved = if configured_offset < 0 {
                                    (ledger_end + configured_offset).max(0)
                                } else {
                                    configured_offset
                                };
                                info!(
                                    "FRESH START: Using starting_offset {} (resolved to {}, ledger_end={})",
                                    configured_offset, resolved, ledger_end
                                );
                                fresh_start_offset = Some(resolved);
                                resolved
                            }
                            Err(e) => {
                                error!(
                                    "Failed to get ledger end: {}. Retrying in {:?}",
                                    e, current_delay
                                );
                                tokio::time::sleep(current_delay).await;
                                current_delay = std::cmp::min(
                                    Duration::from_secs_f64(
                                        current_delay.as_secs_f64() * backoff_config.multiplier,
                                    ),
                                    backoff_config.max_delay,
                                );
                                continue;
                            }
                        }
                    }
                    None => match get_ledger_end(&sync_config.ledger_url, Some(&token)).await {
                        Ok(ledger_end) => {
                            info!(
                                "FRESH START: No starting_offset configured, using current ledger end: {}",
                                ledger_end
                            );
                            fresh_start_offset = Some(ledger_end);
                            ledger_end
                        }
                        Err(e) => {
                            error!(
                                "Failed to get ledger end: {}. Retrying in {:?}",
                                e, current_delay
                            );
                            tokio::time::sleep(current_delay).await;
                            current_delay = std::cmp::min(
                                Duration::from_secs_f64(
                                    current_delay.as_secs_f64() * backoff_config.multiplier,
                                ),
                                backoff_config.max_delay,
                            );
                            continue;
                        }
                    },
                }
            }
        } else if let Some(initial_offset) = fresh_start_offset {
            // Initial offset was pinned on the first fresh iteration to anchor
            // the ACS load. For the update stream on retry, resume from Neo4j's
            // committed max offset if it's further along — otherwise the
            // explorer re-streams offsets already processed and progress stalls.
            let neo4j_max = get_last_processed_offset(
                &sync_config.neo4j_uri,
                &sync_config.neo4j_user,
                &sync_config.neo4j_pass,
            )
            .await
            .ok()
            .flatten();
            let resume = resume_offset_after_fresh(initial_offset, neo4j_max);
            if resume != initial_offset {
                info!(
                    "Fresh retry: resuming from Neo4j max offset {} (anchor was {})",
                    resume, initial_offset
                );
            }
            resume
        } else {
            // Normal mode: check Neo4j for resume point
            match get_last_processed_offset(
                &sync_config.neo4j_uri,
                &sync_config.neo4j_user,
                &sync_config.neo4j_pass,
            )
            .await
            {
                Ok(Some(offset)) => {
                    info!("Resuming from Neo4j offset: {}", offset);
                    offset
                }
                Ok(None) => {
                    // No data in Neo4j, use configured starting_offset or fall back to pruning offset
                    // Negative starting_offset means relative to ledger end (e.g., -5000000 = 5M before end)
                    if let Some(configured_offset) = sync_config.starting_offset {
                        if configured_offset < 0 {
                            match get_ledger_end(&sync_config.ledger_url, Some(&token)).await {
                                Ok(ledger_end) => {
                                    let resolved = (ledger_end + configured_offset).max(0);
                                    info!(
                                        "No existing data in Neo4j, starting from ledger_end ({}) {} = {}",
                                        ledger_end, configured_offset, resolved
                                    );
                                    resolved
                                }
                                Err(e) => {
                                    error!(
                                        "Failed to get ledger end for relative offset: {}. Using offset 0",
                                        e
                                    );
                                    0
                                }
                            }
                        } else {
                            info!(
                                "No existing data in Neo4j, starting from configured starting_offset: {}",
                                configured_offset
                            );
                            configured_offset
                        }
                    } else {
                        match get_pruning_offset(&sync_config.ledger_url, Some(&token)).await {
                            Ok(pruning_offset) => {
                                info!(
                                    "No existing data in Neo4j, starting from ledger pruning offset: {}",
                                    pruning_offset
                                );
                                pruning_offset
                            }
                            Err(e) => {
                                error!(
                                    "Failed to get pruning offset from ledger: {}. Starting from 0",
                                    e
                                );
                                0
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to query Neo4j for last offset: {}. Querying ledger for pruning offset",
                        e
                    );
                    match get_pruning_offset(&sync_config.ledger_url, Some(&token)).await {
                        Ok(pruning_offset) => {
                            info!("Starting from ledger pruning offset: {}", pruning_offset);
                            pruning_offset
                        }
                        Err(e2) => {
                            error!(
                                "Failed to get pruning offset from ledger: {}. Starting from 0",
                                e2
                            );
                            0
                        }
                    }
                }
            }
        };

        // Get template identifiers for this iteration
        let template_identifiers = sync_config.get_template_identifiers();

        // Load ACS on first run if not already loaded (at the starting offset)
        if !acs_loaded_checked {
            let marker = crate::graph::read_acs_marker(
                &sync_config.neo4j_uri,
                &sync_config.neo4j_user,
                &sync_config.neo4j_pass,
            )
            .await
            .map_err(|e| anyhow::anyhow!("failed to read ACS marker: {}", e))?;
            match crate::graph::decide_acs_load(marker) {
                crate::graph::AcsLoadState::Skip => {
                    info!("ACS already complete (marker), skipping ACS load");
                    acs_loaded_checked = true;
                }
                crate::graph::AcsLoadState::Fresh => {
                    info!("ACS not loaded, loading fresh at offset {}", begin_offset);
                    match load_acs_to_neo4j(
                        &sync_config.ledger_url,
                        &sync_config.neo4j_uri,
                        &sync_config.neo4j_user,
                        &sync_config.neo4j_pass,
                        &sync_config.parties,
                        &token_manager,
                        begin_offset,
                        None,
                        template_identifiers.as_deref(),
                        flatten_config,
                    )
                    .await
                    {
                        Ok(()) => {
                            info!("ACS loaded successfully");
                            acs_loaded_checked = true;
                        }
                        Err(e) => {
                            error!("Failed to load ACS: {}. Retrying in {:?}", e, current_delay);
                            tokio::time::sleep(current_delay).await;
                            current_delay = std::cmp::min(
                                Duration::from_secs_f64(
                                    current_delay.as_secs_f64() * backoff_config.multiplier,
                                ),
                                backoff_config.max_delay,
                            );
                            continue;
                        }
                    }
                }
                crate::graph::AcsLoadState::Resume {
                    acs_offset,
                    continuation,
                } => {
                    info!("Resuming incomplete ACS load at offset {}", acs_offset);
                    match load_acs_to_neo4j(
                        &sync_config.ledger_url,
                        &sync_config.neo4j_uri,
                        &sync_config.neo4j_user,
                        &sync_config.neo4j_pass,
                        &sync_config.parties,
                        &token_manager,
                        acs_offset,
                        continuation,
                        template_identifiers.as_deref(),
                        flatten_config,
                    )
                    .await
                    {
                        Ok(()) => {
                            info!("ACS load resumed and completed");
                            acs_loaded_checked = true;
                        }
                        Err(e) => {
                            if crate::graph::is_pruning_error(&e) {
                                warn!(
                                    "ACS resume rejected (participant pruned past snapshot offset): {}. Clearing marker + from_acs nodes and restarting fresh.",
                                    e
                                );
                                if let Err(re) = crate::graph::reset_acs_state(
                                    &sync_config.neo4j_uri,
                                    &sync_config.neo4j_user,
                                    &sync_config.neo4j_pass,
                                )
                                .await
                                {
                                    error!("Failed to reset ACS state after pruning: {}", re);
                                }
                                // leave acs_loaded_checked = false so the next iteration re-evaluates
                                // (marker now absent -> Fresh load at the current begin_offset)
                                continue;
                            }
                            error!(
                                "Failed to resume ACS load: {}. Retrying in {:?}",
                                e, current_delay
                            );
                            tokio::time::sleep(current_delay).await;
                            current_delay = std::cmp::min(
                                Duration::from_secs_f64(
                                    current_delay.as_secs_f64() * backoff_config.multiplier,
                                ),
                                backoff_config.max_delay,
                            );
                            continue;
                        }
                    }
                }
            }
        }

        // G3: never start the update stream before the ACS snapshot offset,
        // even when no Transaction nodes exist yet.
        let begin_offset = {
            let marker_offset = crate::graph::read_acs_marker(
                &sync_config.neo4j_uri,
                &sync_config.neo4j_user,
                &sync_config.neo4j_pass,
            )
            .await
            .ok()
            .flatten()
            .map(|m| m.acs_offset);
            crate::graph::acs_floor(Some(begin_offset), marker_offset).unwrap_or(begin_offset)
        };

        info!(
            "Starting stream from offset {}, template_filters={:?}",
            begin_offset, sync_config.template_filters
        );

        // Start the update stream
        let update_stream = match stream_updates(
            Some(&token),
            begin_offset,
            None,
            sync_config.parties.clone(),
            sync_config.ledger_url.clone(),
            template_identifiers.as_deref(),
        )
        .await
        {
            Ok(stream) => stream,
            Err(e) => {
                consecutive_failures += 1;
                error!(
                    "Failed to connect to ledger (attempt {}): {}. Retrying in {:?}",
                    consecutive_failures, e, current_delay
                );
                tokio::time::sleep(current_delay).await;
                current_delay = std::cmp::min(
                    Duration::from_secs_f64(
                        current_delay.as_secs_f64() * backoff_config.multiplier,
                    ),
                    backoff_config.max_delay,
                );
                continue;
            }
        };

        // Reset backoff on successful connection
        current_delay = backoff_config.initial_delay;
        consecutive_failures = 0;
        info!("Successfully connected to ledger stream");

        // Process the stream - take items while they're Ok, stop on first error
        // This allows us to gracefully reconnect when token expires
        let cypher_stream = update_stream
            .take_while(|update| {
                match update {
                    Ok(_) => true,
                    Err(e) => {
                        error!(error = %e, "Error in update stream, will reconnect");
                        false // Stop the stream on error
                    }
                }
            })
            .map(|update| {
                // Safe to unwrap here because take_while filters out errors
                let response = update.unwrap();
                let offset = response.update.as_ref().map(|u| match u {
                    ledger_api::v2::get_updates_response::Update::Transaction(tx) => tx.offset,
                    ledger_api::v2::get_updates_response::Update::Reassignment(r) => r.offset,
                    ledger_api::v2::get_updates_response::Update::OffsetCheckpoint(c) => c.offset,
                    ledger_api::v2::get_updates_response::Update::TopologyTransaction(t) => {
                        t.offset
                    }
                });
                debug!(offset = ?offset, "Processing update from stream");
                cypher::get_updates_response_to_cypher(&response, flatten_config)
            });

        // Apply to Neo4j - this will return when the stream ends or errors
        match apply_cypher_vec_stream_to_neo4j(
            &sync_config.neo4j_uri,
            &sync_config.neo4j_user,
            &sync_config.neo4j_pass,
            cypher_stream,
            sync_config.batch_size,
            sync_config.flush_timeout_secs,
            sync_config.idle_timeout_secs,
            Some(rotation_at),
        )
        .await
        {
            Ok((before, after, time, exit)) => {
                info!(
                    "Stream processing completed. Offset {} -> {}, took {} ms",
                    before.unwrap_or(-1),
                    after.unwrap_or(-1),
                    time
                );
                match exit {
                    StreamExit::Rotated => {
                        info!(
                            "Rotating update stream ahead of token expiry (reached rotation deadline)"
                        );
                        current_delay = backoff_config.initial_delay; // planned, not a failure
                        consecutive_failures = 0;
                        // loop immediately: fresh (token, rotation_at), resume from Neo4j offset
                    }
                    StreamExit::Ended => {
                        info!(
                            "Stream ended, reconnecting in {:?}",
                            backoff_config.initial_delay
                        );
                        tokio::time::sleep(backoff_config.initial_delay).await;
                    }
                }
            }
            Err(e) => {
                consecutive_failures += 1;
                error!(
                    "Stream processing failed (attempt {}): {}. Reconnecting in {:?}",
                    consecutive_failures, e, current_delay
                );
                tokio::time::sleep(current_delay).await;
                current_delay = std::cmp::min(
                    Duration::from_secs_f64(
                        current_delay.as_secs_f64() * backoff_config.multiplier,
                    ),
                    backoff_config.max_delay,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_uses_initial_when_neo4j_empty() {
        assert_eq!(resume_offset_after_fresh(100, None), 100);
    }

    #[test]
    fn resume_uses_initial_when_neo4j_behind() {
        // Neo4j max is lower than the anchor — possible if data was cleared
        // but anchor was already set; trust the anchor.
        assert_eq!(resume_offset_after_fresh(100, Some(50)), 100);
    }

    #[test]
    fn resume_uses_initial_when_neo4j_equal() {
        // No new progress past the anchor; nothing to advance to.
        assert_eq!(resume_offset_after_fresh(100, Some(100)), 100);
    }

    #[test]
    fn resume_advances_to_neo4j_when_ahead() {
        // The real fix path: previous iteration made progress; subsequent
        // retry resumes from there instead of restarting at the anchor.
        assert_eq!(resume_offset_after_fresh(100, Some(250)), 250);
    }

    #[test]
    fn bisect_narrow_advances_when_probe_before_target() {
        // record time at mid is before the cutoff -> search the upper half
        assert_eq!(bisect_narrow(10, 20, 15, Some(1400), 1500), (16, 20));
    }

    #[test]
    fn bisect_narrow_retreats_when_probe_at_target() {
        // record time equals the cutoff -> this offset is a candidate, keep it in range
        assert_eq!(bisect_narrow(10, 20, 15, Some(1500), 1500), (10, 15));
    }

    #[test]
    fn bisect_narrow_retreats_when_probe_after_target() {
        assert_eq!(bisect_narrow(10, 20, 15, Some(1600), 1500), (10, 15));
    }

    #[test]
    fn bisect_narrow_retreats_on_none_probe() {
        // no timed update at/after mid (near ledger end) -> narrow downward
        assert_eq!(bisect_narrow(10, 20, 15, None, 1500), (10, 15));
    }

    // Drives a full bisect over a dense, monotonic offset->time map.
    // probe(mid) returns the record time at offset mid (mid*100) for the
    // retained range 10..=20, else None.
    fn run_bisect(lo: i64, hi: i64, target: i64) -> i64 {
        let probe = |mid: i64| -> Option<i64> {
            if (10..=20).contains(&mid) {
                Some(mid * 100)
            } else {
                None
            }
        };
        let (mut lo, mut hi) = (lo, hi);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let (nl, nh) = bisect_narrow(lo, hi, mid, probe(mid), target);
            lo = nl;
            hi = nh;
        }
        lo
    }

    #[test]
    fn bisect_converges_to_exact_match() {
        assert_eq!(run_bisect(10, 20, 1500), 15);
    }

    #[test]
    fn bisect_converges_to_first_offset_at_or_after_target() {
        // 1550 falls between offsets 15 (1500) and 16 (1600) -> first >= is 16
        assert_eq!(run_bisect(10, 20, 1550), 16);
    }

    #[test]
    fn bisect_clamps_to_low_when_target_predates_range() {
        assert_eq!(run_bisect(10, 20, 500), 10);
    }

    #[test]
    fn bisect_clamps_to_high_when_target_after_range() {
        assert_eq!(run_bisect(10, 20, 5000), 20);
    }

    #[test]
    fn record_time_from_transaction() {
        use ledger_api::v2::get_updates_response::Update;
        let update = Update::Transaction(ledger_api::v2::Transaction {
            record_time: Some(::prost_types::Timestamp {
                seconds: 123,
                nanos: 0,
            }),
            ..Default::default()
        });
        assert_eq!(record_time_seconds(&update), Some(123));
    }

    #[test]
    fn record_time_from_offset_checkpoint_takes_max() {
        use ledger_api::v2::get_updates_response::Update;
        let update = Update::OffsetCheckpoint(ledger_api::v2::OffsetCheckpoint {
            offset: 42,
            synchronizer_times: vec![
                ledger_api::v2::SynchronizerTime {
                    synchronizer_id: "a".to_string(),
                    record_time: Some(::prost_types::Timestamp {
                        seconds: 100,
                        nanos: 0,
                    }),
                },
                ledger_api::v2::SynchronizerTime {
                    synchronizer_id: "b".to_string(),
                    record_time: Some(::prost_types::Timestamp {
                        seconds: 200,
                        nanos: 0,
                    }),
                },
            ],
        });
        assert_eq!(record_time_seconds(&update), Some(200));
    }

    #[test]
    fn record_time_none_for_empty_checkpoint() {
        use ledger_api::v2::get_updates_response::Update;
        let update = Update::OffsetCheckpoint(ledger_api::v2::OffsetCheckpoint {
            offset: 42,
            synchronizer_times: vec![],
        });
        assert_eq!(record_time_seconds(&update), None);
    }

    #[test]
    fn record_time_none_for_transaction_without_time() {
        use ledger_api::v2::get_updates_response::Update;
        let update = Update::Transaction(ledger_api::v2::Transaction {
            record_time: None,
            ..Default::default()
        });
        assert_eq!(record_time_seconds(&update), None);
    }
}
