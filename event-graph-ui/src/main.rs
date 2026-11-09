// WIP scaffolding (file-based config, saved-queries persistence, graph helpers)
// is defined but not yet wired up. Allow until those features are completed.
#![allow(dead_code)]

mod components;
mod config;
mod models;
mod server;
mod state;

use components::app::App;

fn main() {
    #[cfg(feature = "server")]
    {
        use tracing_subscriber::EnvFilter;
        tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .init();

        match config::find_and_read_config() {
            Ok(cfg) => {
                tracing::info!("Loaded config, connecting to Neo4j at {}", cfg.neo4j.uri);
                if let Err(e) = server::neo4j_pool::init(&cfg.neo4j) {
                    tracing::error!("Failed to initialize Neo4j pool: {e}");
                    std::process::exit(1);
                }
            }
            Err(e) => {
                tracing::warn!("Config not found ({e}), Neo4j queries will fail");
            }
        }
    }

    dioxus::launch(App);
}
