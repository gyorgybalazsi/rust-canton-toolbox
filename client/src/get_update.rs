use crate::utils::build_filters_by_party;
use anyhow::{Context, Result};
use ledger_api::v2::{
    EventFormat, GetUpdateByIdRequest, GetUpdateResponse, TransactionFormat, TransactionShape,
    UpdateFormat, update_service_client::UpdateServiceClient,
};
use tonic::metadata::MetadataValue;
use tracing::{debug, error, info};

/// Looks up a single update by its ID via the Canton `GetUpdateById` gRPC API.
///
/// Returns the full `GetUpdateResponse`, which may be a Transaction, Reassignment,
/// or TopologyTransaction.
pub async fn get_update_by_id(
    url: &str,
    access_token: &str,
    parties: &[String],
    update_id: &str,
) -> Result<GetUpdateResponse> {
    info!("Looking up update_id={} at {}", update_id, url);

    debug!("Connecting to UpdateService at {}", url);
    let mut client = UpdateServiceClient::connect(url.to_string())
        .await
        .map(|c| c.max_decoding_message_size(64 * 1024 * 1024))
        .map_err(|e| {
            error!("Failed to connect to UpdateService at {}: {:?}", url, e);
            anyhow::anyhow!("Failed to connect to UpdateService at {}: {}", url, e)
        })?;

    let filters_by_party = build_filters_by_party(parties);

    let event_format = EventFormat {
        filters_by_party,
        filters_for_any_party: None,
        verbose: true,
    };

    let transaction_format = TransactionFormat {
        event_format: Some(event_format),
        transaction_shape: TransactionShape::LedgerEffects as i32,
    };

    let update_format = UpdateFormat {
        include_transactions: Some(transaction_format),
        include_reassignments: None,
        include_topology_events: None,
    };

    let request = GetUpdateByIdRequest {
        update_id: update_id.to_string(),
        update_format: Some(update_format),
    };

    let mut req = tonic::Request::new(request);
    debug!("Adding authorization token to request");
    let meta = MetadataValue::try_from(format!("Bearer {}", access_token))
        .context("Failed to parse access token for metadata")?;
    req.metadata_mut().insert("authorization", meta);

    debug!("Sending GetUpdateById request");
    let response = client.get_update_by_id(req).await.map_err(|e| {
        error!("GetUpdateById failed: {:?}", e);
        anyhow::anyhow!("GetUpdateById failed: {}", e)
    })?;

    info!("Successfully retrieved update {}", update_id);
    Ok(response.into_inner())
}
