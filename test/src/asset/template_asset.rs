use anyhow::{Result, anyhow};
use client::submit_commands::CommandResult;
use client::submit_commands::submit_commands;
use daml_type_rep::built_in_types::{DamlParty, DamlText};
use daml_type_rep::lapi_access::LapiAccess;
use daml_type_rep::lapi_access::ToCreateArguments;
use daml_type_rep::template_id::TemplateId;
use derive_lapi_access::LapiAccess;
use derive_lapi_access::ToCreateArguments;
use ledger_api::v2::Record;
use ledger_api::v2::{
    Command, Commands, CreateCommand, ExerciseCommand,
    command_service_client::CommandServiceClient, value::Sum,
};
pub use submit::create_contract::create_contract;
use tracing::{debug, info};

pub async fn create_asset(
    command_service_client: &mut CommandServiceClient<tonic::transport::Channel>,
    access_token: Option<&str>,
    user_id: Option<&str>,
    package_id: String,
    payload: Asset,
) -> Result<(String, Option<Vec<u8>>)> {
    let act_as = payload.issuer.to_string();
    let create_asset_command = CreateCommand {
        template_id: Some(TemplateId::new(&package_id, "Main", "Asset").to_template_id()),
        create_arguments: Some(payload.to_create_arguments()),
    };

    let commands = Commands {
        act_as: vec![act_as],
        commands: vec![Command {
            command: Some(ledger_api::v2::command::Command::Create(
                create_asset_command,
            )),
        }],
        user_id: user_id.unwrap_or("").to_string(),
        command_id: format!("command-{}", uuid::Uuid::new_v4()),
        ..Default::default()
    };

    let result = submit_commands(command_service_client, access_token, commands, None).await?;
    let created_contracts: Vec<_> = result
        .iter()
        .filter_map(|r| {
            if let CommandResult::Created {
                contract_id,
                create_argument_blob,
            } = r
            {
                debug!("create_argument_blob: {:?}", create_argument_blob);
                Some((contract_id.clone(), create_argument_blob.clone()))
            } else {
                None
            }
        })
        .collect();

    if created_contracts.len() == 1 {
        Ok(created_contracts[0].clone())
    } else {
        Err(anyhow!(
            "Expected exactly one contract id, found {}",
            created_contracts.len()
        ))
    }
}

pub async fn exercise_give(
    command_service_client: &mut CommandServiceClient<tonic::transport::Channel>,
    access_token: Option<&str>,
    user_id: Option<&str>,
    package_id: String,
    contract_id: String,
    current_owner: String,
    new_owner: String,
) -> Result<(String, Option<Vec<u8>>)> {
    let exercise_command = ExerciseCommand {
        template_id: Some(TemplateId::new(&package_id, "Main", "Asset").to_template_id()),
        contract_id: contract_id.clone(),
        choice: "Give".to_string(),
        choice_argument: Some(Give::new(new_owner.clone()).to_lapi_value()),
        ..Default::default()
    };

    let commands = Commands {
        act_as: vec![current_owner.clone()],
        user_id: user_id.unwrap_or("").to_string(),
        commands: vec![Command {
            command: Some(ledger_api::v2::command::Command::Exercise(exercise_command)),
        }],
        command_id: format!("command-{}", uuid::Uuid::new_v4()),
        ..Default::default()
    };

    let result = submit_commands(command_service_client, access_token, commands, None).await?;
    let created_contracts: Vec<_> = result
        .iter()
        .filter_map(|r| {
            if let CommandResult::Created {
                contract_id,
                create_argument_blob,
            } = r
            {
                debug!("create_argument_blob: {:?}", create_argument_blob);
                Some((contract_id.clone(), create_argument_blob.clone()))
            } else {
                None
            }
        })
        .collect();

    if created_contracts.len() == 1 {
        Ok(created_contracts[0].clone())
    } else {
        Err(anyhow!(
            "Expected exactly one contract id, found {}",
            created_contracts.len()
        ))
    }
}

