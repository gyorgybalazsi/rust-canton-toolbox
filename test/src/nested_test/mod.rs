pub mod generated {
    include!(concat!(env!("OUT_DIR"), "/nested_test_structs.rs"));
}

pub use generated::daml_nested_test::main::Registry;
pub use generated::daml_nested_test::main::UpdatePerson;
pub use generated::daml_nested_test::main::types::Person;
pub use generated::daml_nested_test::main::types::address::Address;

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use client::jwt::fake_jwt_for_user;
    use client::party_management::allocate_parties::allocate_parties;
    use client::submit_commands::{CommandResult, submit_commands};
    use client::testutils::start_sandbox;
    use client::upload_dar::upload_dars;
    use daml_type_rep::built_in_types::*;
    use daml_type_rep::lapi_access::{LapiAccess, ToCreateArguments};
    use daml_type_rep::template_id::TemplateId;
    use ledger_api::v2::{
        Command, Commands, CreateCommand, ExerciseCommand,
        command_service_client::CommandServiceClient,
    };
    use tracing::info;
    use tracing_subscriber::EnvFilter;

    #[tokio::test]
    async fn test_nested_test_create_and_exercise() -> Result<()> {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("info"))
            .pretty()
            .try_init()
            .ok();

        let sandbox_port = 6865;
        let url = format!("http://localhost:{}", sandbox_port);
        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let package_root = std::path::PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-nested-test")
            .canonicalize()
            .expect("Failed to canonicalize package_root");

        let dar_path = package_root
            .join("main")
            .join(".daml")
            .join("dist")
            .join("daml-nested-test-0.0.1.dar");

        info!("Starting sandbox with DAR: {}", dar_path.display());
        let _guard = start_sandbox(package_root, dar_path.clone(), sandbox_port).await?;

        // Upload DAR via Admin API (required for package vetting in Canton 3.x)
        let ledger_api = std::path::PathBuf::from(url.clone());
        upload_dars(&ledger_api, &[dar_path]).await?;

        // Setup auth
        let user_id = "admin_user";
        let token = fake_jwt_for_user(user_id);

        // Allocate party
        let parties =
            allocate_parties(url.clone(), Some(&token), vec!["Admin".to_string()]).await?;
        let admin_party = parties.first().cloned().unwrap();
        info!("Allocated party: {}", admin_party);

        // Connect to ledger
        let channel = tonic::transport::Channel::from_shared(url)
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut client = CommandServiceClient::new(channel);

        let package_id = "#daml-nested-test".to_string();

        // --- Create a Registry contract ---
        let registry = Registry {
            admin: DamlParty::new(&admin_party),
            person: Person {
                name: DamlText::new("Alice"),
                age: DamlInt::new(30),
                home_address: Address {
                    street: DamlText::new("123 Main St"),
                    city: DamlText::new("Zurich"),
                    zip: DamlText::new("8001"),
                },
            },
        };

        let create_command = CreateCommand {
            template_id: Some(TemplateId::new(&package_id, "Main", "Registry").to_template_id()),
            create_arguments: Some(registry.to_create_arguments()),
        };

        let commands = Commands {
            act_as: vec![admin_party.clone()],
            commands: vec![Command {
                command: Some(ledger_api::v2::command::Command::Create(create_command)),
            }],
            user_id: user_id.to_string(),
            command_id: format!("cmd-create-{}", uuid::Uuid::new_v4()),
            ..Default::default()
        };

        let result = submit_commands(&mut client, Some(&token), commands, None).await?;
        let contract_id = match result.first() {
            Some(CommandResult::Created { contract_id, .. }) => {
                info!("Created Registry contract: {}", contract_id);
                contract_id.clone()
            }
            other => {
                return Err(anyhow::anyhow!("Expected Created result, got: {:?}", other));
            }
        };

        // --- Exercise UpdatePerson choice ---
        let update_person = UpdatePerson {
            new_person: Person {
                name: DamlText::new("Bob"),
                age: DamlInt::new(25),
                home_address: Address {
                    street: DamlText::new("456 Oak Ave"),
                    city: DamlText::new("Geneva"),
                    zip: DamlText::new("1200"),
                },
            },
        };

        let exercise_command = ExerciseCommand {
            template_id: Some(TemplateId::new(&package_id, "Main", "Registry").to_template_id()),
            contract_id: contract_id.clone(),
            choice: "UpdatePerson".to_string(),
            choice_argument: Some(update_person.to_lapi_value()),
            ..Default::default()
        };

        let commands = Commands {
            act_as: vec![admin_party.clone()],
            commands: vec![Command {
                command: Some(ledger_api::v2::command::Command::Exercise(exercise_command)),
            }],
            user_id: user_id.to_string(),
            command_id: format!("cmd-exercise-{}", uuid::Uuid::new_v4()),
            ..Default::default()
        };

        let result = submit_commands(&mut client, Some(&token), commands, None).await?;
        info!("Exercise result: {:?}", result);

        // Verify we got a new contract back
        let new_contract_found = result
            .iter()
            .any(|r| matches!(r, CommandResult::Created { .. }));
        assert!(
            new_contract_found,
            "UpdatePerson should create a new Registry contract"
        );

        info!(
            "Integration test passed: created Registry with nested Person/Address, exercised UpdatePerson"
        );
        Ok(())
    }
}
