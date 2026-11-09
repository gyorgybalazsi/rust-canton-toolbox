use crate::utils::build_filters_by_party;
use anyhow::Result;
use ledger_api::v2::Commands;
use ledger_api::v2::DisclosedContract;
use ledger_api::v2::EventFormat;
use ledger_api::v2::SubmitAndWaitForTransactionRequest;
use ledger_api::v2::TransactionFormat;
use ledger_api::v2::TransactionShape;
use ledger_api::v2::Value;
use ledger_api::v2::command_service_client::CommandServiceClient;
use ledger_api::v2::event::Event;
use tracing::{debug, error, info};

#[derive(Debug)]
pub enum CommandResult {
    Created {
        contract_id: String,
        create_argument_blob: Option<Vec<u8>>,
    },
    ExerciseResult(Value),
}

pub async fn submit_commands(
    command_service_client: &mut CommandServiceClient<tonic::transport::Channel>,
    access_token: Option<&str>,
    commands: Commands,
    disclosed_contracts: Option<Vec<DisclosedContract>>,
) -> Result<Vec<CommandResult>> {
    info!(
        "Submitting commands at {}:{}: act_as={:?}, command_id={:?}, command: {:#?}, disclosed_contracts: {:?}",
        file!(),
        line!(),
        commands.act_as,
        commands.command_id,
        commands.commands,
        disclosed_contracts.as_ref().map(|d| d.len())
    );

    let parties = commands.act_as.clone();

    // Apply disclosed contracts if provided
    let commands = if let Some(disclosed) = disclosed_contracts {
        Commands {
            disclosed_contracts: disclosed,
            ..commands
        }
    } else {
        commands
    };

    let filters_by_party = build_filters_by_party(&parties);

    let event_format = EventFormat {
        filters_by_party,
        filters_for_any_party: None,
        verbose: true,
    };

    let transaction_format = TransactionFormat {
        event_format: Some(event_format),
        transaction_shape: TransactionShape::LedgerEffects as i32,
    };

    let request = SubmitAndWaitForTransactionRequest {
        commands: Some(commands.clone()),
        transaction_format: Some(transaction_format),
    };

    let response = if let Some(token) = access_token {
        use tonic::Request;
        let mut req = Request::new(request);
        req.metadata_mut().insert(
            "authorization",
            format!("Bearer {}", token).parse().unwrap(),
        );
        match command_service_client
            .submit_and_wait_for_transaction(req)
            .await
        {
            Ok(resp) => resp.into_inner(),
            Err(e) => {
                error!("Error at {}:{} - {:?}", file!(), line!(), e);
                return Err(e.into());
            }
        }
    } else {
        match command_service_client
            .submit_and_wait_for_transaction(request)
            .await
        {
            Ok(resp) => resp.into_inner(),
            Err(e) => {
                error!("Error at {}:{} - {:?}", file!(), line!(), e);
                return Err(e.into());
            }
        }
    };

    let mut results = Vec::new();
    if let Some(tx) = &response.transaction {
        debug!("Transaction at {}:{}: {:#?}", file!(), line!(), tx);
        for event in &tx.events {
            match &event.event {
                Some(Event::Created(created_event)) => {
                    let blob = if created_event.created_event_blob.is_empty() {
                        None
                    } else {
                        Some(created_event.created_event_blob.clone())
                    };
                    results.push(CommandResult::Created {
                        contract_id: created_event.contract_id.clone(),
                        create_argument_blob: blob,
                    });
                }
                Some(Event::Exercised(exercised_event)) => {
                    if let Some(val) = &exercised_event.exercise_result {
                        results.push(CommandResult::ExerciseResult(val.clone()));
                    }
                }
                _ => {}
            }
        }
    } else {
        error!(
            "No transaction found in response at {}:{}",
            file!(),
            line!()
        );
    }
    info!(
        "Submit commands result at {}:{}: {:#?}",
        file!(),
        line!(),
        results
    );
    Ok(results)
}
