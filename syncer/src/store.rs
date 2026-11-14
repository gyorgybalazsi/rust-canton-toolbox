//! SurrealDB access: connecting, defining the schema, and writing batches.
//!
//! The syncer is one client among several (graph viewers, SurrealDB Studio)
//! of the same server, so it only ever connects over the network as a
//! database-level user, and only touches the configured namespace/database.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use surrealdb::Surreal;
use surrealdb::engine::any::{self, Any};
use surrealdb::opt::auth::Database;
use surrealdb::types::{Array, Bytes, Number, Object, Value};
use tracing::{debug, info, warn};

pub use crate::config::DbConfig;
use crate::model::{
    ACTION, CONSEQUENCE, CONSUMES, CREATED, EXERCISED, Edge, PARTY, REQUESTED, Row, TARGET,
    TRANSACTION, WriteBatch,
};

/// Opens a client connection and signs in as a database user.
pub async fn connect(c: &DbConfig) -> surrealdb::Result<Surreal<Any>> {
    let db = any::connect(c.url.as_str()).await?;
    db.signin(Database {
        namespace: c.namespace.clone(),
        database: c.database.clone(),
        username: c.username.clone(),
        password: c.password.clone(),
    })
    .await?;
    db.use_ns(&c.namespace).use_db(&c.database).await?;
    Ok(db)
}

/// Tables, relation tables and indexes. Run once at startup (and again after
/// a reconnect, in case the server lost them); every statement is idempotent.
pub const SCHEMA: &str = "
DEFINE TABLE IF NOT EXISTS transaction TYPE NORMAL SCHEMALESS;
DEFINE TABLE IF NOT EXISTS created TYPE NORMAL SCHEMALESS;
DEFINE TABLE IF NOT EXISTS exercised TYPE NORMAL SCHEMALESS;
DEFINE TABLE IF NOT EXISTS party TYPE NORMAL SCHEMALESS;
DEFINE TABLE IF NOT EXISTS sync_state TYPE NORMAL SCHEMALESS;
DEFINE TABLE IF NOT EXISTS action TYPE RELATION IN transaction OUT created | exercised SCHEMALESS;
DEFINE TABLE IF NOT EXISTS consequence TYPE RELATION IN exercised OUT created | exercised SCHEMALESS;
DEFINE TABLE IF NOT EXISTS target TYPE RELATION IN exercised OUT created SCHEMALESS;
DEFINE TABLE IF NOT EXISTS consumes TYPE RELATION IN exercised OUT created SCHEMALESS;
DEFINE TABLE IF NOT EXISTS requested TYPE RELATION IN party OUT transaction SCHEMALESS;
DEFINE INDEX IF NOT EXISTS action_in_out ON action FIELDS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS consequence_in_out ON consequence FIELDS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS target_in_out ON target FIELDS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS consumes_in_out ON consumes FIELDS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS requested_in_out ON requested FIELDS in, out UNIQUE;
DEFINE INDEX IF NOT EXISTS created_offset_node ON created FIELDS offset, node_id;
DEFINE INDEX IF NOT EXISTS created_template_name ON created FIELDS template_name;
DEFINE INDEX IF NOT EXISTS created_from_acs ON created FIELDS from_acs;
DEFINE INDEX IF NOT EXISTS exercised_choice_name ON exercised FIELDS choice_name;
DEFINE INDEX IF NOT EXISTS exercised_target_contract ON exercised FIELDS target_contract_id;
DEFINE INDEX IF NOT EXISTS transaction_update_id ON transaction FIELDS update_id;
";

/// Retry delays for connection and write errors; the last one repeats.
const RETRY_DELAYS: [Duration; 6] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(20),
    Duration::from_secs(30),
];

/// The delay before retry number `attempt` (0-based): 1s, 2s, 5s, 10s, 20s, then 30s.
pub fn retry_delay(attempt: usize) -> Duration {
    RETRY_DELAYS[attempt.min(RETRY_DELAYS.len() - 1)]
}

/// How long one write attempt may take before it counts as failed. While the
/// server is down the client waits for it to come back; this turns that wait
/// into a logged, retried failure instead of a silent hang.
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);

/// Durable ACS-load state stored on `sync_state:singleton`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcsMarker {
    pub acs_offset: i64,
    pub acs_continuation_token: Option<Vec<u8>>,
    pub acs_complete: bool,
}

