use crate::api_record_to_json::{
    api_record_to_json, choice_argument_json, flatten_record_to_properties,
    flatten_value_to_properties,
};
use chrono::DateTime;
use client::utils::{
    extract_contract_ids_from_value, extract_edges, structure_markers_from_transaction,
};
use ledger_api::v2::{
    CreatedEvent, GetUpdatesResponse, event::Event, get_updates_response::Update,
};
use neo4rs::{BoltType, Query};
use serde_json::json;

/// Configuration for argument storage in Cypher generation
#[derive(Debug, Clone, Copy)]
pub struct FlattenConfig {
    pub enabled: bool,
    pub max_depth: usize,
    pub store_arguments_json: bool,
}

impl Default for FlattenConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_depth: 10,
            store_arguments_json: false,
        }
    }
}

/// Wrapper around neo4rs::Query that preserves the cypher string and params for debugging
#[derive(Clone)]
pub struct CypherQuery {
    pub cypher: String,
    pub params: Vec<(String, String)>,
    pub query: Query,
}

impl std::fmt::Debug for CypherQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CypherQuery")
            .field("cypher", &self.cypher)
            .field("params", &self.params)
            .finish()
    }
}

impl std::fmt::Display for CypherQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.cypher)?;
        if !self.params.is_empty() {
            write!(f, " [")?;
            for (i, (key, value)) in self.params.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}={}", key, value)?;
            }
            write!(f, "]")?;
        }
        Ok(())
    }
}

impl CypherQuery {
    pub fn new(cypher: String) -> Self {
        Self {
            query: Query::new(cypher.clone()),
            cypher,
            params: Vec::new(),
        }
    }

    pub fn with_param<T: Into<BoltType>>(mut self, key: &str, value: T) -> Self {
        self.query = self.query.param(key, value);
        self
    }

    pub fn with_json_param(mut self, key: &str, value: serde_json::Value) -> Self {
        // Convert serde_json::Value to BoltType
        let bolt_value: BoltType = value.try_into().unwrap_or(BoltType::Null(neo4rs::BoltNull));
        self.query = self.query.param(key, bolt_value);
        self
    }
}

macro_rules! cypher_query {
    ($cypher:expr, $($key:ident = $value:expr),* $(,)?) => {{
        let cypher_str = $cypher.to_string();
        let query = Query::new(cypher_str.clone())
            $(.param(stringify!($key), $value.clone()))*;
        let params = vec![
            $((stringify!($key).to_string(), format!("{:?}", $value))),*
        ];
        CypherQuery { cypher: cypher_str, params, query }
    }};
}

