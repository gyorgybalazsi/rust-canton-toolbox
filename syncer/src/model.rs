//! Turns ledger updates into the rows and edges written to SurrealDB.
//!
//! Tables (one per former Neo4j label):
//!
//! | table         | record id                     | was                 |
//! |---------------|-------------------------------|---------------------|
//! | `transaction` | `transaction:<offset>`        | `(:Transaction)`    |
//! | `created`     | `created:<contract_id>`       | `(:Created)`        |
//! | `exercised`   | `exercised:[<offset>, <node_id>]` | `(:Exercised)`  |
//! | `party`       | `party:<party_id>`            | `(:Party)`          |
//! | `sync_state`  | `sync_state:singleton` / `sync_state:stream` | `(:SyncState)` |
//!
//! Relation tables (one per former relationship type), each with a stable edge
//! id so that replaying an update never creates a second edge:
//!
//! | relation      | in → out                         | edge id                  |
//! |---------------|----------------------------------|--------------------------|
//! | `action`      | transaction → created/exercised  | `[offset, node_id]` of the child |
//! | `consequence` | exercised → created/exercised    | `[offset, node_id]` of the child |
//! | `target`      | exercised → created              | `[offset, node_id]` of the exercise |
//! | `consumes`    | exercised → created              | `[offset, node_id]` of the exercise |
//! | `requested`   | party → transaction              | `[party_id, offset]`     |

use std::collections::{HashMap, HashSet};

use client::utils::{
    extract_contract_ids_from_value, extract_edges, structure_markers_from_transaction,
};
use ledger_api::v2::{
    CreatedEvent, ExercisedEvent, GetUpdatesResponse, Identifier, Transaction, event::Event,
    get_updates_response::Update,
};
use surrealdb::types::{Array, Number, Object, RecordId, ToSql, Value};

use crate::daml_value::{daml_to_value, record_to_value, timestamp_to_datetime};

pub const TRANSACTION: &str = "transaction";
pub const CREATED: &str = "created";
pub const EXERCISED: &str = "exercised";
pub const PARTY: &str = "party";
pub const SYNC_STATE: &str = "sync_state";

pub const ACTION: &str = "action";
pub const CONSEQUENCE: &str = "consequence";
pub const TARGET: &str = "target";
pub const CONSUMES: &str = "consumes";
pub const REQUESTED: &str = "requested";

/// A record to upsert: `UPSERT <id> MERGE <data>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: RecordId,
    pub data: Object,
}

/// An edge to insert into a relation table, with a stable id.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub id: RecordId,
    pub from: RecordId,
    pub to: RecordId,
}

/// Everything one or more ledger updates write to the database.
///
/// Rows of the same kind are grouped so a whole batch is written with a fixed
/// number of statements, no matter how many updates it holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WriteBatch {
    pub transactions: Vec<Row>,
    pub parties: Vec<Row>,
    pub created: Vec<Row>,
    pub exercised: Vec<Row>,
    pub action: Vec<Edge>,
    pub consequence: Vec<Edge>,
    pub target: Vec<Edge>,
    pub consumes: Vec<Edge>,
    pub requested: Vec<Edge>,
    /// Highest transaction offset in the batch, used as the resume point.
    pub max_offset: Option<i64>,
}

impl WriteBatch {
    pub fn is_empty(&self) -> bool {
        self.row_count() == 0 && self.max_offset.is_none()
    }

    /// Number of records plus edges in the batch.
    pub fn row_count(&self) -> usize {
        self.transactions.len()
            + self.parties.len()
            + self.created.len()
            + self.exercised.len()
            + self.action.len()
            + self.consequence.len()
            + self.target.len()
            + self.consumes.len()
            + self.requested.len()
    }

    /// Appends another batch to this one.
    pub fn extend(&mut self, other: WriteBatch) {
        self.transactions.extend(other.transactions);
        self.parties.extend(other.parties);
        self.created.extend(other.created);
        self.exercised.extend(other.exercised);
        self.action.extend(other.action);
        self.consequence.extend(other.consequence);
        self.target.extend(other.target);
        self.consumes.extend(other.consumes);
        self.requested.extend(other.requested);
        self.max_offset = match (self.max_offset, other.max_offset) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }

