use anyhow::Result;
use ledger_api::v2::admin::{
    ListUsersRequest, User, user_management_service_client::UserManagementServiceClient,
};
use tonic::Request;
use tonic::metadata::MetadataValue;

/// Lists all users on the participant node.
///
/// # Arguments
/// * `url` - The gRPC endpoint URL of the participant node
/// * `access_token` - Optional bearer token for authentication
/// * `identity_provider_id` - Optional identity provider ID filter
///
/// # Returns
/// A list of all users
pub async fn list_users(
    url: String,
    access_token: Option<&str>,
    identity_provider_id: Option<String>,
) -> Result<Vec<User>> {
    let mut client = UserManagementServiceClient::connect(url).await?;
    let mut all_users = Vec::new();
    let mut page_token = String::new();

    loop {
        let request = ListUsersRequest {
            page_token: page_token.clone(),
            page_size: 100,
            identity_provider_id: identity_provider_id.clone().unwrap_or_default(),
        };

        let mut req = Request::new(request);
        if let Some(token) = access_token {
            let meta = MetadataValue::try_from(format!("Bearer {}", token))?;
            req.metadata_mut().insert("authorization", meta);
        }

        let response = client.list_users(req).await?;
        let inner = response.into_inner();

        all_users.extend(inner.users);

        if inner.next_page_token.is_empty() {
            break;
        }
        page_token = inner.next_page_token;
    }

    Ok(all_users)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party_management::allocate_parties::allocate_parties;
    use crate::testutils::start_sandbox;
    use crate::user_management::create_user::{can_act_as, can_read_as, create_user};
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_create_and_list_users() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        tracing::info!("Starting test_create_and_list_users");

        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let package_root = PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-asset")
            .join("main");
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

        // First allocate parties for the users
        let party_hints = vec!["Alice".to_string(), "Bob".to_string()];
        tracing::info!(%url, ?party_hints, "Allocating parties");
        let allocated = allocate_parties(url.clone(), None, party_hints)
            .await
            .expect("Failed to allocate parties");
        let alice_party = &allocated[0];
        let bob_party = &allocated[1];
        tracing::info!(?alice_party, ?bob_party, "Parties allocated");

        // Create first user (Alice)
        let alice_user_id = "alice-user".to_string();
        let alice_rights = vec![can_act_as(alice_party), can_read_as(alice_party)];
        tracing::info!(%alice_user_id, "Creating Alice user");
        let created_alice = create_user(
            url.clone(),
            None,
            alice_user_id.clone(),
            Some(alice_party.clone()),
            alice_rights,
        )
        .await
        .expect("Failed to create Alice user");
        assert_eq!(created_alice, alice_user_id);
        tracing::info!("Alice user created successfully");

        // Create second user (Bob)
        let bob_user_id = "bob-user".to_string();
        let bob_rights = vec![can_act_as(bob_party), can_read_as(bob_party)];
        tracing::info!(%bob_user_id, "Creating Bob user");
        let created_bob = create_user(
            url.clone(),
            None,
            bob_user_id.clone(),
            Some(bob_party.clone()),
            bob_rights,
        )
        .await
        .expect("Failed to create Bob user");
        assert_eq!(created_bob, bob_user_id);
        tracing::info!("Bob user created successfully");

        // List all users and verify both exist
        tracing::info!("Listing all users");
        let users = list_users(url.clone(), None, None)
            .await
            .expect("Failed to list users");
        tracing::info!(?users, "Users listed");

        let user_ids: Vec<&str> = users.iter().map(|u| u.id.as_str()).collect();
        assert!(user_ids.contains(&"alice-user"), "Alice user should exist");
        assert!(user_ids.contains(&"bob-user"), "Bob user should exist");

        // Verify user details
        let alice = users.iter().find(|u| u.id == "alice-user").unwrap();
        assert!(
            alice.primary_party.contains("Alice"),
            "Alice should have Alice party as primary"
        );

        let bob = users.iter().find(|u| u.id == "bob-user").unwrap();
        assert!(
            bob.primary_party.contains("Bob"),
            "Bob should have Bob party as primary"
        );

        tracing::info!("test_create_and_list_users completed successfully");
    }
}
