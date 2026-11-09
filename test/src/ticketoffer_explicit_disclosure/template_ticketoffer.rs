use anyhow::{Result, anyhow};
use client::submit_commands::CommandResult;
use client::submit_commands::submit_commands;
use daml_type_rep::built_in_types::{DamlContractId, DamlDecimal, DamlParty};
use daml_type_rep::lapi_access::LapiAccess;
use daml_type_rep::lapi_access::ToCreateArguments;
use daml_type_rep::template_id::TemplateId;
use derive_lapi_access::{LapiAccess, ToCreateArguments};
use ledger_api::v2::Record;
use ledger_api::v2::{
    Command, Commands, CreateCommand, DisclosedContract,
    command_service_client::CommandServiceClient,
};

#[derive(Clone, Debug, serde::Serialize, ToCreateArguments)]
pub struct TicketOffer {
    pub organizer: DamlParty,
    pub buyer: DamlParty,
    pub price: DamlDecimal,
}

impl TicketOffer {
    pub fn new(organizer: DamlParty, buyer: DamlParty, price: DamlDecimal) -> Self {
        TicketOffer {
            organizer,
            buyer,
            price,
        }
    }
}

#[derive(serde::Serialize, LapiAccess)]
/// Represents the Accept choice on the TicketOffer template.
pub struct Accept {
    pub cash_id: DamlContractId,
}

impl Accept {
    pub fn new(cash_id: String) -> Self {
        Accept {
            cash_id: DamlContractId::new(&cash_id),
        }
    }
}

pub async fn create_ticketoffer(
    command_service_client: &mut CommandServiceClient<tonic::transport::Channel>,
    access_token: Option<&str>,
    user_id: Option<&str>,
    package_id: &str,
    organizer: String,
    buyer: String,
    price: f64,
) -> Result<CreateTicketOfferResult> {
    let create_ticketoffer_command = CreateCommand {
        template_id: Some(TemplateId::new(package_id, "Main", "TicketOffer").to_template_id()),
        create_arguments: Some(
            TicketOffer::new(
                DamlParty::new(&organizer),
                DamlParty::new(&buyer),
                DamlDecimal::new(price),
            )
            .to_create_arguments(),
        ),
    };

    let commands = Commands {
        act_as: vec![organizer.clone()],
        user_id: user_id.unwrap_or("").to_string(),
        commands: vec![Command {
            command: Some(ledger_api::v2::command::Command::Create(
                create_ticketoffer_command,
            )),
        }],
        command_id: format!("command-{}", uuid::Uuid::new_v4()),
        ..Default::default()
    };

    let result = submit_commands(command_service_client, access_token, commands, None).await?;
    if let Some(CommandResult::Created {
        contract_id,
        create_argument_blob,
    }) = result.first()
    {
        Ok(CreateTicketOfferResult {
            contract_id: contract_id.clone(),
            created_event_blob: create_argument_blob.clone(),
        })
    } else {
        Err(anyhow!("No contract id found in create_ticketoffer result"))
    }
}

/// Result of creating a ticket offer, including the blob for explicit disclosure
#[derive(Debug)]
pub struct CreateTicketOfferResult {
    pub contract_id: String,
    pub created_event_blob: Option<Vec<u8>>,
}

