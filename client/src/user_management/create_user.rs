use anyhow::Result;
use ledger_api::v2::admin::{
    CreateUserRequest, Right, User, user_management_service_client::UserManagementServiceClient,
};
use tonic::Request;
use tonic::metadata::MetadataValue;

/// Creates a new Canton user with the specified rights.
///
/// # Arguments
/// * `url` - The gRPC endpoint URL of the participant node
/// * `access_token` - Optional bearer token for authentication
/// * `user_id` - The unique identifier for the new user
/// * `primary_party` - Optional primary party for the user
/// * `rights` - List of rights to grant to the user
///
/// # Returns
/// The created user's ID on success
pub async fn create_user(
    url: String,
    access_token: Option<&str>,
    user_id: String,
    primary_party: Option<String>,
    rights: Vec<Right>,
) -> Result<String> {
    let mut client = UserManagementServiceClient::connect(url).await?;

    let user = User {
        id: user_id,
        primary_party: primary_party.unwrap_or_default(),
        is_deactivated: false,
        metadata: None,
        identity_provider_id: String::new(),
        primary_party_authentication: false,
    };

    let request = CreateUserRequest {
        user: Some(user),
        rights,
    };

    let mut req = Request::new(request);
    if let Some(token) = access_token {
        let meta = MetadataValue::try_from(format!("Bearer {}", token))?;
        req.metadata_mut().insert("authorization", meta);
    }

    let response = client.create_user(req).await?;
    let created_user = response
        .into_inner()
        .user
        .ok_or_else(|| anyhow::anyhow!("No user returned in response"))?;

    Ok(created_user.id)
}

/// Helper function to create a CanActAs right for a party
pub fn can_act_as(party: &str) -> Right {
    Right {
        kind: Some(ledger_api::v2::admin::right::Kind::CanActAs(
            ledger_api::v2::admin::right::CanActAs {
                party: party.to_string(),
            },
        )),
    }
}

/// Helper function to create a CanReadAs right for a party
pub fn can_read_as(party: &str) -> Right {
    Right {
        kind: Some(ledger_api::v2::admin::right::Kind::CanReadAs(
            ledger_api::v2::admin::right::CanReadAs {
                party: party.to_string(),
            },
        )),
    }
}

/// Helper function to create a ParticipantAdmin right
pub fn participant_admin() -> Right {
    Right {
        kind: Some(ledger_api::v2::admin::right::Kind::ParticipantAdmin(
            ledger_api::v2::admin::right::ParticipantAdmin {},
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::party_management::allocate_parties::allocate_parties;
    use crate::testutils::start_sandbox;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_create_user() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        tracing::info!("Starting test_create_user");

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

        // First allocate a party for the user
        let party_hints = vec!["TestUser".to_string()];
        tracing::info!(%url, ?party_hints, "Allocating party for user");
        let allocated = allocate_parties(url.clone(), None, party_hints)
            .await
            .expect("Failed to allocate party");
        let party = &allocated[0];
        tracing::info!(?party, "Party allocated");

        // Create a user with rights to act as and read as the party
        let user_id = "test-user".to_string();
        let rights = vec![can_act_as(party), can_read_as(party)];

        tracing::info!(%user_id, ?rights, "Creating user");
        let created_user_id = create_user(
            url.clone(),
            None,
            user_id.clone(),
            Some(party.clone()),
            rights,
        )
        .await
        .expect("Failed to create user");
        tracing::info!(?created_user_id, "User created successfully");

        assert_eq!(created_user_id, user_id);

        tracing::info!("test_create_user completed successfully");
    }
}
