use anyhow::Result;
use ledger_api::v2::admin::{
    AllocatePartyRequest, party_management_service_client::PartyManagementServiceClient,
};
use tonic::Request;
use tonic::metadata::MetadataValue;

pub async fn allocate_parties(
    url: String,
    access_token: Option<&str>,
    party_hints: Vec<String>,
) -> Result<Vec<String>> {
    let mut client = PartyManagementServiceClient::connect(url).await?;
    let mut allocated_parties = Vec::new();

    for party_hint in party_hints {
        let request = AllocatePartyRequest {
            party_id_hint: party_hint,
            identity_provider_id: "".to_string(),
            local_metadata: None,
            synchronizer_id: "".to_string(),
            user_id: "".to_string(),
        };
        let mut req = Request::new(request);
        if let Some(token) = access_token {
            let meta = MetadataValue::try_from(format!("Bearer {}", token))?;
            req.metadata_mut().insert("authorization", meta);
        }
        let response = client.allocate_party(req).await?;
        if let Some(party_details) = response.into_inner().party_details {
            allocated_parties.push(party_details.party);
        }
    }

    Ok(allocated_parties)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party_management::get_parties::get_parties;
    use crate::testutils::start_sandbox;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_allocate_parties() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        tracing::info!("Starting test_allocate_parties");

        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let package_root = PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-asset");
        let dar_path = package_root
            .join(".daml")
            .join("dist")
            .join("daml-asset-0.0.1.dar");
        let sandbox_port = 6865;

        tracing::info!(?package_root, ?dar_path, sandbox_port, "Starting sandbox");
        let _guard = start_sandbox(package_root, dar_path, sandbox_port)
            .await
            .expect("Failed to start sandbox");
        tracing::info!("Sandbox started successfully");

        let url = format!("http://localhost:{}", sandbox_port);
        let party_hints = vec!["Alice".to_string(), "Bob".to_string()];

        tracing::info!(%url, ?party_hints, "Allocating parties");
        let allocated = allocate_parties(url.clone(), None, party_hints)
            .await
            .expect("Failed to allocate parties");
        tracing::info!(?allocated, "Parties allocated successfully");

        assert_eq!(allocated.len(), 2);
        assert!(allocated[0].contains("Alice"));
        assert!(allocated[1].contains("Bob"));

        // Verify allocated parties via get_parties
        tracing::info!("Verifying allocated parties via get_parties");
        let alice_parties = get_parties(url.clone(), None, Some("Alice".to_string()))
            .await
            .expect("Failed to get Alice parties");
        tracing::info!(?alice_parties, "Alice parties retrieved");
        assert!(!alice_parties.is_empty(), "Alice party should exist");
        assert!(alice_parties.iter().any(|p| p.contains("Alice")));

        let bob_parties = get_parties(url.clone(), None, Some("Bob".to_string()))
            .await
            .expect("Failed to get Bob parties");
        tracing::info!(?bob_parties, "Bob parties retrieved");
        assert!(!bob_parties.is_empty(), "Bob party should exist");
        assert!(bob_parties.iter().any(|p| p.contains("Bob")));

        let all_parties = get_parties(url, None, None)
            .await
            .expect("Failed to get all parties");
        tracing::info!(?all_parties, "All parties retrieved");
        assert!(all_parties.len() >= 2, "Should have at least 2 parties");

        tracing::info!("test_allocate_parties completed successfully");
    }
}