pub async fn exercise_accept(
    command_service_client: &mut CommandServiceClient<tonic::transport::Channel>,
    access_token: Option<&str>,
    user_id: Option<&str>,
    package_id: &str,
    contract_id: String,
    cash_id: String,
    buyer: String,
    disclosed_contracts: Option<Vec<DisclosedContract>>,
) -> Result<()> {
    let exercise_command = ledger_api::v2::ExerciseCommand {
        template_id: Some(TemplateId::new(package_id, "Main", "TicketOffer").to_template_id()),
        contract_id,
        choice: "Accept".to_string(),
        choice_argument: Some(Accept::new(cash_id).to_lapi_value()),
        ..Default::default()
    };

    let commands = Commands {
        act_as: vec![buyer],
        user_id: user_id.unwrap_or("").to_string(),
        commands: vec![Command {
            command: Some(ledger_api::v2::command::Command::Exercise(exercise_command)),
        }],
        command_id: format!("command-{}", uuid::Uuid::new_v4()),
        ..Default::default()
    };

    submit_commands(
        command_service_client,
        access_token,
        commands,
        disclosed_contracts,
    )
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticketoffer_explicit_disclosure::template_cash::create_cash;
    use client::jwt::fake_jwt_for_user;
    use client::party_management::get_parties::get_parties;
    use client::run_script::run_script;
    use client::testutils::start_sandbox;
    use tokio;
    use tracing::info;

    #[tokio::test]
    async fn test_create_and_accept_ticketoffer() -> Result<()> {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;

        let log_file = std::fs::File::create("test_create_and_accept_ticketoffer.log")
            .expect("Failed to create log file");
        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(log_file)
            .with_ansi(false);
        let stdout_layer = tracing_subscriber::fmt::layer().with_test_writer();

        let _ = tracing_subscriber::registry()
            .with(file_layer)
            .with(stdout_layer)
            .try_init();

        let sandbox_port = 6865;
        let url = format!("http://localhost:{}", sandbox_port);
        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        // Use PathBuf for package_root
        let package_root = std::path::PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-ticketoffer-explicit-disclosure");

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
        let organizer = ticketwizard_parties.first().cloned().unwrap();
        let buyer = alice_parties.first().cloned().unwrap();

        // Connect to ledger
        let channel = tonic::transport::Channel::from_shared(url.clone())
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut command_service_client = CommandServiceClient::new(channel);

        // Create ticket offer
        let create_ticketoffer_result = create_ticketoffer(
            &mut command_service_client,
            Some(ticketwizard_token.as_str()),
            Some(alice_user),
            &package_id,
            organizer.clone(),
            buyer.clone(),
            amount,
        )
        .await;

        assert!(
            create_ticketoffer_result.is_ok(),
            "Ticketoffer creation failed: {:?}",
            create_ticketoffer_result
        );

        let ticketoffer_result = create_ticketoffer_result.unwrap();
        let ticketoffer_contract_id = ticketoffer_result.contract_id;
        let ticketoffer_blob = ticketoffer_result.created_event_blob;

        // Test that we can retrieve the blob using get_blobs_by_template
        {
            use client::get_blob::get_blobs_by_template;
            use client::ledger_end::get_ledger_end;
            use ledger_api::v2::Identifier;

            let ledger_end = get_ledger_end(&url, Some(&ticketwizard_token)).await?;
            info!("Ledger end offset: {}", ledger_end);

            let template_id = Identifier {
                package_id: package_id.clone(),
                module_name: "Main".to_string(),
                entity_name: "TicketOffer".to_string(),
            };

            let blobs = get_blobs_by_template(
                &url,
                Some(&ticketwizard_token),
                vec![organizer.clone()],
                template_id,
                ledger_end,
            )
            .await?;

            info!("Retrieved {} blobs from get_blobs_by_template", blobs.len());
            assert!(
                !blobs.is_empty(),
                "Expected to find at least one TicketOffer blob"
            );

            let retrieved_blob = blobs.get(&ticketoffer_contract_id);
            assert!(
                retrieved_blob.is_some(),
                "Expected to find blob for contract {}",
                ticketoffer_contract_id
            );

            let retrieved_blob = retrieved_blob.unwrap();
            assert!(
                !retrieved_blob.created_event_blob.is_empty(),
                "Retrieved blob should not be empty"
            );

            // If we have the original blob from create, verify they match
            if let Some(ref original_blob) = ticketoffer_blob {
                assert_eq!(
                    &retrieved_blob.created_event_blob, original_blob,
                    "Retrieved blob should match the blob from create command"
                );
                info!(
                    "Blob verification successful: retrieved blob matches original ({} bytes)",
                    original_blob.len()
                );
            }
        }

        info!(
            "TicketOffer created with id: {}, blob present: {}",
            ticketoffer_contract_id,
            ticketoffer_blob.is_some()
        );

        let create_cash_result = create_cash(
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
            create_cash_result.is_ok(),
            "Cash creation failed: {:?}",
            create_cash_result
        );

        let cash_result = create_cash_result.unwrap();
        let cash_contract_id = cash_result.contract_id;

        info!("Cash contract created with id: {}", cash_contract_id);

        // Build disclosed contracts for explicit disclosure
        // The TicketOffer contract needs to be disclosed to Alice (buyer)
        // because she is not a stakeholder/observer on the contract created by TicketWizard
        let disclosed_contracts = if let Some(blob) = ticketoffer_blob {
            info!(
                "Using explicit disclosure with TicketOffer contract blob ({} bytes)",
                blob.len()
            );
            Some(vec![DisclosedContract {
                template_id: None,
                contract_id: String::new(),
                created_event_blob: blob,
                synchronizer_id: String::new(),
            }])
        } else {
            info!("No blob available, proceeding without explicit disclosure");
            None
        };

        // Accept ticket offer with explicit disclosure of the TicketOffer contract
        let accept_result = exercise_accept(
            &mut command_service_client,
            Some(alice_token.as_str()),
            Some(alice_user),
            &package_id,
            ticketoffer_contract_id,
            cash_contract_id,
            buyer.clone(),
            disclosed_contracts,
        )
        .await;

        assert!(
            accept_result.is_ok(),
            "Accept ticket offer failed: {:?}",
            accept_result
        );

        info!("Successfully accepted ticket offer with explicit disclosure!");

        Ok(())
    }
}