    /// Human-readable dump of the batch, for the `print-surql` command.
    pub fn to_surql_preview(&self) -> String {
        let mut out = String::new();
        let rows = [
            ("transactions", &self.transactions),
            ("parties", &self.parties),
            ("created", &self.created),
            ("exercised", &self.exercised),
        ];
        for (name, rows) in rows {
            for row in rows {
                out.push_str(&format!(
                    "-- {name}\nUPSERT {} MERGE {};\n",
                    row.id.to_sql(),
                    row.data.to_sql()
                ));
            }
        }
        let edges = [
            (ACTION, &self.action),
            (CONSEQUENCE, &self.consequence),
            (TARGET, &self.target),
            (CONSUMES, &self.consumes),
            (REQUESTED, &self.requested),
        ];
        for (name, edges) in edges {
            for e in edges {
                out.push_str(&format!(
                    "INSERT RELATION IGNORE INTO {name} {{ id: {}, in: {}, out: {} }};\n",
                    e.id.to_sql(),
                    e.from.to_sql(),
                    e.to.to_sql()
                ));
            }
        }
        out
    }
}

pub fn transaction_id(offset: i64) -> RecordId {
    RecordId::new(TRANSACTION, offset)
}

pub fn created_id(contract_id: &str) -> RecordId {
    RecordId::new(CREATED, contract_id)
}

pub fn exercised_id(offset: i64, node_id: i32) -> RecordId {
    RecordId::new(EXERCISED, offset_node_key(offset, node_id))
}

pub fn party_id(party: &str) -> RecordId {
    RecordId::new(PARTY, party)
}

fn offset_node_key(offset: i64, node_id: i32) -> Array {
    Array::from(vec![
        Value::Number(Number::Int(offset)),
        Value::Number(Number::Int(i64::from(node_id))),
    ])
}

fn edge(table: &str, key: Array, from: RecordId, to: RecordId) -> Edge {
    Edge {
        id: RecordId::new(table, key),
        from,
        to,
    }
}

fn strings(items: &[String]) -> Value {
    Value::Array(items.iter().cloned().collect())
}

/// The short name of a party (`alice` for `alice::1220...`), used as its label.
fn party_display_name(party: &str) -> &str {
    party.split("::").next().unwrap_or(party)
}

/// `{ package_id, module_name, entity_name }` of a template id.
fn template_value(id: &Identifier) -> Value {
    let mut o = Object::new();
    o.insert("package_id", id.package_id.clone());
    o.insert("module_name", id.module_name.clone());
    o.insert("entity_name", id.entity_name.clone());
    Value::Object(o)
}

/// Builds the `created` row for a created event. `name_suffix` is the offset
/// for streamed events and `ACS` for contracts loaded from the active set.
fn created_row(created: &CreatedEvent, offset: i64, node_id: i32, from_acs: bool) -> Row {
    let name_suffix = if from_acs {
        "ACS".to_string()
    } else {
        offset.to_string()
    };
    let mut data = Object::new();
    data.insert("contract_id", created.contract_id.clone());
    match &created.template_id {
        Some(id) => {
            data.insert("name", format!("{}@{}", id.entity_name, name_suffix));
            data.insert(
                "template_name",
                format!("{}.{}", id.module_name, id.entity_name),
            );
            data.insert("template", template_value(id));
        }
        None => {
            data.insert("name", format!("unknown@{name_suffix}"));
            data.insert("template_name", "unknown");
        }
    }
    data.insert("signatories", strings(&created.signatories));
    data.insert("offset", offset);
    data.insert("node_id", i64::from(node_id));
    if let Some(dt) = created.created_at.as_ref().and_then(timestamp_to_datetime) {
        data.insert("created_at", Value::Datetime(dt));
    }
    if let Some(args) = &created.create_arguments {
        data.insert("create_arguments", record_to_value(args));
    }
    data.insert("from_acs", from_acs);
    Row {
        id: created_id(&created.contract_id),
        data,
    }
}

fn exercised_row(exercised: &ExercisedEvent, transaction: &Transaction) -> Row {
    let mut data = Object::new();
    data.insert("name", format!("{}@{}", exercised.choice, exercised.offset));
    data.insert("choice_name", exercised.choice.clone());
    data.insert("target_contract_id", exercised.contract_id.clone());
    data.insert("acting_parties", strings(&exercised.acting_parties));
    data.insert("consuming", exercised.consuming);
    data.insert(
        "result_contract_ids",
        Value::Array(
            extract_contract_ids_from_value(&exercised.exercise_result)
                .into_iter()
                .collect(),
        ),
    );
    data.insert("offset", exercised.offset);
    data.insert("node_id", i64::from(exercised.node_id));
    data.insert(
        "last_descendant_node_id",
        i64::from(exercised.last_descendant_node_id),
    );
    if let Some(dt) = transaction
        .effective_at
        .as_ref()
        .and_then(timestamp_to_datetime)
    {
        data.insert("transaction_effective_at", Value::Datetime(dt));
    }
    if let Some(arg) = &exercised.choice_argument {
        data.insert("choice_argument", daml_to_value(arg));
    }
    Row {
        id: exercised_id(exercised.offset, exercised.node_id),
        data,
    }
}

