use anyhow::{Result, anyhow};
use client::submit_commands::CommandResult;
use client::submit_commands::submit_commands;
use daml_type_rep::built_in_types::{DamlInt, DamlParty, DamlText};
use daml_type_rep::lapi_access::LapiAccess;
use daml_type_rep::lapi_access::ToCreateArguments;
use daml_type_rep::template_id::TemplateId;
use derive_lapi_access::LapiAccess;
use derive_lapi_access::ToCreateArguments;
use ledger_api::v2::Record;
use ledger_api::v2::{
    Command, Commands, CreateCommand, ExerciseCommand, command_service_client::CommandServiceClient,
};
use tracing::info;

#[derive(serde::Serialize, ToCreateArguments)]
pub struct IOU {
    issuer: DamlParty,
    owner: DamlParty,
    value: DamlInt,
    name: DamlText,
}

impl IOU {
    pub fn new(issuer: &String, owner: &String, value: i64, name: String) -> Self {
        Self {
            issuer: DamlParty::new(issuer),
            owner: DamlParty::new(owner),
            value: DamlInt::new(value),
            name: DamlText::new(name),
        }
    }
}

#[derive(serde::Serialize, LapiAccess)]
pub struct GetView {}

// TODO implement from_api_value
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct View {
    pub asset_owner: DamlParty,
    pub description: DamlText,
}

const MAIN_PACKAGE_ID: &str = "#daml-interface-example-main";
const ASSET_PACKAGE_ID: &str = "#daml-interface-example-interfaces";

pub async fn create_iou(
    command_service_client: &mut CommandServiceClient<tonic::transport::Channel>,
    access_token: Option<&str>,
    user_id: Option<&str>,
    issuer: String,
    owner: String,
    value: i64,
    name: String,
) -> Result<String> {
    // TODO the package name #interface-example-main wasn't accepted?
    let package_id = MAIN_PACKAGE_ID;
    let create_iou_command = CreateCommand {
        template_id: Some(TemplateId::new(package_id, "Main", "IOU").to_template_id()),
        create_arguments: Some(IOU::new(&issuer, &owner, value, name).to_create_arguments()),
    };

    let commands = Commands {
        act_as: vec![issuer.clone()],
        commands: vec![Command {
            command: Some(ledger_api::v2::command::Command::Create(create_iou_command)),
        }],
        user_id: user_id.unwrap_or("").to_string(),
        command_id: format!("command-{}", uuid::Uuid::new_v4()),
        ..Default::default()
    };

    let result = submit_commands(command_service_client, access_token, commands, None).await?;
    let contract_id = if let Some(CommandResult::Created { contract_id, .. }) = result.first() {
        contract_id.clone()
    } else {
        return Err(anyhow!("No contract id found in create_iou result"));
    };

    Ok(contract_id)
}

