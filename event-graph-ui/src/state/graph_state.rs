use crate::models::graph::{GraphData, GraphNode};

#[derive(Clone, Debug, PartialEq)]
pub struct Viewport {
    pub offset_x: f64,
    pub offset_y: f64,
    pub zoom: f64,
    pub width: f64,
    pub height: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            offset_x: 0.0,
            offset_y: 0.0,
            zoom: 1.0,
            width: 800.0,
            height: 600.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub selected_node_id: Option<String>,
}

impl Selection {
    pub fn selected_node<'a>(&self, graph: &'a GraphData) -> Option<&'a GraphNode> {
        let id = self.selected_node_id.as_ref()?;
        graph.nodes.iter().find(|n| &n.id == id)
    }
}
