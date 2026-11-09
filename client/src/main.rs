use clap::{Parser, Subcommand};

use client::config;
use client::get_update::get_update_by_id;
use client::jwt::{fake_jwt, fake_jwt_for_user, keycloak_jwt};
use client::ledger_end::get_ledger_end;
use client::stream_updates::stream_updates;

use anyhow::Result;
use futures_util::StreamExt;
use ledger_api::v2::admin::user_management_service_client::UserManagementServiceClient;
use tracing::{debug, info};

#[derive(Parser, Debug)]
#[command(name = "app")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Get the ledger end
    GetLedgerEnd {
        #[arg(long)]
        url: String,
        #[arg(long)]
        access_token: String,
    },
    /// Create fake access token for Sandbox
    FakeAccessToken {
        #[arg(long)]
        url: String,
        #[arg(long)]
        party: String,
    },
    /// Stream ledger updates for a party
    StreamUpdates {
        #[arg(long)]
        url: String,
        #[arg(long)]
        access_token: String,
        #[arg(long)]
        party: String,
        #[arg(long)]
        begin_exclusive: i64,
        #[arg(long)]
        end_inclusive: Option<i64>,
    },
    /// Stream transactions for a party
    StreamTransactions {
        #[arg(long)]
        url: String,
        #[arg(long)]
        access_token: String,
        #[arg(long)]
        party: String,
        #[arg(long)]
        begin_exclusive: i64,
        #[arg(long)]
        end_inclusive: Option<i64>,
    },
    /// Get parties, optionally filtered by a substring
    Parties {
        #[arg(long)]
        url: String,
        #[arg(long)]
        access_token: String,
        #[arg(long)]
        filter: Option<String>,
    },
    /// Look up a single update (transaction) by its ID
    GetUpdateById {
        /// Path to config.toml file
        #[arg(long)]
        config_file: Option<String>,
        /// Profile to use (overrides active_profile in config)
        #[arg(long, short)]
        profile: Option<String>,
        /// The update ID to look up
        #[arg(long)]
        update_id: String,
        /// Party to filter events for (overrides parties from config)
        #[arg(long)]
        party: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();

    match cli.command {
        Commands::GetLedgerEnd { url, access_token } => {
            let ledger_end = get_ledger_end(&url, Some(&access_token)).await?;
            info!("Ledger end: {}", ledger_end);
            Ok(())
        }
        Commands::FakeAccessToken { url, party } => {
            let channel = tonic::transport::Channel::from_shared(url)?
                .connect()
                .await?;
            let mut user_management_client = UserManagementServiceClient::new(channel);
            let token = fake_jwt(&mut user_management_client, &party).await?;
            info!("Fake access token: {}", token);
            Ok(())
        }
        Commands::StreamUpdates {
            access_token,
            party,
            url,
            begin_exclusive,
            end_inclusive,
        } => {
            info!(
                "StreamUpdates called with begin_exclusive: {}, end_inclusive: {:?}, party: {:?}, url: {}",
                begin_exclusive, end_inclusive, party, url
            );
            let mut stream = stream_updates(
                Some(&access_token),
                begin_exclusive,
                end_inclusive,
                vec![party],
                url,
                None, // No template filters
            )
            .await?;
            while let Some(update) = stream.next().await {
                info!("{:#?}", update);
            }
            Ok(())
        }
        Commands::StreamTransactions {
            access_token,
            party,
            url,
            begin_exclusive,
            end_inclusive,
        } => {
            info!(
                "StreamTransactions called with begin_exclusive: {}, end_inclusive: {:?}, party: {:?}, url: {}",
                begin_exclusive, end_inclusive, party, url
            );
            let mut stream = stream_updates(
                Some(&access_token),
                begin_exclusive,
                end_inclusive,
                vec![party],
                url,
                None, // No template filters
            )
            .await?;
            while let Some(Ok(response)) = stream.next().await {
                if let ledger_api::v2::get_updates_response::Update::Transaction(tx) =
                    &response.update.unwrap()
                {
                    info!("Transaction events: {:#?}", tx.events);
                    debug!(
                        "Structure markers: {:#?}",
                        client::utils::structure_markers_from_transaction(tx)
                    );
                }
            }
            Ok(())
        }
        Commands::Parties {
            filter,
            url,
            access_token,
        } => {
            let parties = client::party_management::get_parties::get_parties(
                url,
                Some(&access_token),
                filter,
            )
            .await?;
            if parties.is_empty() {
                info!("No parties found.");
            } else {
                info!("Known parties: {:?}", parties);
            }
            Ok(())
        }
        Commands::GetUpdateById {
            config_file,
            profile,
            update_id,
            party,
        } => {
            let cfg = match config_file {
                Some(path) => config::read_config(&path, profile.as_deref())?,
                None => config::read_config_from_toml(profile.as_deref())?,
            };

            // Determine parties
            let parties = match party {
                Some(p) => vec![p],
                None => cfg
                    .ledger
                    .parties
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No party specified. Use --party or set parties in config profile."
                        )
                    })?,
            };

            // Get access token
            let token = match cfg.keycloak {
                Some(kc_config) => {
                    info!(
                        "Authenticating via Keycloak at {}",
                        kc_config.token_endpoint
                    );
                    keycloak_jwt(&kc_config).await?
                }
                None => {
                    info!(
                        "No Keycloak config, using fake JWT for user: {}",
                        cfg.ledger.fake_jwt_user
                    );
                    fake_jwt_for_user(&cfg.ledger.fake_jwt_user)
                }
            };

            let response = get_update_by_id(&cfg.ledger.url, &token, &parties, &update_id).await?;

            println!("{}", serde_json::to_string_pretty(&response)?);
            Ok(())
        }
    }
}
