use ledger_api::v2::Value;

use ledger_api::v2::{CumulativeFilter, Filters, Identifier, TemplateFilter, WildcardFilter};
use std::collections::HashMap;

/// Recursively extracts all contract IDs from a Value.
/// Returns a Vec<String> of contract IDs found.
pub fn extract_contract_ids_from_value(value: &Option<Value>) -> Vec<String> {
    let mut result = Vec::new();
    if let Some(val) = value {
        match &val.sum {
            Some(ledger_api::v2::value::Sum::ContractId(cid)) => {
                result.push(cid.clone());
            }
            Some(ledger_api::v2::value::Sum::List(list)) => {
                for v in &list.elements {
                    result.extend(extract_contract_ids_from_value(&Some(v.clone())));
                }
            }
            _ => {}
        }
    }
    result
}

/// Helper function to build filters_by_party for a list of parties.
/// Uses wildcard filter to match all templates.
pub fn build_filters_by_party(parties: &[String]) -> HashMap<String, Filters> {
    build_filters_by_party_with_identifiers(parties, None)
}

/// Helper function to build filters_by_party for a list of parties with optional template filtering.
/// If template_filters is Some and non-empty, creates TemplateFilter entries.
/// If template_filters is None or empty, uses WildcardFilter to match all templates.
pub fn build_filters_by_party_with_identifiers(
    parties: &[String],
    template_filters: Option<&[Identifier]>,
) -> HashMap<String, Filters> {
    let mut filters_by_party = HashMap::new();

    let cumulative_filters: Vec<CumulativeFilter> = match template_filters {
        Some(identifiers) if !identifiers.is_empty() => {
            // Create a TemplateFilter for each specified template
            identifiers
                .iter()
                .map(|identifier| CumulativeFilter {
                    identifier_filter: Some(
                        ledger_api::v2::cumulative_filter::IdentifierFilter::TemplateFilter(
                            TemplateFilter {
                                template_id: Some(identifier.clone()),
                                include_created_event_blob: true,
                            },
                        ),
                    ),
                })
                .collect()
        }
        _ => {
            // Use wildcard filter to match all templates
            vec![CumulativeFilter {
                identifier_filter: Some(
                    ledger_api::v2::cumulative_filter::IdentifierFilter::WildcardFilter(
                        WildcardFilter {
                            include_created_event_blob: true,
                        },
                    ),
                ),
            }]
        }
    };

    for party in parties {
        filters_by_party.insert(
            party.clone(),
            Filters {
                cumulative: cumulative_filters.clone(),
            },
        );
    }
    filters_by_party
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureMarker {
    offset: i64, // Offset in the transaction stream
    node_id: i32,
    last_descendant_node_id: i32,
}

/// Returns a Vec of StructureMarkers for a Transaction.
pub fn structure_markers_from_transaction(
    transaction: &ledger_api::v2::Transaction,
) -> Vec<StructureMarker> {
    let mut markers = Vec::new();

    for event in &transaction.events {
        match &event.event {
            Some(ledger_api::v2::event::Event::Created(created)) => {
                markers.push(StructureMarker {
                    offset: created.offset,
                    node_id: created.node_id,
                    last_descendant_node_id: created.node_id,
                });
            }
            Some(ledger_api::v2::event::Event::Exercised(exercised)) => {
                markers.push(StructureMarker {
                    offset: exercised.offset,
                    node_id: exercised.node_id,
                    last_descendant_node_id: exercised.last_descendant_node_id,
                });
            }
            _ => {}
        }
    }

    markers
}

pub fn extract_edges(markers: &[StructureMarker]) -> Vec<(i64, i32, i32)> {
    // Sort markers by node_id to ensure traversal order
    let mut sorted = markers.to_vec();
    sorted.sort_by_key(|m| m.node_id);

    let mut stack: Vec<(i64, i32, i32)> = Vec::new(); // (node_id, last_descendant_node_id)
    let mut edges: Vec<(i64, i32, i32)> = Vec::new();

    for marker in &sorted {
        // Pop nodes whose descendants are already processed
        while let Some(&(_, _, last_desc)) = stack.last() {
            if last_desc < marker.node_id {
                stack.pop();
            } else {
                break;
            }
        }

        // If there's a parent on the stack, add an edge
        if let Some(&(_, parent_id, _)) = stack.last() {
            edges.push((marker.offset, parent_id, marker.node_id));
        }

        // Push the current node onto the stack
        stack.push((
            marker.offset,
            marker.node_id,
            marker.last_descendant_node_id,
        ));
    }

    edges
}