/// Converts a GetUpdatesResponse directly into a Vec of Cypher statements.
/// Uses UNWIND for batched operations to minimize round-trips.
/// Returns an empty vector if update is None or not a Transaction.
pub fn get_updates_response_to_cypher(
    response: &GetUpdatesResponse,
    flatten: FlattenConfig,
) -> Vec<CypherQuery> {
    let mut cypher_statements = Vec::new();

    let Some(update) = &response.update else {
        return cypher_statements;
    };

    let Update::Transaction(transaction) = update else {
        return cypher_statements;
    };

    let offset = transaction.offset;

    // Create Transaction node with metadata
    let effective_at = transaction
        .effective_at
        .as_ref()
        .and_then(|ts| {
            let dt = DateTime::from_timestamp(ts.seconds, ts.nanos as u32);
            dt.map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        })
        .unwrap_or_default();
    let record_time = transaction
        .record_time
        .as_ref()
        .and_then(|ts| {
            let dt = DateTime::from_timestamp(ts.seconds, ts.nanos as u32);
            dt.map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        })
        .unwrap_or_default();
    let traceparent = transaction
        .trace_context
        .as_ref()
        .and_then(|tc| tc.traceparent.clone())
        .unwrap_or_default();
    let tracestate = transaction
        .trace_context
        .as_ref()
        .and_then(|tc| tc.tracestate.clone())
        .unwrap_or_default();
    let label = format!("TX@{}", transaction.offset);
    // Use MERGE to avoid duplicates on reconnection
    cypher_statements.push(cypher_query!(
        "MERGE (t:Transaction { offset: $offset }) \
        ON CREATE SET \
        t.label = $label, \
        t.update_id = $update_id, \
        t.command_id = $command_id, \
        t.workflow_id = $workflow_id, \
        t.synchronizer_id = $synchronizer_id, \
        t.effective_at = $effective_at, \
        t.record_time = $record_time, \
        t.traceparent = $traceparent, \
        t.tracestate = $tracestate",
        label = label,
        update_id = transaction.update_id.clone(),
        command_id = transaction.command_id.clone(),
        workflow_id = transaction.workflow_id.clone(),
        offset = transaction.offset,
        synchronizer_id = transaction.synchronizer_id.clone(),
        effective_at = effective_at.clone(),
        record_time = record_time.clone(),
        traceparent = traceparent.clone(),
        tracestate = tracestate.clone(),
    ));

    // Collect Created events for batch insert
    let mut created_events: Vec<serde_json::Value> = Vec::new();
    // Collect Exercised events for batch insert
    let mut exercised_events: Vec<serde_json::Value> = Vec::new();

    for event in &transaction.events {
        match &event.event {
            Some(Event::Created(created)) => {
                let label = created
                    .template_id
                    .as_ref()
                    .map(|id| format!("{}@{}", id.entity_name, created.offset))
                    .unwrap_or_else(|| format!("unknown@{}", created.offset));
                let template_name = created
                    .template_id
                    .as_ref()
                    .map(|id| format!("{}.{}", id.module_name, id.entity_name))
                    .unwrap_or_else(|| "unknown".to_string());
                let signatories_str = created
                    .signatories
                    .iter()
                    .map(|s| format!("'{}'", s))
                    .collect::<Vec<_>>()
                    .join(", ");
                let created_at = created
                    .created_at
                    .as_ref()
                    .and_then(|ts| {
                        let dt = DateTime::from_timestamp(ts.seconds, ts.nanos as u32);
                        dt.map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                    })
                    .unwrap_or_default();
                let flattened_args: serde_json::Value = if flatten.enabled {
                    created
                        .create_arguments
                        .as_ref()
                        .map(|args| {
                            let props = flatten_record_to_properties(
                                args,
                                "create_arg.",
                                flatten.max_depth,
                            );
                            serde_json::Value::Object(props.into_iter().collect())
                        })
                        .unwrap_or(json!({}))
                } else {
                    json!({})
                };

                let mut event = json!({
                    "contract_id": created.contract_id,
                    "template_name": template_name,
                    "label": label,
                    "signatories": signatories_str,
                    "offset": created.offset,
                    "node_id": created.node_id,
                    "created_at": created_at,
                    "flattened_args": flattened_args
                });
                if flatten.store_arguments_json {
                    let create_arguments_json = created
                        .create_arguments
                        .as_ref()
                        .map(api_record_to_json)
                        .map(|json| serde_json::to_string(&json).unwrap_or("null".to_string()))
                        .unwrap_or("null".to_string());
                    let create_arguments = created
                        .create_arguments
                        .as_ref()
                        .map(|args| serde_json::to_string(args).unwrap_or("null".to_string()))
                        .unwrap_or("null".to_string());
                    event["create_arguments"] = json!(create_arguments);
                    event["create_arguments_json"] = json!(create_arguments_json);
                }
                created_events.push(event);
            }
            Some(Event::Exercised(exercised)) => {
                let label = format!("{}@{}", exercised.choice, exercised.offset);
                let choice_name = exercised.choice.clone();
                let acting_parties_str = exercised
                    .acting_parties
                    .iter()
                    .map(|s| format!("'{}'", s))
                    .collect::<Vec<_>>()
                    .join(", ");
                let transaction_effective_at = transaction
                    .effective_at
                    .as_ref()
                    .and_then(|ts| {
                        let dt = DateTime::from_timestamp(ts.seconds, ts.nanos as u32);
                        dt.map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                    })
                    .unwrap_or_default();
                let flattened_args: serde_json::Value = if flatten.enabled {
                    exercised
                        .choice_argument
                        .as_ref()
                        .map(|arg| {
                            let props =
                                flatten_value_to_properties(arg, "choice_arg.", flatten.max_depth);
                            serde_json::Value::Object(props.into_iter().collect())
                        })
                        .unwrap_or(json!({}))
                } else {
                    json!({})
                };

                let mut event = json!({
                    "label": label,
                    "choice_name": choice_name,
                    "target_contract_id": exercised.contract_id,
                    "acting_parties": acting_parties_str,
                    "offset": exercised.offset,
                    "node_id": exercised.node_id,
                    "consuming": exercised.consuming,
                    "result_contract_ids": serde_json::to_string(&extract_contract_ids_from_value(&exercised.exercise_result)).unwrap_or("[]".to_string()),
                    "last_descendant_node_id": exercised.last_descendant_node_id,
                    "transaction_effective_at": transaction_effective_at,
                    "flattened_args": flattened_args
                });
                if flatten.store_arguments_json {
                    let choice_argument_json_val = choice_argument_json(&exercised.choice_argument);
                    event["choice_argument"] = json!(
                        exercised
                            .choice_argument
                            .as_ref()
                            .map(|arg| serde_json::to_string(arg).unwrap_or("null".to_string()))
                            .unwrap_or("null".to_string())
                    );
                    event["choice_argument_json"] = json!(
                        serde_json::to_string(&choice_argument_json_val)
                            .unwrap_or("null".to_string())
                    );
                }
                exercised_events.push(event);
            }
            _ => {}
        }
    }

    // Batch MERGE Created nodes (use contract_id as unique key to avoid duplicates)
    if !created_events.is_empty() {
        let mut created_cypher = String::from(
            "UNWIND $props AS p \
            MERGE (c:Created { contract_id: p.contract_id }) \
            ON CREATE SET \
            c.template_name = p.template_name, \
            c.label = p.label, \
            c.signatories = p.signatories, \
            c.offset = p.offset, \
            c.node_id = p.node_id, \
            c.created_at = p.created_at",
        );
        if flatten.store_arguments_json {
            created_cypher.push_str(
                ", \
            c.create_arguments = p.create_arguments, \
            c.create_arguments_json = p.create_arguments_json",
            );
        }
        created_cypher.push_str(
            " \
            SET c += p.flattened_args",
        );
        let cypher = CypherQuery::new(created_cypher)
            .with_json_param("props", serde_json::Value::Array(created_events));
        cypher_statements.push(cypher);
    }

    // Batch MERGE Exercised nodes (use offset + node_id as unique key to avoid duplicates)
    if !exercised_events.is_empty() {
        let mut exercised_cypher = String::from(
            "UNWIND $props AS p \
            MERGE (e:Exercised { offset: p.offset, node_id: p.node_id }) \
            ON CREATE SET \
            e.label = p.label, \
            e.choice_name = p.choice_name, \
            e.target_contract_id = p.target_contract_id, \
            e.acting_parties = p.acting_parties, \
            e.consuming = p.consuming, \
            e.result_contract_ids = p.result_contract_ids, \
            e.last_descendant_node_id = p.last_descendant_node_id, \
            e.transaction_effective_at = p.transaction_effective_at",
        );
        if flatten.store_arguments_json {
            exercised_cypher.push_str(
                ", \
            e.choice_argument = p.choice_argument, \
            e.choice_argument_json = p.choice_argument_json",
            );
        }
        exercised_cypher.push_str(
            " \
            SET e += p.flattened_args",
        );
        let cypher = CypherQuery::new(exercised_cypher)
            .with_json_param("props", serde_json::Value::Array(exercised_events));
        cypher_statements.push(cypher);
    }

    // Batch CONSEQUENCE edges - split by child type for index usage
    // Parents are always Exercised (only exercises have consequences)
    // Children can be Created or Exercised
    let markers = structure_markers_from_transaction(transaction);
    let edges = extract_edges(&markers);
    if !edges.is_empty() {
        // Collect child node_ids that are Created vs Exercised for this transaction
        let created_node_ids: std::collections::HashSet<i32> = transaction
            .events
            .iter()
            .filter_map(|e| match &e.event {
                Some(Event::Created(c)) => Some(c.node_id),
                _ => None,
            })
            .collect();

        let mut edges_to_created: Vec<serde_json::Value> = Vec::new();
        let mut edges_to_exercised: Vec<serde_json::Value> = Vec::new();

        for (off, parent_id, child_id) in &edges {
            let edge_data = json!({
                "offset": off,
                "parent_id": parent_id,
                "child_id": child_id
            });
            if created_node_ids.contains(child_id) {
                edges_to_created.push(edge_data);
            } else {
                edges_to_exercised.push(edge_data);
            }
        }

        // Exercised -> Created edges (uses exercised_offset_node and created_offset_node indexes)
        if !edges_to_created.is_empty() {
            let cypher = CypherQuery::new(
                "UNWIND $edges AS e \
                MATCH (parent:Exercised {offset: e.offset, node_id: e.parent_id}), \
                (child:Created {offset: e.offset, node_id: e.child_id}) \
                MERGE (parent)-[:CONSEQUENCE]->(child)"
                    .to_string(),
            )
            .with_json_param("edges", serde_json::Value::Array(edges_to_created));
            cypher_statements.push(cypher);
        }

        // Exercised -> Exercised edges (uses exercised_offset_node index for both)
        if !edges_to_exercised.is_empty() {
            let cypher = CypherQuery::new(
                "UNWIND $edges AS e \
                MATCH (parent:Exercised {offset: e.offset, node_id: e.parent_id}), \
                (child:Exercised {offset: e.offset, node_id: e.child_id}) \
                MERGE (parent)-[:CONSEQUENCE]->(child)"
                    .to_string(),
            )
            .with_json_param("edges", serde_json::Value::Array(edges_to_exercised));
            cypher_statements.push(cypher);
        }
    }

    // Batch TARGET and CONSUMES relationships for Exercised events
    let mut target_rels: Vec<serde_json::Value> = Vec::new();
    let mut consumes_rels: Vec<serde_json::Value> = Vec::new();

    for event in &transaction.events {
        if let Some(Event::Exercised(exercised)) = &event.event {
            target_rels.push(json!({
                "offset": exercised.offset,
                "node_id": exercised.node_id,
                "target_contract_id": exercised.contract_id
            }));
            if exercised.consuming {
                consumes_rels.push(json!({
                    "offset": exercised.offset,
                    "node_id": exercised.node_id,
                    "target_contract_id": exercised.contract_id
                }));
            }
        }
    }

    if !target_rels.is_empty() {
        let cypher = CypherQuery::new(
            "UNWIND $rels AS r \
            MATCH (e:Exercised {offset: r.offset, node_id: r.node_id}), \
            (c:Created {contract_id: r.target_contract_id}) \
            MERGE (e)-[:TARGET]->(c)"
                .to_string(),
        )
        .with_json_param("rels", serde_json::Value::Array(target_rels));
        cypher_statements.push(cypher);
    }

    if !consumes_rels.is_empty() {
        let cypher = CypherQuery::new(
            "UNWIND $rels AS r \
            MATCH (e:Exercised {offset: r.offset, node_id: r.node_id}), \
            (c:Created {contract_id: r.target_contract_id}) \
            MERGE (e)-[:CONSUMES]->(c)"
                .to_string(),
        )
        .with_json_param("rels", serde_json::Value::Array(consumes_rels));
        cypher_statements.push(cypher);
    }

    // Identify root-level events (those not in any edge as a child)
    let child_node_ids: std::collections::HashSet<i32> =
        edges.iter().map(|(_, _, child)| *child).collect();

    // Collect root-level events for ACTION relationships
    let mut root_exercised: Vec<serde_json::Value> = Vec::new();
    let mut root_created: Vec<serde_json::Value> = Vec::new();
    let mut requesting_parties: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    for event in &transaction.events {
        if let Some(Event::Exercised(exercised)) = &event.event
            && !child_node_ids.contains(&exercised.node_id)
        {
            for party in &exercised.acting_parties {
                requesting_parties.insert(party.clone());
            }
            root_exercised.push(json!({
                "offset": offset,
                "node_id": exercised.node_id
            }));
        } else if let Some(Event::Created(created)) = &event.event
            && !child_node_ids.contains(&created.node_id)
        {
            for party in &created.signatories {
                requesting_parties.insert(party.clone());
            }
            root_created.push(json!({
                "offset": offset,
                "node_id": created.node_id
            }));
        }
    }

    // Batch ACTION relationships for root-level Exercised events
    if !root_exercised.is_empty() {
        let cypher = CypherQuery::new(
            "UNWIND $rels AS r \
            MATCH (t:Transaction {offset: r.offset}), \
            (e:Exercised {offset: r.offset, node_id: r.node_id}) \
            MERGE (t)-[:ACTION]->(e)"
                .to_string(),
        )
        .with_json_param("rels", serde_json::Value::Array(root_exercised));
        cypher_statements.push(cypher);
    }

    // Batch ACTION relationships for root-level Created events
    if !root_created.is_empty() {
        let cypher = CypherQuery::new(
            "UNWIND $rels AS r \
            MATCH (t:Transaction {offset: r.offset}), \
            (c:Created {offset: r.offset, node_id: r.node_id}) \
            MERGE (t)-[:ACTION]->(c)"
                .to_string(),
        )
        .with_json_param("rels", serde_json::Value::Array(root_created));
        cypher_statements.push(cypher);
    }

    // Batch Party MERGE and REQUESTED relationships
    if !requesting_parties.is_empty() {
        let parties: Vec<serde_json::Value> = requesting_parties
            .iter()
            .map(|p| json!({"party_id": p, "offset": offset}))
            .collect();

        // First MERGE all parties
        let merge_cypher = CypherQuery::new(
            "UNWIND $parties AS p \
            MERGE (party:Party {party_id: p.party_id})"
                .to_string(),
        )
        .with_json_param("parties", serde_json::Value::Array(parties.clone()));
        cypher_statements.push(merge_cypher);

        // Then MERGE REQUESTED relationships (avoid duplicates on reconnection)
        let rel_cypher = CypherQuery::new(
            "UNWIND $parties AS p \
            MATCH (party:Party {party_id: p.party_id}), \
            (t:Transaction {offset: p.offset}) \
            MERGE (party)-[:REQUESTED]->(t)"
                .to_string(),
        )
        .with_json_param("parties", serde_json::Value::Array(parties));
        cypher_statements.push(rel_cypher);
    }

    cypher_statements
}

