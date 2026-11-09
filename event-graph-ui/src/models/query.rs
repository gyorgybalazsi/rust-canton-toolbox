use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueryRequest {
    pub cypher: String,
    pub params: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SchemaStats {
    pub node_counts: HashMap<String, u64>,
    pub rel_counts: HashMap<String, u64>,
}
