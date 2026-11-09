use anyhow::{Context, Result};
use ledger_api::v2::{
    CumulativeFilter, EventFormat, Filters, GetActiveContractsRequest, Identifier, TemplateFilter,
    state_service_client::StateServiceClient,
};
use std::collections::HashMap;
use tonic::metadata::MetadataValue;
use tracing::{debug, info};

/// Result containing the created_event_blob for a contract, along with its synchronizer_id.
#[derive(Debug, Clone)]
pub struct ContractBlob {
    pub contract_id: String,
    pub created_event_blob: Vec<u8>,
    pub synchronizer_id: String,
}

/// Fetches the created_event_blob for all active contracts of a given template.
/// Returns a map from contract_id to ContractBlob.
///
/// # Arguments
/// * `url` - The gRPC endpoint URL of the ledger API
/// * `access_token` - Optional bearer token for authentication
/// * `parties` - The parties whose visibility to use for querying
/// * `template_id` - The template identifier to filter by
/// * `active_at_offset` - The offset at which to query the ACS (use ledger end for current state)
pub async fn get_blobs_by_template(
    url: &str,
    access_token: Option<&str>,
    parties: Vec<String>,
    template_id: Identifier,
    active_at_offset: i64,
) -> Result<HashMap<String, ContractBlob>> {
    info!(
        "Starting get_blobs_by_template: url={}, parties={:?}, template={:?}, active_at_offset={}",
        url, parties, template_id, active_at_offset
    );

    let mut result: HashMap<String, ContractBlob> = HashMap::new();

    debug!("Connecting to state service at {}", url);
    let mut client = StateServiceClient::connect(url.to_string())
        .await
        .with_context(|| format!("Failed to connect to state service at {}", url))?;

    // Build filters with template filter and include_created_event_blob = true
    let filters_by_party = build_template_filters_with_blob(&parties, &template_id);
    debug!("Built filters_by_party: {:?}", filters_by_party);

    let event_format = EventFormat {
        filters_by_party,
        filters_for_any_party: None,
        verbose: false,
    };

    let request = GetActiveContractsRequest {
        active_at_offset,
        event_format: Some(event_format),
        stream_continuation_token: None,
    };
    debug!("Created GetActiveContractsRequest: {:?}", request);

    let mut req = tonic::Request::new(request);
    if let Some(token) = access_token {
        debug!("Adding authorization token to request");
        let meta = MetadataValue::try_from(format!("Bearer {}", token))
            .with_context(|| "Failed to parse access token for metadata")?;
        req.metadata_mut().insert("authorization", meta);
    }

    debug!("Sending get_active_contracts request");
    let response = client
        .get_active_contracts(req)
        .await
        .with_context(|| "Failed to get active contracts from ledger")?;

    let mut stream = response.into_inner();

    while let Some(resp) = stream
        .message()
        .await
        .with_context(|| "Error reading from active contracts stream")?
    {
        if let Some(contract_entry) = resp.contract_entry {
            match contract_entry {
                ledger_api::v2::get_active_contracts_response::ContractEntry::ActiveContract(
                    active_contract,
                ) => {
                    if let Some(created_event) = active_contract.created_event {
                        debug!("Found contract: {}", created_event.contract_id);
                        result.insert(
                            created_event.contract_id.clone(),
                            ContractBlob {
                                contract_id: created_event.contract_id,
                                created_event_blob: created_event.created_event_blob,
                                synchronizer_id: active_contract.synchronizer_id,
                            },
                        );
                    }
                }
                ledger_api::v2::get_active_contracts_response::ContractEntry::IncompleteUnassigned(
                    incomplete,
                ) => {
                    if let Some(created_event) = incomplete.created_event {
                        debug!(
                            "Found contract in incomplete unassigned: {}",
                            created_event.contract_id
                        );
                        let synchronizer_id = incomplete
                            .unassigned_event
                            .map(|e| e.source)
                            .unwrap_or_default();
                        result.insert(
                            created_event.contract_id.clone(),
                            ContractBlob {
                                contract_id: created_event.contract_id,
                                created_event_blob: created_event.created_event_blob,
                                synchronizer_id,
                            },
                        );
                    }
                }
                ledger_api::v2::get_active_contracts_response::ContractEntry::IncompleteAssigned(
                    incomplete,
                ) => {
                    if let Some(assigned_event) = incomplete.assigned_event
                        && let Some(created_event) = assigned_event.created_event {
                            debug!(
                                "Found contract in incomplete assigned: {}",
                                created_event.contract_id
                            );
                            result.insert(
                                created_event.contract_id.clone(),
                                ContractBlob {
                                    contract_id: created_event.contract_id,
                                    created_event_blob: created_event.created_event_blob,
                                    synchronizer_id: assigned_event.target,
                                },
                            );
                        }
                }
            }
        }
    }

    info!(
        "Found {} contracts for template {:?}",
        result.len(),
        template_id
    );

    Ok(result)
}

/// Helper function to build filters_by_party with a template filter and include_created_event_blob = true.
fn build_template_filters_with_blob(
    parties: &[String],
    template_id: &Identifier,
) -> HashMap<String, Filters> {
    let mut filters_by_party = HashMap::new();
    for party in parties {
        filters_by_party.insert(
            party.clone(),
            Filters {
                cumulative: vec![CumulativeFilter {
                    identifier_filter: Some(
                        ledger_api::v2::cumulative_filter::IdentifierFilter::TemplateFilter(
                            TemplateFilter {
                                template_id: Some(template_id.clone()),
                                include_created_event_blob: true,
                            },
                        ),
                    ),
                }],
            },
        );
    }
    filters_by_party
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_template_filters_with_blob() {
        let parties = vec!["Alice".to_string(), "Bob".to_string()];
        let template_id = Identifier {
            package_id: "pkg123".to_string(),
            module_name: "Main".to_string(),
            entity_name: "Asset".to_string(),
        };
        let filters = build_template_filters_with_blob(&parties, &template_id);

        assert_eq!(filters.len(), 2);
        assert!(filters.contains_key("Alice"));
        assert!(filters.contains_key("Bob"));
    }
}