/// Converts a CreatedEvent (from ACS) into Cypher statements to create a Created node.
/// The offset is set to -1 to indicate this is from ACS (pre-existing contract).
/// The node_id is set to 0 since there's no transaction structure for ACS contracts.
pub fn created_event_to_cypher(created: &CreatedEvent, flatten: FlattenConfig) -> Vec<CypherQuery> {
    let mut cypher_statements = Vec::new();

    let label = created
        .template_id
        .as_ref()
        .map(|id| format!("{}@ACS", id.entity_name))
        .unwrap_or_else(|| "unknown@ACS".to_string());
    let template_name = created
        .template_id
        .as_ref()
        .map(|id| format!("{}.{}", id.module_name, id.entity_name))
        .unwrap_or_else(|| "unknown".to_string());
    let signatories_str = created
        .signatories
        .iter()
        .map(|s| format!("'{}'", s))
        .collect::<Vec<_>>()
        .join(", ");
    let created_at = created
        .created_at
        .as_ref()
        .and_then(|ts| {
            let dt = DateTime::from_timestamp(ts.seconds, ts.nanos as u32);
            dt.map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        })
        .unwrap_or_default();
    let flattened_args: serde_json::Value = if flatten.enabled {
        created
            .create_arguments
            .as_ref()
            .map(|args| {
                let props = flatten_record_to_properties(args, "create_arg.", flatten.max_depth);
                serde_json::Value::Object(props.into_iter().collect())
            })
            .unwrap_or(json!({}))
    } else {
        json!({})
    };

    // Use MERGE to avoid duplicates if contract already exists
    // SET c += $flattened_args runs on both CREATE and MATCH to backfill existing nodes
    let mut cypher_str = String::from(
        "MERGE (c:Created { contract_id: $contract_id }) \
        ON CREATE SET \
        c.template_name = $template_name, \
        c.label = $label, \
        c.signatories = $signatories, \
        c.offset = $offset, \
        c.node_id = $node_id, \
        c.created_at = $created_at, \
        c.from_acs = true",
    );
    if flatten.store_arguments_json {
        cypher_str.push_str(
            ", \
        c.create_arguments = $create_arguments, \
        c.create_arguments_json = $create_arguments_json",
        );
    }
    cypher_str.push_str(
        " \
        SET c += $flattened_args",
    );

    let mut query = CypherQuery::new(cypher_str)
        .with_param("contract_id", created.contract_id.clone())
        .with_param("template_name", template_name)
        .with_param("label", label)
        .with_param("signatories", signatories_str)
        .with_param("offset", -1i64)
        .with_param("node_id", 0i64)
        .with_param("created_at", created_at)
        .with_json_param("flattened_args", flattened_args);

    if flatten.store_arguments_json {
        let create_arguments = created
            .create_arguments
            .as_ref()
            .map(|args| serde_json::to_string(args).unwrap_or("null".to_string()))
            .unwrap_or("null".to_string());
        let create_arguments_json = created
            .create_arguments
            .as_ref()
            .map(api_record_to_json)
            .map(|json| serde_json::to_string(&json).unwrap_or("null".to_string()))
            .unwrap_or("null".to_string());
        query = query
            .with_param("create_arguments", create_arguments)
            .with_param("create_arguments_json", create_arguments_json);
    }

    cypher_statements.push(query);
    cypher_statements
}
