use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub label: NodeLabel,
    pub display_name: String,
    pub properties: HashMap<String, serde_json::Value>,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NodeLabel {
    Transaction,
    Created,
    Exercised,
    Party,
}

impl NodeLabel {
    pub fn color(&self) -> &'static str {
        match self {
            NodeLabel::Transaction => "#4A90D9",
            NodeLabel::Created => "#50C878",
            NodeLabel::Exercised => "#F5A623",
            NodeLabel::Party => "#9B59B6",
        }
    }

    pub fn display(&self) -> &'static str {
        match self {
            NodeLabel::Transaction => "Transaction",
            NodeLabel::Created => "Created",
            NodeLabel::Exercised => "Exercised",
            NodeLabel::Party => "Party",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub rel_type: RelType,
    pub properties: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelType {
    Action,
    Consequence,
    Target,
    Consumes,
    Requested,
}

impl RelType {
    pub fn stroke_color(&self) -> &'static str {
        match self {
            RelType::Action => "#555555",
            RelType::Consequence => "#888888",
            RelType::Target => "#4A90D9",
            RelType::Consumes => "#E74C3C",
            RelType::Requested => "#9B59B6",
        }
    }

    pub fn dash_array(&self) -> &'static str {
        match self {
            RelType::Action => "",
            RelType::Consequence => "6,3",
            RelType::Target => "3,3",
            RelType::Consumes => "",
            RelType::Requested => "",
        }
    }

    pub fn stroke_width(&self) -> f64 {
        match self {
            RelType::Requested => 1.0,
            _ => 2.0,
        }
    }

    pub fn display(&self) -> &'static str {
        match self {
            RelType::Action => "ACTION",
            RelType::Consequence => "CONSEQUENCE",
            RelType::Target => "TARGET",
            RelType::Consumes => "CONSUMES",
            RelType::Requested => "REQUESTED",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl GraphData {
    pub fn merge(&mut self, other: GraphData) {
        for node in other.nodes {
            if !self.nodes.iter().any(|n| n.id == node.id) {
                self.nodes.push(node);
            }
        }
        for edge in other.edges {
            if !self.edges.iter().any(|e| e.id == edge.id) {
                self.edges.push(edge);
            }
        }
    }
}