pub async fn exercise_getview(
    command_service_client: &mut CommandServiceClient<tonic::transport::Channel>,
    access_token: Option<&str>,
    owner: String,
    user_id: Option<&str>,
    contract_id: String,
) -> Result<()> {
    info!(
        "Called exercise_getview with owner: {}, user_id: {:?}, contract_id: {}",
        owner, user_id, contract_id
    );
    let package_id = ASSET_PACKAGE_ID;
    info!("Using package_id: {} for Asset interface", package_id);
    let exercise_getview_command = ExerciseCommand {
        // To exercise a choice on an interface, specify the interface identifier in the template_id field.
        // https://docs.digitalasset.com/build/3.3/reference/lapi-proto-docs.html#exercisecommand-message-version-com-daml-ledger-api-v2
        template_id: Some(TemplateId::new(package_id, "Asset", "Asset").to_template_id()),
        contract_id: contract_id.clone(),
        choice: "GetView".into(),
        choice_argument: Some(GetView {}.to_lapi_value()),
    };

    info!(
        "Constructed ExerciseCommand: contract_id={}, choice=GetView",
        contract_id
    );

    let commands = Commands {
        act_as: vec![owner.clone()],
        commands: vec![Command {
            command: Some(ledger_api::v2::command::Command::Exercise(
                exercise_getview_command,
            )),
        }],
        user_id: user_id.unwrap_or("").to_string(),
        command_id: format!("command-{}", uuid::Uuid::new_v4()),
        ..Default::default()
    };

    info!(
        "Submitting commands as act_as: {:?}, user_id: {:?}",
        commands.act_as, commands.user_id
    );
    let result = submit_commands(command_service_client, access_token, commands, None).await?;
    info!("Result contains {} elements", result.len());
    if result.is_empty() {
        info!("exercise_getview result is empty");
        return Err(anyhow!("exercise_getview result is empty"));
    }
    if let Some(CommandResult::ExerciseResult(value)) = result.first() {
        info!("view_result at {}:{}: {:#?}", file!(), line!(), value);
    } else {
        info!("No view result found in exercise_getview result");
        return Err(anyhow!("No view result found in exercise_getview result"));
    }
    info!("exercise_getview completed successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::jwt::fake_jwt_for_user;
    use client::party_management::get_parties::get_parties;
    use client::run_script::run_script;
    use client::testutils::start_sandbox;
    use client::upload_dar::upload_dars;
    use tokio;

    #[tokio::test]
    async fn test_create_iou_and_exercise_getview() -> anyhow::Result<()> {
        use tracing_subscriber::EnvFilter;
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("debug"))
            .with_test_writer()
            .try_init();
        tracing::info!("Logger initialized for test_create_iou_and_exercise_getview");

        let sandbox_port = 6865;
        let url = format!("http://localhost:{}", sandbox_port);
        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        info!("Crate root: {}", crate_root);

        // Construct package_root as PathBuf
        let package_root = std::path::PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-interface-example")
            .join("test")
            .canonicalize()
            .expect("Failed to canonicalize package_root");
        info!("Starting sandbox at {:?}", package_root);
        let dar_path = package_root
            .join(".daml")
            .join("dist")
            .join("daml-interface-example-test-1.0.0.dar");

        let _guard = start_sandbox(package_root, dar_path, sandbox_port).await?;

        upload_dars(
            &std::path::PathBuf::from(format!("http://localhost:{}", sandbox_port)),
            &[
                std::path::PathBuf::from(&crate_root)
                    .join("..")
                    .join("_daml")
                    .join("daml-interface-example")
                    .join("interfaces")
                    .join(".daml")
                    .join("dist")
                    .join("daml-interface-example-interfaces-1.0.0.dar"),
                std::path::PathBuf::from(&crate_root)
                    .join("..")
                    .join("_daml")
                    .join("daml-interface-example")
                    .join("main")
                    .join(".daml")
                    .join("dist")
                    .join("daml-interface-example-main-1.0.0.dar"),
            ],
        )
        .await?;

        // Allocate Alice and Bob by running the Test:test script (which
        // allocates parties as a side-effect before returning the view)
        let test_dar_path = std::path::PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-interface-example")
            .join("test")
            .join(".daml")
            .join("dist")
            .join("daml-interface-example-test-1.0.0.dar");
        let script_result = run_script("localhost", sandbox_port, &test_dar_path, "Test:test")?;
        info!("Setup script result: {}", script_result);

        // Setup test values
        let alice_user = "alice_user";
        let alice_token = fake_jwt_for_user(alice_user);
        let alice_parties =
            get_parties(url.clone(), Some(&alice_token), Some("Alice".to_string())).await?;
        let bob_parties =
            get_parties(url.clone(), Some(&alice_token), Some("Bob".to_string())).await?;
        let issuer = alice_parties
            .first()
            .cloned()
            .unwrap_or_else(|| "Alice".to_string());
        let owner = bob_parties
            .first()
            .cloned()
            .unwrap_or_else(|| "Bob".to_string());
        let value = 42;
        let name = "Test IOU".to_string();

        // Connect to ledger
        let channel = tonic::transport::Channel::from_shared(url)
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut command_service_client = CommandServiceClient::new(channel);

        // Create IOU
        let create_result = create_iou(
            &mut command_service_client,
            Some(alice_token.as_str()),
            Some(alice_user),
            issuer.clone(),
            owner.clone(),
            value,
            name.clone(),
        )
        .await;

        assert!(
            create_result.is_ok(),
            "IOU creation failed: {:?}",
            create_result
        );
        let contract_id = create_result.unwrap();

        // Exercise GetView
        let getview_result = exercise_getview(
            &mut command_service_client,
            Some(alice_token.as_str()),
            owner.clone(),
            Some(alice_user),
            contract_id,
        )
        .await;

        assert!(
            getview_result.is_ok(),
            "GetView exercise failed: {:?}",
            getview_result
        );

        Ok(())
    }
}
