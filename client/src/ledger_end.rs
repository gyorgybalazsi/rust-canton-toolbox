use anyhow::Result;
use ledger_api::v2::{
    GetLatestPrunedOffsetsRequest, GetLatestPrunedOffsetsResponse, GetLedgerEndRequest,
    GetLedgerEndResponse, state_service_client::StateServiceClient,
};
use tonic::Request;
use tonic::metadata::MetadataValue;

pub async fn get_ledger_end(url: &str, access_token: Option<&str>) -> Result<i64> {
    let mut state_service_client = StateServiceClient::connect(url.to_string()).await?;
    let mut req: Request<GetLedgerEndRequest> = Request::new(GetLedgerEndRequest {
        synchronizer_id: vec![],
    });
    if let Some(token) = access_token {
        let meta = MetadataValue::try_from(format!("Bearer {}", token))?;
        req.metadata_mut().insert("authorization", meta);
    }
    let ledger_end_response: tonic::Response<GetLedgerEndResponse> =
        state_service_client.get_ledger_end(req).await?;
    Ok(ledger_end_response.into_inner().offset)
}

/// Get the latest pruned offset from the ledger.
/// Returns the offset up to which the ledger has been pruned (exclusive for streaming).
/// If the ledger has not been pruned, returns 0.
pub async fn get_pruning_offset(url: &str, access_token: Option<&str>) -> Result<i64> {
    let mut state_service_client = StateServiceClient::connect(url.to_string()).await?;
    let mut req: Request<GetLatestPrunedOffsetsRequest> =
        Request::new(GetLatestPrunedOffsetsRequest {});
    if let Some(token) = access_token {
        let meta = MetadataValue::try_from(format!("Bearer {}", token))?;
        req.metadata_mut().insert("authorization", meta);
    }
    let response: tonic::Response<GetLatestPrunedOffsetsResponse> =
        state_service_client.get_latest_pruned_offsets(req).await?;
    Ok(response.into_inner().participant_pruned_up_to_inclusive)
}