fn transaction_row(tx: &Transaction) -> Row {
    let mut data = Object::new();
    data.insert("offset", tx.offset);
    data.insert("name", format!("TX@{}", tx.offset));
    data.insert("update_id", tx.update_id.clone());
    data.insert("command_id", tx.command_id.clone());
    data.insert("workflow_id", tx.workflow_id.clone());
    data.insert("synchronizer_id", tx.synchronizer_id.clone());
    if let Some(dt) = tx.effective_at.as_ref().and_then(timestamp_to_datetime) {
        data.insert("effective_at", Value::Datetime(dt));
    }
    if let Some(dt) = tx.record_time.as_ref().and_then(timestamp_to_datetime) {
        data.insert("record_time", Value::Datetime(dt));
    }
    if let Some(tc) = &tx.trace_context {
        let mut trace = Object::new();
        if let Some(p) = &tc.traceparent {
            trace.insert("traceparent", p.clone());
        }
        if let Some(s) = &tc.tracestate {
            trace.insert("tracestate", s.clone());
        }
        data.insert("trace_context", Value::Object(trace));
    }
    Row {
        id: transaction_id(tx.offset),
        data,
    }
}

fn party_row(party: &str) -> Row {
    let mut data = Object::new();
    data.insert("party_id", party.to_string());
    data.insert("name", party_display_name(party).to_string());
    Row {
        id: party_id(party),
        data,
    }
}

/// Converts one update from the ledger stream into the rows and edges it
/// writes. Only transactions produce data; other updates give an empty batch.
pub fn update_to_batch(response: &GetUpdatesResponse) -> WriteBatch {
    match &response.update {
        Some(Update::Transaction(tx)) => transaction_to_batch(tx),
        _ => WriteBatch::default(),
    }
}

/// Converts a transaction into its rows and edges.
pub fn transaction_to_batch(tx: &Transaction) -> WriteBatch {
    let mut batch = WriteBatch {
        max_offset: Some(tx.offset),
        ..Default::default()
    };
    let offset = tx.offset;
    let tx_id = transaction_id(offset);
    batch.transactions.push(transaction_row(tx));

    // node_id -> record id of the event, so edges can point at the right table.
    let mut node_ids: HashMap<i32, RecordId> = HashMap::new();
    for event in &tx.events {
        match &event.event {
            Some(Event::Created(created)) => {
                let row = created_row(created, created.offset, created.node_id, false);
                node_ids.insert(created.node_id, row.id.clone());
                batch.created.push(row);
            }
            Some(Event::Exercised(exercised)) => {
                let row = exercised_row(exercised, tx);
                node_ids.insert(exercised.node_id, row.id.clone());
                batch.exercised.push(row);

                let key = offset_node_key(exercised.offset, exercised.node_id);
                let from = exercised_id(exercised.offset, exercised.node_id);
                let to = created_id(&exercised.contract_id);
                batch
                    .target
                    .push(edge(TARGET, key.clone(), from.clone(), to.clone()));
                if exercised.consuming {
                    batch.consumes.push(edge(CONSUMES, key, from, to));
                }
            }
            _ => {}
        }
    }

    // Parent/child structure inside the transaction.
    let edges = extract_edges(&structure_markers_from_transaction(tx));
    let mut child_ids: HashSet<i32> = HashSet::new();
    for (edge_offset, parent, child) in &edges {
        child_ids.insert(*child);
        if let (Some(from), Some(to)) = (node_ids.get(parent), node_ids.get(child)) {
            batch.consequence.push(edge(
                CONSEQUENCE,
                offset_node_key(*edge_offset, *child),
                from.clone(),
                to.clone(),
            ));
        }
    }

    // Root events hang off the transaction; their parties requested it.
    let mut requesting: Vec<String> = Vec::new();
    for event in &tx.events {
        let (node_id, parties) = match &event.event {
            Some(Event::Created(c)) => (c.node_id, &c.signatories),
            Some(Event::Exercised(e)) => (e.node_id, &e.acting_parties),
            _ => continue,
        };
        if child_ids.contains(&node_id) {
            continue;
        }
        if let Some(to) = node_ids.get(&node_id) {
            batch.action.push(edge(
                ACTION,
                offset_node_key(offset, node_id),
                tx_id.clone(),
                to.clone(),
            ));
        }
        for p in parties {
            if !requesting.contains(p) {
                requesting.push(p.clone());
            }
        }
    }
    for p in &requesting {
        batch.parties.push(party_row(p));
        let key = Array::from(vec![
            Value::String(p.clone()),
            Value::Number(Number::Int(offset)),
        ]);
        batch
            .requested
            .push(edge(REQUESTED, key, party_id(p), tx_id.clone()));
    }

    batch
}

