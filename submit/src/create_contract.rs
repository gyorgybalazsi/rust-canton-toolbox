use anyhow::{Result, anyhow};
use client::submit_commands::CommandResult;
use client::submit_commands::submit_commands;
use daml_type_rep::lapi_access::ToCreateArguments;
use daml_type_rep::template_id::TemplateId;
use ledger_api::v2::{
    Command, Commands, CreateCommand, DisclosedContract,
    command_service_client::CommandServiceClient,
};

pub async fn create_contract<T: ToCreateArguments>(
    command_service_client: &mut CommandServiceClient<tonic::transport::Channel>,
    access_token: Option<&str>,
    user_id: Option<&str>,
    act_as: Vec<String>,
    template_id: TemplateId,
    payload: T,
    disclosed_contracts: Option<Vec<DisclosedContract>>,
) -> Result<String> {
    let create_command = CreateCommand {
        template_id: Some(template_id.to_template_id()),
        create_arguments: Some(payload.to_create_arguments()),
    };

    let commands = Commands {
        act_as,
        commands: vec![Command {
            command: Some(ledger_api::v2::command::Command::Create(create_command)),
        }],
        user_id: user_id.unwrap_or("").to_string(),
        command_id: format!("command-{}", uuid::Uuid::new_v4()),
        ..Default::default()
    };

    let result = submit_commands(
        command_service_client,
        access_token,
        commands,
        disclosed_contracts,
    )
    .await?;
    let contract_ids = result
        .iter()
        .filter_map(|r| {
            if let CommandResult::Created { contract_id, .. } = r {
                Some(contract_id.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    if contract_ids.len() == 1 {
        Ok(contract_ids[0].clone())
    } else {
        Err(anyhow!(
            "Expected exactly one contract id, found {}",
            contract_ids.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_types::asset::Asset;
    use client::jwt::fake_jwt_for_user;
    use client::party_management::get_parties::get_parties;
    use client::run_script::run_script;
    use client::testutils::start_sandbox;
    use tokio;
    use tracing::info;
    use tracing_subscriber::EnvFilter;

    #[tokio::test]
    async fn test_create_contract() -> Result<()> {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("debug"))
            .pretty()
            .try_init()
            .ok();
        let sandbox_port = 6865;
        let url = format!("http://localhost:{}", sandbox_port);
        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let package_root = std::path::PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-asset")
            .canonicalize()
            .expect("Failed to canonicalize package_root");

        info!("Starting DAML sandbox at {}", package_root.display());
        let dar_path = package_root
            .join("main")
            .join(".daml")
            .join("dist")
            .join("daml-asset-0.0.1.dar");
        let test_dar_path = package_root
            .join("test")
            .join(".daml")
            .join("dist")
            .join("daml-asset-test-0.0.1.dar");

        let _guard = start_sandbox(package_root, dar_path, sandbox_port).await?;

        // Allocate parties by running the test DAR's setup script
        let result = run_script("localhost", sandbox_port, &test_dar_path, "Test:setup")?;
        info!("Script result: {}", result);

        // Setup test values
        let package_id = "#daml-asset".to_string();
        let alice_user = "alice_user";
        let alice_token = fake_jwt_for_user(alice_user);
        let alice_parties =
            get_parties(url.clone(), Some(&alice_token), Some("Alice".to_string())).await?;
        let alice_party = alice_parties.first().cloned().unwrap();

        // Connect to ledger
        let channel = tonic::transport::Channel::from_shared(url)
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut command_service_client = CommandServiceClient::new(channel);

        // Create asset using the generic create_contract function
        let asset = Asset::new(
            alice_party.clone(), // issuer
            alice_party.clone(), // owner
            "Test asset".to_string(),
        );
        let template_id = TemplateId::new(&package_id, "Main", "Asset");
        let create_result = create_contract(
            &mut command_service_client,
            Some(alice_token.as_str()),
            Some(alice_user),
            vec![alice_party.clone()],
            template_id,
            asset,
            None, // no disclosed contracts
        )
        .await;

        assert!(
            create_result.is_ok(),
            "Contract creation failed: {:?}",
            create_result
        );
        let created_contract_id = create_result.unwrap();
        info!("Created contract with id: {}", created_contract_id);

        Ok(())
    }
}
