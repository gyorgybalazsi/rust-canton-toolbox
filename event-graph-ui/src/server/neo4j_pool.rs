use crate::config::Neo4jConfig;
use anyhow::Result;
use std::sync::OnceLock;

static NEO4J: OnceLock<neo4rs::Graph> = OnceLock::new();

pub fn init(config: &Neo4jConfig) -> Result<()> {
    let graph = neo4rs::Graph::new(&config.uri, &config.user, &config.password)?;
    NEO4J
        .set(graph)
        .map_err(|_| anyhow::anyhow!("Neo4j pool already initialized"))?;
    tracing::info!("Neo4j connection pool initialized ({})", config.uri);
    Ok(())
}

pub fn pool() -> &'static neo4rs::Graph {
    NEO4J.get().expect("Neo4j pool not initialized")
}