/// Converts a contract from the active contract set into a `created` row.
/// Its offset is -1 and node id 0, since it has no transaction in the graph.
pub fn acs_created_to_batch(created: &CreatedEvent) -> WriteBatch {
    WriteBatch {
        created: vec![created_row(created, -1, 0, true)],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_api::v2::{
        Event as EventWrapper, Record, RecordField, Value as DamlValue, value::Sum,
    };

    fn template() -> Identifier {
        Identifier {
            package_id: "#pkg".into(),
            module_name: "Main".into(),
            entity_name: "Asset".into(),
        }
    }

    fn sample_tx() -> Transaction {
        let created = CreatedEvent {
            offset: 10,
            node_id: 1,
            contract_id: "cid-new".into(),
            template_id: Some(template()),
            signatories: vec!["alice::1220".into()],
            create_arguments: Some(Record {
                record_id: None,
                fields: vec![RecordField {
                    label: "owner".into(),
                    value: Some(DamlValue {
                        sum: Some(Sum::Party("alice::1220".into())),
                    }),
                }],
            }),
            ..Default::default()
        };
        let exercised = ExercisedEvent {
            offset: 10,
            node_id: 0,
            contract_id: "cid-old".into(),
            choice: "Transfer".into(),
            acting_parties: vec!["alice::1220".into()],
            consuming: true,
            last_descendant_node_id: 1,
            ..Default::default()
        };
        Transaction {
            offset: 10,
            update_id: "u1".into(),
            events: vec![
                EventWrapper {
                    event: Some(Event::Exercised(exercised)),
                },
                EventWrapper {
                    event: Some(Event::Created(created)),
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn transaction_produces_records_and_edges() {
        let b = transaction_to_batch(&sample_tx());
        assert_eq!(b.max_offset, Some(10));
        assert_eq!(b.transactions.len(), 1);
        assert_eq!(b.transactions[0].id, transaction_id(10));
        assert_eq!(b.created.len(), 1);
        assert_eq!(b.created[0].id, created_id("cid-new"));
        assert_eq!(b.exercised[0].id, exercised_id(10, 0));
        // exercise is the root; the created contract is its consequence
        assert_eq!(b.action.len(), 1);
        assert_eq!(b.action[0].to, exercised_id(10, 0));
        assert_eq!(b.consequence.len(), 1);
        assert_eq!(b.consequence[0].from, exercised_id(10, 0));
        assert_eq!(b.consequence[0].to, created_id("cid-new"));
        assert_eq!(b.target[0].to, created_id("cid-old"));
        assert_eq!(b.consumes.len(), 1);
        assert_eq!(b.parties[0].id, party_id("alice::1220"));
        assert_eq!(b.requested[0].to, transaction_id(10));
    }

    #[test]
    fn signatories_are_an_array_and_arguments_nested() {
        let b = transaction_to_batch(&sample_tx());
        let data = &b.created[0].data;
        assert!(matches!(data.get("signatories"), Some(Value::Array(a)) if a.len() == 1));
        match data.get("create_arguments") {
            Some(Value::Object(o)) => {
                assert_eq!(o.get("owner"), Some(&Value::String("alice::1220".into())))
            }
            other => panic!("expected nested object, got {other:?}"),
        }
        assert_eq!(data.get("name"), Some(&Value::String("Asset@10".into())));
    }

    #[test]
    fn batches_merge() {
        let mut a = transaction_to_batch(&sample_tx());
        let mut tx = sample_tx();
        tx.offset = 12;
        a.extend(transaction_to_batch(&tx));
        assert_eq!(a.transactions.len(), 2);
        assert_eq!(a.max_offset, Some(12));
    }
}