/// A shared SurrealDB connection. Cheap to clone.
#[derive(Clone)]
pub struct Store {
    db: Surreal<Any>,
    cfg: DbConfig,
    /// How many times a failed write is retried; `None` retries until it succeeds.
    max_write_attempts: Option<usize>,
    attempt_timeout: Duration,
}

impl Store {
    /// Connects once and defines the schema. Fails on the first error.
    pub async fn connect(cfg: &DbConfig) -> Result<Self> {
        let db = connect(cfg)
            .await
            .with_context(|| format!("failed to connect to SurrealDB at {}", cfg.url))?;
        let store = Store {
            db,
            cfg: cfg.clone(),
            max_write_attempts: None,
            attempt_timeout: ATTEMPT_TIMEOUT,
        };
        store.ensure_schema().await?;
        Ok(store)
    }

    /// Connects, retrying with a growing delay until the server is reachable.
    pub async fn connect_with_retry(cfg: &DbConfig) -> Self {
        let mut attempt = 0;
        loop {
            match Store::connect(cfg).await {
                Ok(store) => {
                    info!(url = %cfg.url, ns = %cfg.namespace, db = %cfg.database, "Connected to SurrealDB");
                    return store;
                }
                Err(e) => {
                    let delay = retry_delay(attempt);
                    warn!("SurrealDB not reachable ({e:#}); retrying in {delay:?}");
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }

    /// Limits how often a failed write is retried (used by tests). By default
    /// writes are retried until they succeed, so no data is dropped.
    pub fn with_max_write_attempts(mut self, attempts: usize) -> Self {
        self.max_write_attempts = Some(attempts.max(1));
        self
    }

    /// Changes how long one write attempt may take (default 30 s).
    pub fn with_attempt_timeout(mut self, timeout: Duration) -> Self {
        self.attempt_timeout = timeout;
        self
    }

    /// The underlying client, for ad-hoc queries.
    pub fn client(&self) -> &Surreal<Any> {
        &self.db
    }

    pub async fn ensure_schema(&self) -> Result<()> {
        let mut response = self.db.query(SCHEMA).await?;
        check_errors(&mut response).context("failed to define schema")?;
        debug!("SurrealDB schema ensured");
        Ok(())
    }

    /// Signs in again and re-selects the namespace/database after the server
    /// restarted or the connection dropped. The WebSocket client reconnects on
    /// its own; this restores the session on top of it.
    async fn recover(&self) -> Result<()> {
        self.db
            .signin(Database {
                namespace: self.cfg.namespace.clone(),
                database: self.cfg.database.clone(),
                username: self.cfg.username.clone(),
                password: self.cfg.password.clone(),
            })
            .await?;
        self.db
            .use_ns(&self.cfg.namespace)
            .use_db(&self.cfg.database)
            .await?;
        self.ensure_schema().await
    }

    /// Runs `query` with `vars` in one transaction, retrying with a growing
    /// delay on connection or statement errors. The data stays in memory
    /// until it is written.
    async fn run_with_retry(
        &self,
        what: &str,
        query: &str,
        vars: Vec<(String, Value)>,
    ) -> Result<()> {
        let mut attempt = 0;
        loop {
            let result = match tokio::time::timeout(
                self.attempt_timeout,
                self.run_once(query, vars.clone()),
            )
            .await
            {
                Ok(r) => r,
                Err(_) => Err(anyhow!(
                    "no answer from SurrealDB within {:?}",
                    self.attempt_timeout
                )),
            };
            match result {
                Ok(()) => return Ok(()),
                Err(e) => {
                    attempt += 1;
                    if let Some(max) = self.max_write_attempts
                        && attempt >= max
                    {
                        return Err(e.context(format!("{what} failed after {attempt} attempts")));
                    }
                    let delay = retry_delay(attempt - 1);
                    warn!("{what} failed (attempt {attempt}): {e:#}. Retrying in {delay:?}");
                    tokio::time::sleep(delay).await;
                    match tokio::time::timeout(self.attempt_timeout, self.recover()).await {
                        Ok(Ok(())) => {}
                        Ok(Err(re)) => debug!("Reconnect attempt failed: {re:#}"),
                        Err(_) => debug!("Reconnect attempt timed out"),
                    }
                }
            }
        }
    }

    async fn run_once(&self, query: &str, vars: Vec<(String, Value)>) -> Result<()> {
        let mut q = self.db.query(query);
        for var in vars {
            q = q.bind(var);
        }
        let mut response = q.await?;
        check_errors(&mut response)
    }

    /// Writes a batch of ledger updates in one transaction, retrying until it
    /// succeeds.
    pub async fn write_batch(&self, batch: &WriteBatch) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        let (query, vars) = batch_query(batch, None);
        self.run_with_retry("Batch write", &query, vars).await
    }

    /// Writes a batch of ACS contracts together with the ACS marker, so the
    /// stored resume point never runs ahead of the stored contracts.
    pub async fn write_acs_batch(
        &self,
        batch: &WriteBatch,
        acs_offset: i64,
        continuation: Option<Vec<u8>>,
        complete: bool,
    ) -> Result<()> {
        let marker = AcsMarker {
            acs_offset,
            acs_continuation_token: continuation,
            acs_complete: complete,
        };
        let (query, vars) = batch_query(batch, Some(&marker));
        self.run_with_retry("ACS batch write", &query, vars).await
    }

    /// The highest transaction offset written so far, if any.
    pub async fn last_processed_offset(&self) -> Result<Option<i64>> {
        let mut response = self
            .db
            .query("SELECT VALUE last_offset FROM ONLY type::record('sync_state', 'stream');")
            .await?;
        check_errors(&mut response)?;
        let value: Value = response.take(0)?;
        Ok(as_i64(&value))
    }

    /// Reads the ACS marker, if present.
    pub async fn read_acs_marker(&self) -> Result<Option<AcsMarker>> {
        let mut response = self
            .db
            .query("SELECT * FROM ONLY type::record('sync_state', 'singleton');")
            .await?;
        check_errors(&mut response)?;
        let value: Value = response.take(0)?;
        let Value::Object(obj) = value else {
            return Ok(None);
        };
        let Some(acs_offset) = obj.get("acs_offset").and_then(as_i64) else {
            return Ok(None);
        };
        let acs_continuation_token = match obj.get("acs_continuation_token") {
            Some(Value::Bytes(b)) => Some(b.to_vec()),
            _ => None,
        };
        let acs_complete = matches!(obj.get("acs_complete"), Some(Value::Bool(true)));
        Ok(Some(AcsMarker {
            acs_offset,
            acs_continuation_token,
            acs_complete,
        }))
    }

    /// Removes the ACS marker and every contract loaded from the ACS (and
    /// their edges), so a fresh ACS load can start cleanly.
    pub async fn reset_acs_state(&self) -> Result<()> {
        let mut response = self
            .db
            .query(
                "BEGIN;
                 DELETE type::record('sync_state', 'singleton');
                 DELETE created WHERE from_acs = true;
                 COMMIT;",
            )
            .await?;
        check_errors(&mut response)
    }

    /// Deletes every record the syncer wrote (for `--fresh`). Only the data
    /// is deleted; tables and indexes stay defined.
    pub async fn clear_all(&self) -> Result<()> {
        info!("Clearing all synced data from SurrealDB...");
        let mut response = self
            .db
            .query(
                "BEGIN;
                 DELETE action; DELETE consequence; DELETE target; DELETE consumes; DELETE requested;
                 DELETE transaction; DELETE created; DELETE exercised; DELETE party; DELETE sync_state;
                 COMMIT;",
            )
            .await?;
        check_errors(&mut response)?;
        info!("SurrealDB data cleared");
        Ok(())
    }
}

/// Returns an error listing every failed statement, or `Ok` if all succeeded.
fn check_errors(response: &mut surrealdb::IndexedResults) -> Result<()> {
    let errors = response.take_errors();
    if errors.is_empty() {
        return Ok(());
    }
    let mut list: Vec<_> = errors.into_iter().collect();
    list.sort_by_key(|(i, _)| *i);
    let msg = list
        .iter()
        .map(|(i, e)| format!("statement {i}: {e}"))
        .collect::<Vec<_>>()
        .join("; ");
    Err(anyhow!("SurrealDB returned errors: {msg}"))
}

fn as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(Number::Int(i)) => Some(*i),
        Value::Number(Number::Float(f)) => Some(*f as i64),
        _ => None,
    }
}