pub async fn exercise_get_view(
    command_service_client: &mut CommandServiceClient<tonic::transport::Channel>,
    access_token: Option<&str>,
    user_id: Option<&str>,
    package_id: String,
    contract_id: String,
    owner: String,
) -> Result<()> {
    let exercise_command = ExerciseCommand {
        template_id: Some(TemplateId::new(&package_id, "Main", "Asset").to_template_id()),
        contract_id: contract_id.clone(),
        choice: "GetView".to_string(),
        choice_argument: Some(ledger_api::v2::Value {
            sum: Some(Sum::Record(Record {
                record_id: None,
                fields: vec![],
            })),
        }),
        ..Default::default()
    };

    let commands = Commands {
        act_as: vec![owner.clone()],
        user_id: user_id.unwrap_or("").to_string(),
        commands: vec![Command {
            command: Some(ledger_api::v2::command::Command::Exercise(exercise_command)),
        }],
        command_id: format!("command-{}", uuid::Uuid::new_v4()),
        ..Default::default()
    };

    let result = submit_commands(command_service_client, access_token, commands, None).await?;
    info!("Length of result: {}", result.len());
    if let Some(CommandResult::ExerciseResult(value)) = result.first() {
        info!("Exercise GetView result: {:#?}", value);
    }
    Ok(())
}

#[derive(Debug, serde::Serialize, ToCreateArguments)]
pub struct Asset {
    issuer: DamlParty,
    owner: DamlParty,
    name: DamlText,
}

impl Asset {
    pub fn new(issuer: String, owner: String, name: String) -> Self {
        Asset {
            issuer: DamlParty::new(&issuer),
            owner: DamlParty::new(&owner),
            name: DamlText::new(name),
        }
    }
}

#[derive(Debug, PartialEq, serde::Serialize, LapiAccess)]
pub struct Give {
    new_owner: DamlParty,
}

impl Give {
    pub fn new(new_owner: String) -> Self {
        Give {
            new_owner: DamlParty::new(&new_owner),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

        // Run the setup script from the test DAR to allocate Alice
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

    #[tokio::test]
    async fn test_create_and_give_asset_simple() -> Result<()> {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("debug"))
            .with_test_writer()
            .pretty()
            .try_init();
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

        // Run the setup script from the test DAR
        let result = run_script("localhost", sandbox_port, &test_dar_path, "Test:setup")?;
        info!("Script result: {}", result);

        // Setup test values
        let package_id = "#daml-asset".to_string();
        let alice_user = "alice_user";
        let alice_token = fake_jwt_for_user(alice_user);
        let bob_user = "bob_user";
        let bob_token = fake_jwt_for_user(bob_user);
        let alice_parties =
            get_parties(url.clone(), Some(&alice_token), Some("Alice".to_string())).await?;
        let alice_party = alice_parties.first().cloned().unwrap();
        let bob_parties =
            get_parties(url.clone(), Some(&alice_token), Some("Bob".to_string())).await?;
        let bob_party = bob_parties.first().cloned().unwrap();

        // Connect to ledger
        let channel = tonic::transport::Channel::from_shared(url)
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut command_service_client = CommandServiceClient::new(channel);

        // Create asset
        let asset = Asset::new(
            alice_party.clone(), // issuer
            alice_party.clone(), // owner
            "Test asset".to_string(),
        );
        let create_result = create_asset(
            &mut command_service_client,
            Some(alice_token.as_str()),
            Some(alice_user),
            package_id.clone(),
            asset,
        )
        .await;

        assert!(
            create_result.is_ok(),
            "Asset creation failed: {:?}",
            create_result
        );
        let (created_contract_id, _blob) = create_result.unwrap();

        // Give asset
        let give_result = exercise_give(
            &mut command_service_client,
            Some(alice_token.as_str()),
            Some(alice_user),
            package_id.clone(),
            created_contract_id.clone(),
            alice_party.clone(), // current_owner
            bob_party.clone(),   // new_owner
        )
        .await;

        assert!(give_result.is_ok(), "Give asset failed: {:?}", give_result);

        let (given_contract_id, _blob) = give_result.unwrap();

        // Exercise GetView
        let get_view_result = exercise_get_view(
            &mut command_service_client,
            Some(bob_token.as_str()),
            Some(bob_user),
            package_id,
            given_contract_id,
            bob_party.clone(), // owner
        )
        .await;

        assert!(
            get_view_result.is_ok(),
            "GetView exercise failed: {:?}",
            get_view_result
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_run_script() -> Result<()> {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("debug"))
            .pretty()
            .try_init()
            .ok();

        let sandbox_port = 6865;
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

        // Run the setup script from the test DAR
        let result = run_script("localhost", sandbox_port, &test_dar_path, "Test:setup")?;

        info!("Script result: {}", result);

        Ok(())
    }
}
