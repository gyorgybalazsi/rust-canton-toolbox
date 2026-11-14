use canton_event_graph_sync::config;
use canton_event_graph_sync::model;
use canton_event_graph_sync::sync::{BackoffConfig, SyncConfig, run_resilient_sync};
use clap::{Parser, Subcommand};
use client::jwt::TokenSource;
use client::stream_updates::stream_updates;
use ledger_api::v2::Identifier;
use std::time::Instant;
use tokio_stream::StreamExt;
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "canton-event-graph-sync")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print the SurrealQL writes generated from the update stream (no database needed)
    PrintSurql {
        #[arg(long)]
        access_token: String,
        #[arg(long)]
        party: String,
        #[arg(long)]
        url: String,
        #[arg(long)]
        begin_exclusive: i64,
        #[arg(long)]
        end_inclusive: Option<i64>,
        /// Optional template filter package names (e.g., "#splice-amulet")
        #[arg(long)]
        template_package: Vec<String>,
        /// Optional template filter module names (e.g., "Splice.Amulet")
        #[arg(long)]
        template_module: Vec<String>,
        /// Optional template filter entity names (e.g., "FeaturedAppActivityMarker")
        #[arg(long)]
        template_entity: Vec<String>,
    },
    /// Benchmark raw Canton stream throughput (no database writes)
    Benchmark {
        /// Path to config.toml file
        #[arg(long)]
        config_file: Option<String>,
        /// Profile to use (overrides active_profile in config)
        #[arg(long, short)]
        profile: Option<String>,
        /// Use Keycloak to obtain a real JWT token
        #[arg(long)]
        use_keycloak: bool,
        /// Number of updates to process (default: 10000)
        #[arg(long, default_value = "10000")]
        count: u64,
        /// Starting offset (if not specified, uses pruning offset from ledger)
        #[arg(long)]
        begin_offset: Option<i64>,
    },
    Sync {
        /// Path to config.toml file (defaults to ./config/config.toml or CARGO_MANIFEST_DIR/config/config.toml)
        #[arg(long)]
        config_file: Option<String>,
        /// Profile to use (overrides active_profile in config)
        #[arg(long, short)]
        profile: Option<String>,
        /// Optional access token (if not provided, will try Keycloak config, then fall back to fake JWT)
        #[arg(long)]
        access_token: Option<String>,
        /// Use Keycloak to obtain a real JWT token (requires keycloak section in profile)
        #[arg(long)]
        use_keycloak: bool,
        /// Fresh start: delete the indexer data in SurrealDB, load current ACS, and stream from ledger end
        #[arg(long)]
        fresh: bool,
        /// Start a fresh sync from the offset that was current N days ago
        /// (resolved by binary search over the Ledger API). Implies --fresh.
        #[arg(long)]
        from_days_ago: Option<u32>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Determine log level from config file if available (for Sync command), otherwise default to INFO
    let log_level = match &cli.command {
        Commands::Sync {
            config_file,
            profile,
            ..
        } => {
            let config = match config_file {
                Some(path) => {
                    canton_event_graph_sync::config::read_config(path, profile.as_deref()).ok()
                }
                None => {
                    canton_event_graph_sync::config::read_config_from_toml(profile.as_deref()).ok()
                }
            };
            config
                .map(|c| c.logging.level)
                .unwrap_or_else(|| "info".to_string())
        }
        _ => "info".to_string(),
    };

    // Initialize tracing subscriber with env filter (RUST_LOG takes precedence over config)
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&log_level));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    match cli.command {
        Commands::PrintSurql {
            access_token,
            url,
            begin_exclusive,
            end_inclusive,
            party,
            template_package,
            template_module,
            template_entity,
        } => {
            let parties = vec![party];
            // Build Identifier list from parallel CLI args
            let template_identifiers: Vec<Identifier> = template_package
                .into_iter()
                .zip(template_module.into_iter())
                .zip(template_entity.into_iter())
                .map(|((package, module), entity)| Identifier {
                    package_id: package,
                    module_name: module,
                    entity_name: entity,
                })
                .collect();
            let template_filters: Option<&[Identifier]> = if template_identifiers.is_empty() {
                None
            } else {
                Some(&template_identifiers)
            };
            let mut update_stream = stream_updates(
                Some(&access_token),
                begin_exclusive,
                end_inclusive,
                parties,
                url,
                template_filters,
            )
            .await?;
            while let Some(response) = update_stream.next().await {
                let batch = model::update_to_batch(&response?);
                if batch.is_empty() {
                    continue;
                }
                println!("-- offset {:?}", batch.max_offset);
                print!("{}", batch.to_surql_preview());
            }
        }
        Commands::Benchmark {
            config_file,
            profile,
            use_keycloak,
            count,
            begin_offset,
        } => {
            info!("Starting Canton stream benchmark (no database writes)");

            let config = match config_file {
                Some(path) => {
                    canton_event_graph_sync::config::read_config(&path, profile.as_deref())
                        .expect("failed to read config")
                }
                None => canton_event_graph_sync::config::read_config_from_toml(profile.as_deref())
                    .expect("failed to read config"),
            };
            let parties = config.ledger.parties.unwrap_or_default();
            let ledger_url = config.ledger.url;
            let template_filters = config.ledger.template_filters;
            let template_identifiers: Vec<Identifier> = template_filters
                .unwrap_or_default()
                .iter()
                .map(|f| Identifier {
                    package_id: f.package_name.clone(),
                    module_name: f.module_name.clone(),
                    entity_name: f.entity_name.clone(),
                })
                .collect();
            let template_filter_arg: Option<&[Identifier]> = if template_identifiers.is_empty() {
                None
            } else {
                Some(&template_identifiers)
            };

            // Get token
            let token = if use_keycloak {
                let kc_config = config
                    .keycloak
                    .expect("--use-keycloak requires keycloak section in profile");
                let auth_method = match kc_config.auth_method {
                    config::KeycloakAuthMethod::ClientCredentials { client_secret } => {
                        client::jwt::KeycloakAuthMethod::ClientCredentials { client_secret }
                    }
                    config::KeycloakAuthMethod::Password {
                        username,
                        password,
                        client_secret,
                    } => client::jwt::KeycloakAuthMethod::Password {
                        username,
                        password,
                        client_secret,
                    },
                };
                let kc = client::jwt::KeycloakConfig {
                    client_id: kc_config.client_id,
                    token_endpoint: kc_config.token_endpoint,
                    auth_method,
                };
                client::jwt::keycloak_jwt(&kc).await?
            } else {
                client::jwt::fake_jwt_for_user(&config.ledger.fake_jwt_user)
            };

            // Determine start offset
            let start_offset = match begin_offset {
                Some(o) => o,
                None => {
                    info!("Querying pruning offset from ledger...");
                    client::ledger_end::get_pruning_offset(&ledger_url, Some(&token)).await?
                }
            };

            info!(
                "Benchmark config: start_offset={}, count={}, parties={:?}",
                start_offset, count, parties
            );
            info!(
                "Streaming {} updates from Canton (stream only, no conversion, no database)...",
                count
            );

            // Benchmark 1: Raw stream only
            let mut update_stream = stream_updates(
                Some(&token),
                start_offset,
                None,
                parties.clone(),
                ledger_url.clone(),
                template_filter_arg,
            )
            .await?;
            let start_time = Instant::now();
            let mut raw_count = 0u64;
            let mut last_offset = start_offset;

            while let Some(response) = update_stream.next().await {
                match response {
                    Ok(resp) => {
                        if let Some(update) = &resp.update {
                            last_offset = match update {
                                ledger_api::v2::get_updates_response::Update::Transaction(tx) => tx.offset,
                                ledger_api::v2::get_updates_response::Update::Reassignment(r) => r.offset,
                                ledger_api::v2::get_updates_response::Update::OffsetCheckpoint(c) => c.offset,
                                ledger_api::v2::get_updates_response::Update::TopologyTransaction(t) => t.offset,
                            };
                        }
                        raw_count += 1;
                        if raw_count >= count {
                            break;
                        }
                        if raw_count.is_multiple_of(1000) {
                            let elapsed = start_time.elapsed().as_secs_f64();
                            info!(
                                "[Raw Stream] {} updates, {:.1} updates/s, offset {}",
                                raw_count,
                                raw_count as f64 / elapsed,
                                last_offset
                            );
                        }
                    }
                    Err(e) => {
                        warn!("Stream error: {}", e);
                        break;
                    }
                }
            }
            let raw_elapsed = start_time.elapsed();
            let raw_rate = raw_count as f64 / raw_elapsed.as_secs_f64();
            info!("=== RAW STREAM BENCHMARK ===");
            info!("  Updates: {}", raw_count);
            info!("  Time: {:.2}s", raw_elapsed.as_secs_f64());
            info!("  Rate: {:.1} updates/s", raw_rate);
            info!("  Offset range: {} -> {}", start_offset, last_offset);

            // Benchmark 2: Stream + conversion to SurrealDB rows
            info!(
                "\nStreaming {} updates with conversion to SurrealDB rows (no database)...",
                count
            );
            let mut update_stream = stream_updates(
                Some(&token),
                start_offset,
                None,
                parties.clone(),
                ledger_url.clone(),
                template_filter_arg,
            )
            .await?;
            let start_time = Instant::now();
            let mut convert_count = 0u64;
            let mut total_rows = 0usize;

            while let Some(response) = update_stream.next().await {
                match response {
                    Ok(resp) => {
                        total_rows += model::update_to_batch(&resp).row_count();
                        convert_count += 1;
                        if convert_count >= count {
                            break;
                        }
                        if convert_count.is_multiple_of(1000) {
                            let elapsed = start_time.elapsed().as_secs_f64();
                            info!(
                                "[Stream+Convert] {} updates, {:.1} updates/s",
                                convert_count,
                                convert_count as f64 / elapsed
                            );
                        }
                    }
                    Err(e) => {
                        warn!("Stream error: {}", e);
                        break;
                    }
                }
            }
            let convert_elapsed = start_time.elapsed();
            let convert_rate = convert_count as f64 / convert_elapsed.as_secs_f64();
            info!("=== STREAM + CONVERT BENCHMARK ===");
            info!("  Updates: {}", convert_count);
            info!("  Rows generated: {}", total_rows);
            info!("  Time: {:.2}s", convert_elapsed.as_secs_f64());
            info!("  Rate: {:.1} updates/s", convert_rate);

            // Summary
            info!("\n=== SUMMARY ===");
            info!("  Raw stream:      {:.1} updates/s", raw_rate);
            info!("  Stream + Convert: {:.1} updates/s", convert_rate);
            info!(
                "  Conversion overhead: {:.1}%",
                (1.0 - convert_rate / raw_rate) * 100.0
            );
            info!(
                "  (Compare with canton-event-graph-sync sync rate to see database write overhead)"
            );
        }
        Commands::Sync {
            config_file,
            profile,
            access_token,
            use_keycloak,
            fresh,
            from_days_ago,
        } => {
            // --from-days-ago anchors a fresh start at a past offset, so it implies --fresh.
            let fresh = fresh || from_days_ago.is_some();
            info!(
                "Starting resilient sync command (fresh={}, from_days_ago={:?})",
                fresh, from_days_ago
            );

            debug!(config_path = ?config_file, profile = ?profile, "Reading configuration from TOML file");
            let config = match config_file {
                Some(path) => {
                    canton_event_graph_sync::config::read_config(&path, profile.as_deref())
                        .expect("failed to read config from specified path")
                }
                None => canton_event_graph_sync::config::read_config_from_toml(profile.as_deref())
                    .expect("failed to read config from toml"),
            };
            let fake_jwt_user = config.ledger.fake_jwt_user;
            let configured_parties = config.ledger.parties.unwrap_or_default();
            let ledger_url = config.ledger.url;
            let template_filters = config.ledger.template_filters;
            let starting_offset = config.ledger.starting_offset;
            let db = config
                .surrealdb
                .resolve()
                .expect("incomplete SurrealDB settings");
            let keycloak_config = config.keycloak;

            info!(
                ledger_url = %ledger_url,
                surreal_url = %db.url,
                surreal_namespace = %db.namespace,
                surreal_database = %db.database,
                parties = ?configured_parties,
                template_filters = ?template_filters,
                starting_offset = ?starting_offset,
                "Configuration loaded"
            );

            // Determine token source for automatic renewal
            let token_source = match access_token {
                Some(token) => {
                    info!("Using provided static access token");
                    TokenSource::Static(token)
                }
                None if use_keycloak => {
                    let kc_config = keycloak_config
                        .expect("--use-keycloak requires keycloak section in profile");
                    info!(
                        "Using Keycloak for JWT token management at {}",
                        kc_config.token_endpoint
                    );

                    // Convert from canton-event-graph-sync's KeycloakConfig to client's KeycloakConfig
                    let auth_method = match kc_config.auth_method {
                        config::KeycloakAuthMethod::ClientCredentials { client_secret } => {
                            client::jwt::KeycloakAuthMethod::ClientCredentials { client_secret }
                        }
                        config::KeycloakAuthMethod::Password {
                            username,
                            password,
                            client_secret,
                        } => client::jwt::KeycloakAuthMethod::Password {
                            username,
                            password,
                            client_secret,
                        },
                    };

                    TokenSource::Keycloak(client::jwt::KeycloakConfig {
                        client_id: kc_config.client_id,
                        token_endpoint: kc_config.token_endpoint,
                        auth_method,
                    })
                }
                None => {
                    info!("Using fake JWT token for user: {}", fake_jwt_user);
                    TokenSource::FakeJwt(fake_jwt_user)
                }
            };

            // Auto-discover parties from ledger if none configured
            let parties = if configured_parties.is_empty() {
                info!("No parties configured, discovering from ledger...");
                let tm = client::jwt::TokenManager::new(token_source.clone());
                let discovery_token = tm.get_token().await?;
                let discovered: Vec<String> = client::party_management::get_parties::get_parties(
                    ledger_url.clone(),
                    Some(&discovery_token),
                    None,
                )
                .await?
                .into_iter()
                .filter(|p| !p.starts_with("sandbox::"))
                .collect();
                info!("Discovered {} parties: {:?}", discovered.len(), discovered);
                discovered
            } else {
                configured_parties
            };

            let sync_config = SyncConfig {
                ledger_url,
                parties,
                db,
                template_filters,
                starting_offset,
                batch_size: config.surrealdb.batch_size,
                flush_timeout_secs: config.surrealdb.flush_timeout_secs,
                idle_timeout_secs: config.surrealdb.idle_timeout_secs,
            };

            if fresh {
                info!(
                    "FRESH START: Will clear SurrealDB data, load current ACS, and stream from ledger end"
                );
            } else {
                info!(
                    "Starting resilient sync loop (will auto-reconnect on failures, resume from SurrealDB checkpoint)"
                );
            }
            run_resilient_sync(
                sync_config,
                token_source,
                BackoffConfig::default(),
                fresh,
                from_days_ago,
            )
            .await?;
        }
    }

    Ok(())
}
