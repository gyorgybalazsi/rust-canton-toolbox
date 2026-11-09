use anyhow::Result;
use ledger_api::v2::admin::{
    ListKnownPartiesRequest, party_management_service_client::PartyManagementServiceClient,
};
use tonic::Request;
use tonic::metadata::MetadataValue;

pub async fn get_parties(
    url: String,
    access_token: Option<&str>,
    filter: Option<String>,
) -> Result<Vec<String>> {
    let mut client = PartyManagementServiceClient::connect(url).await?;
    let request = ListKnownPartiesRequest {
        page_token: "".to_string(),
        page_size: 0,
        identity_provider_id: "".to_string(),
        filter_party: "".to_string(),
    };
    let mut req = Request::new(request);
    if let Some(token) = access_token {
        let meta = MetadataValue::try_from(format!("Bearer {}", token))?;
        req.metadata_mut().insert("authorization", meta);
    }
    let response = client.list_known_parties(req).await?;
    let parties = response
        .into_inner()
        .party_details
        .into_iter()
        .map(|party_detail| party_detail.party)
        .filter(|party| {
            if let Some(ref f) = filter {
                party.contains(f)
            } else {
                true
            }
        })
        .collect::<Vec<String>>();
    Ok(parties)
}
