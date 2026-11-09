use crate::utils::build_filters_by_party_with_identifiers;
use anyhow::{Context, Result};
use ledger_api::v2::{
    EventFormat, GetUpdatesRequest, Identifier, TransactionFormat, TransactionShape, UpdateFormat,
    update_service_client::UpdateServiceClient,
};
use tonic::metadata::MetadataValue;
use tracing::{debug, error, info};

/// Streams ledger updates for the given parties, starting after `begin_exclusive` offset.
/// `end_inclusive` is optional; if set, the stream will end at that offset.
/// `template_filters` is optional; if provided, only events matching those templates are streamed.
pub async fn stream_updates(
    access_token: Option<&str>,
    begin_exclusive: i64,
    end_inclusive: Option<i64>,
    parties: Vec<String>,
    url: String,
    template_filters: Option<&[Identifier]>,
) -> Result<tonic::Streaming<ledger_api::v2::GetUpdatesResponse>> {
    info!(
        "Starting stream_updates at {}:{}: url={}, begin_exclusive={}, end_inclusive={:?}, parties={:?}, template_filters={:?}",
        file!(),
        line!(),
        url,
        begin_exclusive,
        end_inclusive,
        parties,
        template_filters
    );

    debug!(
        "Connecting to update service at {}:{}: {}",
        file!(),
        line!(),
        url
    );
    let mut client = match UpdateServiceClient::connect(url.clone())
        .await
        .map(|c| c.max_decoding_message_size(64 * 1024 * 1024))
    {
        Ok(c) => {
            debug!(
                "Successfully connected to update service at {}:{}",
                file!(),
                line!()
            );
            c
        }
        Err(e) => {
            error!(
                "Failed to connect to update service at {}:{}: {:?}",
                file!(),
                line!(),
                e
            );
            return Err(anyhow::anyhow!(
                "Failed to connect to update service at {}: {}",
                url,
                e
            ));
        }
    };

    let filters_by_party = build_filters_by_party_with_identifiers(&parties, template_filters);
    debug!(
        "Built filters_by_party at {}:{}: {:?}",
        file!(),
        line!(),
        filters_by_party
    );

    let event_format = EventFormat {
        filters_by_party,
        filters_for_any_party: None,
        verbose: true,
    };

    let transaction_format = TransactionFormat {
        event_format: Some(event_format),
        transaction_shape: TransactionShape::LedgerEffects as i32,
    };

    let update_format = UpdateFormat {
        include_transactions: Some(transaction_format),
        include_reassignments: None,
        include_topology_events: None,
    };

    let request = GetUpdatesRequest {
        begin_exclusive,
        end_inclusive,
        update_format: Some(update_format),
        ..Default::default()
    };
    debug!(
        "Created GetUpdatesRequest at {}:{}: {:?}",
        file!(),
        line!(),
        request
    );

    let mut req = tonic::Request::new(request);
    if let Some(token) = access_token {
        debug!(
            "Adding authorization token to request at {}:{}",
            file!(),
            line!()
        );
        let meta = MetadataValue::try_from(format!("Bearer {}", token))
            .with_context(|| "Failed to parse access token for metadata")?;
        req.metadata_mut().insert("authorization", meta);
    } else {
        debug!("No access token provided at {}:{}", file!(), line!());
    }

    debug!("Sending get_updates request at {}:{}", file!(), line!());
    let response = match client.get_updates(req).await {
        Ok(resp) => {
            info!(
                "Successfully initiated updates stream at {}:{}",
                file!(),
                line!()
            );
            resp
        }
        Err(e) => {
            error!(
                "Failed to get updates from ledger at {}:{}: {:?}",
                file!(),
                line!(),
                e
            );
            return Err(anyhow::anyhow!("Failed to get updates from ledger: {}", e));
        }
    };

    Ok(response.into_inner())
}