fn rows_value(rows: &[Row]) -> Value {
    Value::Array(
        rows.iter()
            .map(|r| {
                let mut o = Object::new();
                o.insert("id", Value::RecordId(r.id.clone()));
                o.insert("data", Value::Object(r.data.clone()));
                Value::Object(o)
            })
            .collect::<Array>(),
    )
}

fn edges_value(edges: &[Edge]) -> Value {
    Value::Array(
        edges
            .iter()
            .map(|e| {
                let mut o = Object::new();
                o.insert("id", Value::RecordId(e.id.clone()));
                o.insert("in", Value::RecordId(e.from.clone()));
                o.insert("out", Value::RecordId(e.to.clone()));
                Value::Object(o)
            })
            .collect::<Array>(),
    )
}

/// Builds the SurrealQL for one batch: a single transaction with at most one
/// statement per table. All data is passed as bound parameters.
pub fn batch_query(
    batch: &WriteBatch,
    marker: Option<&AcsMarker>,
) -> (String, Vec<(String, Value)>) {
    let mut query = String::from("BEGIN;\n");
    let mut vars: Vec<(String, Value)> = Vec::new();

    // Records first, so the edges below can find both ends.
    let rows: [(&str, &[Row]); 4] = [
        (TRANSACTION, &batch.transactions),
        (PARTY, &batch.parties),
        (CREATED, &batch.created),
        (EXERCISED, &batch.exercised),
    ];
    for (table, rows) in rows {
        if rows.is_empty() {
            continue;
        }
        let var = format!("{table}_rows");
        query.push_str(&format!(
            "FOR $r IN ${var} {{ UPSERT $r.id MERGE $r.data RETURN NONE; }};\n"
        ));
        vars.push((var, rows_value(rows)));
    }

    // Edges. A repeated update hits the same edge id and is ignored.
    // `target`/`consumes` point at contracts that may be outside the indexed
    // data (e.g. filtered out), so those edges are only written when the
    // contract exists, as the Neo4j MATCH did.
    let edges: [(&str, &[Edge], bool); 5] = [
        (ACTION, &batch.action, false),
        (CONSEQUENCE, &batch.consequence, false),
        (TARGET, &batch.target, true),
        (CONSUMES, &batch.consumes, true),
        (REQUESTED, &batch.requested, false),
    ];
    for (table, edges, only_existing) in edges {
        if edges.is_empty() {
            continue;
        }
        let var = format!("{table}_edges");
        let source = if only_existing {
            format!("${var}.filter(|$e| record::exists($e.out))")
        } else {
            format!("${var}")
        };
        query.push_str(&format!(
            "INSERT RELATION IGNORE INTO {table} {source} RETURN NONE;\n"
        ));
        vars.push((var, edges_value(edges)));
    }

    if let Some(max) = batch.max_offset {
        query.push_str(
            "UPSERT type::record('sync_state', 'stream') \
             SET last_offset = math::max([last_offset ?? $max_offset, $max_offset]) RETURN NONE;\n",
        );
        vars.push(("max_offset".into(), Value::Number(Number::Int(max))));
    }

    if let Some(m) = marker {
        let mut o = Object::new();
        o.insert("acs_offset", m.acs_offset);
        o.insert(
            "acs_continuation_token",
            match &m.acs_continuation_token {
                Some(t) => Value::Bytes(Bytes::from(t.clone())),
                None => Value::Null,
            },
        );
        o.insert("acs_complete", m.acs_complete);
        query.push_str(
            "UPSERT type::record('sync_state', 'singleton') MERGE $acs_marker RETURN NONE;\n",
        );
        vars.push(("acs_marker".into(), Value::Object(o)));
    }

    query.push_str("COMMIT;");
    (query, vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use surrealdb::types::RecordId;

    #[test]
    fn retry_delay_grows_and_caps_at_30s() {
        let secs: Vec<u64> = (0..8).map(|i| retry_delay(i).as_secs()).collect();
        assert_eq!(secs, vec![1, 2, 5, 10, 20, 30, 30, 30]);
    }

    #[test]
    fn empty_batch_only_has_transaction_wrapper() {
        let (q, vars) = batch_query(&WriteBatch::default(), None);
        assert_eq!(q, "BEGIN;\nCOMMIT;");
        assert!(vars.is_empty());
    }

    #[test]
    fn batch_query_binds_values_and_never_inlines_them() {
        let mut batch = WriteBatch::default();
        let mut data = Object::new();
        data.insert("secret_looking_value", "do-not-inline");
        batch.created.push(Row {
            id: RecordId::new(CREATED, "cid"),
            data,
        });
        let (q, vars) = batch_query(&batch, None);
        assert!(!q.contains("do-not-inline"));
        assert!(!q.contains("cid"));
        assert_eq!(vars.len(), 1);
        assert!(q.contains("FOR $r IN $created_rows"));
    }
}
