use anyhow::{Context, Result};
use async_stream::stream;
use futures::Stream;
use ledger_api::v2::{
    CreatedEvent, EventFormat, GetActiveContractsRequest, Identifier,
    state_service_client::StateServiceClient,
};
use std::collections::HashMap;
use std::pin::Pin;
use tonic::metadata::MetadataValue;
use tracing::{debug, info};

use crate::utils::build_filters_by_party_with_identifiers;

/// Represents an active contract from the ACS snapshot.
#[derive(Debug, Clone)]
pub struct ActiveContract {
    pub created_event: CreatedEvent,
    pub synchronizer_id: String,
}

/// Streams all active contracts for the given parties at a specific offset.
///
/// # Arguments
/// * `access_token` - Optional bearer token for authentication
/// * `active_at_offset` - The offset at which to query the ACS
/// * `continuation_token` - Optional token to resume an ACS snapshot from a prior checkpoint
/// * `parties` - The parties whose visibility to use for querying
/// * `url` - The gRPC endpoint URL of the ledger API
/// * `template_filters` - Optional list of template identifiers to filter on
///
/// Returns a stream of `(ActiveContract, Option<Vec<u8>>)` tuples where the second element
/// is the per-response continuation token (None if the response carried an empty token).
pub async fn stream_active_contracts(
    access_token: Option<&str>,
    active_at_offset: i64,
    continuation_token: Option<Vec<u8>>,
    parties: Vec<String>,
    url: String,
    template_filters: Option<&[Identifier]>,
) -> Result<Pin<Box<dyn Stream<Item = Result<(ActiveContract, Option<Vec<u8>>)>> + Send>>> {
    info!(
        "Starting stream_active_contracts: url={}, parties={:?}, active_at_offset={}, template_filters={:?}",
        url, parties, active_at_offset, template_filters
    );

    debug!("Connecting to state service at {}", url);
    let mut client = StateServiceClient::connect(url.clone())
        .await
        .with_context(|| format!("Failed to connect to state service at {}", url))?
        .max_decoding_message_size(64 * 1024 * 1024);

    let filters_by_party: HashMap<String, ledger_api::v2::Filters> =
        build_filters_by_party_with_identifiers(&parties, template_filters);
    debug!("Built filters_by_party: {:?}", filters_by_party);

    let event_format = EventFormat {
        filters_by_party,
        filters_for_any_party: None,
        verbose: true,
    };

    let request = GetActiveContractsRequest {
        active_at_offset,
        event_format: Some(event_format),
        stream_continuation_token: continuation_token,
        include_incomplete_reassignments: false,
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
    let response = client.get_active_contracts(req).await.map_err(|e| {
        tracing::error!(
            "gRPC error getting active contracts: code={:?}, message={}, details={:?}",
            e.code(),
            e.message(),
            e.details()
        );
        anyhow::anyhow!(
            "Failed to get active contracts from ledger: {} (code: {:?})",
            e.message(),
            e.code()
        )
    })?;

    let mut grpc_stream = response.into_inner();

    let output_stream = stream! {
        while let Some(resp) = grpc_stream
            .message()
            .await
            .transpose()
        {
            match resp {
                Ok(resp) => {
                    let cont_tok: Option<Vec<u8>> = if resp.stream_continuation_token.is_empty() {
                        None
                    } else {
                        Some(resp.stream_continuation_token.clone())
                    };
                    if let Some(contract_entry) = resp.contract_entry {
                        match contract_entry {
                            ledger_api::v2::get_active_contracts_response::ContractEntry::ActiveContract(
                                active_contract,
                            ) => {
                                if let Some(created_event) = active_contract.created_event {
                                    debug!("Found active contract: {}", created_event.contract_id);
                                    yield Ok((ActiveContract {
                                        created_event,
                                        synchronizer_id: active_contract.synchronizer_id,
                                    }, cont_tok.clone()));
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
                                    yield Ok((ActiveContract {
                                        created_event,
                                        synchronizer_id,
                                    }, cont_tok.clone()));
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
                                        yield Ok((ActiveContract {
                                            created_event,
                                            synchronizer_id: assigned_event.target,
                                        }, cont_tok.clone()));
                                    }
                            }
                        }
                    }
                }
                Err(e) => {
                    yield Err(anyhow::anyhow!("Error reading from active contracts stream: {}", e));
                }
            }
        }
    };

    Ok(Box::pin(output_stream))
}
