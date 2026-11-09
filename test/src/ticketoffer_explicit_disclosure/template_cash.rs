use anyhow::{Result, anyhow};
use client::submit_commands::CommandResult;
use client::submit_commands::submit_commands;
use daml_type_rep::built_in_types::{DamlDecimal, DamlParty};
use daml_type_rep::lapi_access::LapiAccess;
use daml_type_rep::lapi_access::ToCreateArguments;
use daml_type_rep::template_id::TemplateId;
use derive_lapi_access::LapiAccess;
use derive_lapi_access::ToCreateArguments;
use ledger_api::v2::{
    Command, Commands, CreateCommand, Record, command_service_client::CommandServiceClient,
};

#[derive(Clone, Debug, serde::Serialize, ToCreateArguments)]
pub struct Cash {
    pub issuer: DamlParty,
    pub owner: DamlParty,
    pub amount: DamlDecimal,
}

impl Cash {
    pub fn new(issuer: DamlParty, owner: DamlParty, amount: DamlDecimal) -> Self {
        Cash {
            issuer,
            owner,
            amount,
        }
    }
}

/// Result of creating a cash contract, including the blob for explicit disclosure
#[derive(Debug)]
pub struct CreateCashResult {
    pub contract_id: String,
    pub created_event_blob: Option<Vec<u8>>,
}

pub async fn create_cash(
    command_service_client: &mut CommandServiceClient<tonic::transport::Channel>,
    access_token: Option<&str>,
    user_id: Option<&str>,
    package_id: &str,
    issuer: String,
    owner: String,
    amount: f64,
) -> Result<CreateCashResult> {
    let create_cash_command = CreateCommand {
        template_id: Some(TemplateId::new(package_id, "Main", "Cash").to_template_id()),
        create_arguments: Some(
            Cash::new(
                DamlParty::new(&issuer),
                DamlParty::new(&owner),
                DamlDecimal::new(amount),
            )
            .to_create_arguments(),
        ),
    };

    let commands = Commands {
        act_as: vec![issuer.clone()],
        commands: vec![Command {
            command: Some(ledger_api::v2::command::Command::Create(
                create_cash_command,
            )),
        }],
        user_id: user_id.unwrap_or("").to_string(),
        command_id: format!("command-{}", uuid::Uuid::new_v4()),
        ..Default::default()
    };

    let result = submit_commands(command_service_client, access_token, commands, None).await?;
    if let Some(CommandResult::Created {
        contract_id,
        create_argument_blob,
    }) = result.first()
    {
        Ok(CreateCashResult {
            contract_id: contract_id.clone(),
            created_event_blob: create_argument_blob.clone(),
        })
    } else {
        Err(anyhow!("No contract id found in create_cash result"))
    }
}

#[derive(serde::Serialize, LapiAccess)]
/// Represents the Transfer choice on the Cash template.
pub struct Transfer {
    pub new_owner: DamlParty,
}

impl Transfer {
    pub fn new(new_owner: DamlParty) -> Self {
        Transfer { new_owner }
    }
}

pub async fn exercise_transfer(
    command_service_client: &mut CommandServiceClient<tonic::transport::Channel>,
    access_token: Option<&str>,
    user_id: Option<&str>,
    package_id: &str,
    contract_id: String,
    new_owner: String,
    current_owner: String,
) -> Result<()> {
    let exercise_command = ledger_api::v2::ExerciseCommand {
        template_id: Some(TemplateId::new(package_id, "Main", "Cash").to_template_id()),
        contract_id,
        choice: "Transfer".to_string(),
        choice_argument: Some(Transfer::new(DamlParty::new(&new_owner)).to_lapi_value()),
        ..Default::default()
    };

    let commands = Commands {
        act_as: vec![current_owner],
        user_id: user_id.unwrap_or("").to_string(),
        commands: vec![Command {
            command: Some(ledger_api::v2::command::Command::Exercise(exercise_command)),
        }],
        command_id: format!("command-{}", uuid::Uuid::new_v4()),
        ..Default::default()
    };

    submit_commands(command_service_client, access_token, commands, None).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::jwt::fake_jwt_for_user;
    use client::party_management::get_parties::get_parties;
    use client::run_script::run_script;
    use client::testutils::start_sandbox;
    use std::path::PathBuf;
    use tokio;
    use tracing::info;

    #[tokio::test]
    async fn test_create_and_transfer_cash() -> Result<()> {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let sandbox_port = 6865;
        let url = format!("http://localhost:{}", sandbox_port);
        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        // Use PathBuf for package_root
        let package_root = PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-ticketoffer-explicit-disclosure")
            .canonicalize()
            .unwrap();

        let dar_path = package_root
            .join("main")
            .join(".daml")
            .join("dist")
            .join("daml-ticketoffer-explicit-disclosure-0.0.1.dar");
        let _guard = start_sandbox(package_root.clone(), dar_path, sandbox_port).await?;

        // Run the setup script from the test DAR
        let test_dar_path = package_root
            .join("test")
            .join(".daml")
            .join("dist")
            .join("daml-ticketoffer-explicit-disclosure-test-0.0.1.dar");
        let script_result = run_script("localhost", sandbox_port, &test_dar_path, "Setup:setup")?;
        info!("Script result: {}", script_result);

        // Setup test values
        let package_id = "#daml-ticketoffer-explicit-disclosure".to_string();

        let alice_user = "aliceuser";
        let alice_token = fake_jwt_for_user(alice_user);
        let alice_parties =
            get_parties(url.clone(), Some(&alice_token), Some("Alice".to_string())).await?;

        let scrooge_bank_user = "scroogebankuser";
        let scrooge_bank_token = fake_jwt_for_user(scrooge_bank_user);
        let scrooge_bank_parties = get_parties(
            url.clone(),
            Some(&scrooge_bank_token),
            Some("ScroogeBank".to_string()),
        )
        .await?;

        let ticketwizard_user = "ticketwizarduser";
        let ticketwizard_token = fake_jwt_for_user(ticketwizard_user);
        let ticketwizard_parties = get_parties(
            url.clone(),
            Some(&ticketwizard_token),
            Some("TicketWizard".to_string()),
        )
        .await?;

        let issuer = scrooge_bank_parties.first().cloned().unwrap();
        let owner = alice_parties.first().cloned().unwrap();
        let amount = 10.5_f64;
        let new_owner = ticketwizard_parties.first().cloned().unwrap();

        // Connect to ledger
        let channel = tonic::transport::Channel::from_shared(url)
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut command_service_client = CommandServiceClient::new(channel);

        // Create cash
        let create_result = create_cash(
            &mut command_service_client,
            Some(alice_token.as_str()),
            Some(alice_user),
            &package_id,
            issuer.clone(),
            owner.clone(),
            amount,
        )
        .await;

        assert!(
            create_result.is_ok(),
            "Cash creation failed: {:?}",
            create_result
        );

        let create_cash_result = create_result.unwrap();

        // Transfer cash
        let transfer_result = exercise_transfer(
            &mut command_service_client,
            Some(alice_token.as_str()),
            Some(alice_user),
            &package_id,
            create_cash_result.contract_id,
            new_owner.clone(),
            owner.clone(),
        )
        .await;

        assert!(
            transfer_result.is_ok(),
            "Transfer cash failed: {:?}",
            transfer_result
        );

        Ok(())
    }
}
