use crate::models::graph::{GraphData, GraphEdge, GraphNode, NodeLabel, RelType};
use crate::models::query::SchemaStats;
use dioxus::prelude::*;
use std::collections::HashMap;

fn parse_node_label(labels: &[String]) -> NodeLabel {
    for label in labels {
        match label.as_str() {
            "Transaction" => return NodeLabel::Transaction,
            "Created" => return NodeLabel::Created,
            "Exercised" => return NodeLabel::Exercised,
            "Party" => return NodeLabel::Party,
            _ => {}
        }
    }
    NodeLabel::Transaction
}

fn parse_rel_type(rel_type: &str) -> RelType {
    match rel_type {
        "ACTION" => RelType::Action,
        "CONSEQUENCE" => RelType::Consequence,
        "TARGET" => RelType::Target,
        "CONSUMES" => RelType::Consumes,
        "REQUESTED" => RelType::Requested,
        _ => RelType::Action,
    }
}

fn make_display_name(label: &NodeLabel, props: &HashMap<String, serde_json::Value>) -> String {
    match label {
        NodeLabel::Transaction => {
            if let Some(serde_json::Value::String(uid)) = props.get("update_id") {
                let short = if uid.len() > 12 { &uid[..12] } else { uid };
                format!("T:{short}")
            } else {
                "Transaction".to_string()
            }
        }
        NodeLabel::Created => {
            if let Some(serde_json::Value::String(tmpl)) = props.get("template_name") {
                tmpl.rsplit('.').next().unwrap_or(tmpl).to_string()
            } else {
                "Created".to_string()
            }
        }
        NodeLabel::Exercised => {
            if let Some(serde_json::Value::String(choice)) = props.get("choice_name") {
                choice.clone()
            } else {
                "Exercised".to_string()
            }
        }
        NodeLabel::Party => {
            if let Some(serde_json::Value::String(pid)) = props.get("party_id") {
                let short = if pid.len() > 20 { &pid[..20] } else { pid };
                short.to_string()
            } else {
                "Party".to_string()
            }
        }
    }
}

#[cfg(feature = "server")]
fn extract_node(node: &neo4rs::Node) -> GraphNode {
    let labels: Vec<String> = node.labels().iter().map(|l| l.to_string()).collect();
    let label = parse_node_label(&labels);
    let properties: HashMap<String, serde_json::Value> = node
        .keys()
        .iter()
        .filter_map(|k| {
            node.get::<serde_json::Value>(&**k)
                .ok()
                .map(|v| (k.to_string(), v))
        })
        .collect();
    let display_name = make_display_name(&label, &properties);
    GraphNode {
        id: node.id().to_string(),
        label,
        display_name,
        properties,
        x: 0.0,
        y: 0.0,
    }
}

#[cfg(feature = "server")]
fn extract_relation(rel: &neo4rs::Relation) -> GraphEdge {
    let rel_type = parse_rel_type(rel.typ());
    let properties: HashMap<String, serde_json::Value> = rel
        .keys()
        .iter()
        .filter_map(|k| {
            rel.get::<serde_json::Value>(&**k)
                .ok()
                .map(|v| (k.to_string(), v))
        })
        .collect();
    GraphEdge {
        id: rel.id().to_string(),
        source: rel.start_node_id().to_string(),
        target: rel.end_node_id().to_string(),
        rel_type,
        properties,
    }
}

/// Hierarchical tree layout.
///
/// 1. Sort Transaction nodes by `offset` property, place them in a horizontal
///    row at y=0.
/// 2. For each Transaction, walk ACTION edges to direct children (Created/Exercised),
///    then CONSEQUENCE edges deeper. Each child level is placed at increasing y,
///    spread horizontally under the parent.
/// 3. Party nodes are placed in a row below all trees.
fn layout_tree(nodes: &mut [GraphNode], edges: &[GraphEdge]) {
    use std::collections::{HashMap, HashSet, VecDeque};

    if nodes.is_empty() {
        return;
    }

    let id_to_idx: HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.clone(), i))
        .collect();

    // Build tree adjacency: only ACTION and CONSEQUENCE edges (directed).
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut has_parent: HashSet<usize> = HashSet::new();
    for edge in edges {
        if !matches!(edge.rel_type, RelType::Action | RelType::Consequence) {
            continue;
        }
        if let (Some(&src), Some(&tgt)) = (id_to_idx.get(&edge.source), id_to_idx.get(&edge.target))
        {
            children.entry(src).or_default().push(tgt);
            has_parent.insert(tgt);
        }
    }

    // Build REQUESTED adjacency: Party -> Transaction
    let mut party_to_txs: HashMap<usize, Vec<usize>> = HashMap::new();
    for edge in edges {
        if edge.rel_type != RelType::Requested {
            continue;
        }
        if let (Some(&src), Some(&tgt)) = (id_to_idx.get(&edge.source), id_to_idx.get(&edge.target))
        {
            party_to_txs.entry(src).or_default().push(tgt);
        }
    }

    // Identify Transaction roots (no incoming ACTION/CONSEQUENCE) and sort by offset
    let mut tx_roots: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| n.label == NodeLabel::Transaction && !has_parent.contains(i))
        .map(|(i, _)| i)
        .collect();

    tx_roots.sort_by(|&a, &b| {
        let offset_a = nodes[a]
            .properties
            .get("offset")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let offset_b = nodes[b]
            .properties
            .get("offset")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        offset_a.cmp(&offset_b)
    });

    const H_SPACING: f64 = 200.0;
    const V_SPACING: f64 = 120.0;

    let mut positioned: HashSet<usize> = HashSet::new();

    // --- Step 1: Lay out each transaction's subtree as a self-contained column ---
    // Each tree gets its own horizontal band. Within each tree, BFS levels
    // are centered under the root.

    // For each root, collect BFS levels (depth -> Vec<node_idx>)
    struct TreeLayout {
        root: usize,
        levels: Vec<Vec<usize>>, // depth 0 = root
    }

    let mut trees: Vec<TreeLayout> = Vec::new();
    for &root in &tx_roots {
        let mut levels: Vec<Vec<usize>> = Vec::new();
        let mut queue: VecDeque<(usize, usize)> = VecDeque::new();
        let mut visited: HashSet<usize> = HashSet::new();

        queue.push_back((root, 0));
        visited.insert(root);

        while let Some((idx, depth)) = queue.pop_front() {
            while levels.len() <= depth {
                levels.push(Vec::new());
            }
            levels[depth].push(idx);

            if let Some(kids) = children.get(&idx) {
                for &kid in kids {
                    if visited.insert(kid) {
                        queue.push_back((kid, depth + 1));
                    }
                }
            }
        }
        trees.push(TreeLayout { root, levels });
    }

    // Place trees side by side. Transaction roots are at y = V_SPACING (leaving
    // room for Party row at y = 0).
    let tx_y_start = V_SPACING;
    let mut global_x_cursor: f64 = 0.0;

    // Track each tree root's center x for Party placement later
    let mut root_center_x: HashMap<usize, f64> = HashMap::new();

    for tree in &trees {
        // Width of this tree = max nodes at any level * H_SPACING
        let max_level_width = tree.levels.iter().map(|l| l.len()).max().unwrap_or(1);
        let tree_width = (max_level_width as f64) * H_SPACING;

        for (depth, level) in tree.levels.iter().enumerate() {
            let level_width = (level.len() as f64) * H_SPACING;
            let start_x = global_x_cursor + (tree_width - level_width) / 2.0;

            for (j, &idx) in level.iter().enumerate() {
                nodes[idx].x = start_x + (j as f64) * H_SPACING + H_SPACING / 2.0;
                nodes[idx].y = tx_y_start + (depth as f64) * V_SPACING;
                positioned.insert(idx);
            }
        }

        // Root center
        root_center_x.insert(tree.root, global_x_cursor + tree_width / 2.0);

        global_x_cursor += tree_width.max(H_SPACING);
    }

    // --- Step 2: Place Party nodes above their connected transactions ---
    // Each Party node is placed at the average x of the Transaction roots
    // it connects to (via REQUESTED edges).
    let party_indices: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.label == NodeLabel::Party)
        .map(|(i, _)| i)
        .collect();

    // Compute barycenter for each party
    let mut party_positions: Vec<(usize, f64)> = party_indices
        .iter()
        .map(|&idx| {
            let tx_xs: Vec<f64> = party_to_txs
                .get(&idx)
                .map(|txs| {
                    txs.iter()
                        .filter_map(|&t| root_center_x.get(&t).copied())
                        .collect()
                })
                .unwrap_or_default();

            let x = if tx_xs.is_empty() {
                // Fallback: use any connected transaction's x
                nodes[idx].x
            } else {
                tx_xs.iter().sum::<f64>() / tx_xs.len() as f64
            };
            (idx, x)
        })
        .collect();

    // Sort by x position so they don't overlap
    party_positions.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    // Place with minimum spacing to avoid overlaps
    let mut last_x = f64::NEG_INFINITY;
    for &mut (idx, ref mut target_x) in &mut party_positions {
        let min_x = last_x + H_SPACING;
        let x = target_x.max(min_x);
        nodes[idx].x = x;
        nodes[idx].y = 0.0;
        positioned.insert(idx);
        last_x = x;
    }

    // --- Step 3: Place any remaining unpositioned nodes below all trees ---
    let max_y = nodes
        .iter()
        .filter(|n| positioned.contains(&id_to_idx[&n.id]))
        .map(|n| n.y)
        .fold(0.0f64, f64::max);

    let orphan_y = max_y + V_SPACING * 1.5;
    let mut orphan_x: f64 = 0.0;

    for (i, node) in nodes.iter_mut().enumerate() {
        if !positioned.contains(&i) {
            node.x = orphan_x + H_SPACING / 2.0;
            node.y = orphan_y;
            orphan_x += H_SPACING;
            positioned.insert(i);
        }
    }
}

/// Execute arbitrary Cypher and return graph data.
/// Wraps the query to extract nodes and relationships via COLLECT.
#[server]
pub async fn run_cypher(
    cypher: String,
    params: HashMap<String, String>,
    min_offset: Option<i64>,
    max_offset: Option<i64>,
) -> Result<GraphData, ServerFnError> {
    use super::neo4j_pool;

    let graph = neo4j_pool::pool();

    // Wrap user's Cypher to collect all nodes and relationships from paths.
    // We use CALL to run the user's query, then collect graph elements.
    let wrapped = format!(
        r#"CALL {{
  {cypher}
}}
WITH *
UNWIND keys({{_placeholder_: null}}) AS _dummy_
RETURN null"#,
    );

    // First, try the direct approach: run the user's query and extract
    // nodes/rels from each row by trying known types.
    let mut query = neo4rs::query(&cypher);
    for (k, v) in &params {
        query = query.param(k.as_str(), v.as_str());
    }
    // Pass offset window as Neo4j parameters ($min_off, $max_off)
    query = query
        .param("min_off", min_offset.unwrap_or(i64::MIN))
        .param("max_off", max_offset.unwrap_or(i64::MAX));

    let mut result = graph
        .execute(query)
        .await
        .map_err(|e| ServerFnError::new(format!("Neo4j query failed: {e}")))?;

    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut seen_nodes = std::collections::HashSet::new();
    let mut seen_edges = std::collections::HashSet::new();

    // We don't have access to column names directly, so try common column names
    // that the user might use. This covers the typical MATCH (a)-[r]->(b) RETURN a, r, b pattern.
    let _ = &wrapped; // suppress unused warning

    while let Some(row) = result
        .next()
        .await
        .map_err(|e| ServerFnError::new(format!("Neo4j row fetch failed: {e}")))?
    {
        // Try to extract nodes from common column names
        let node_names = [
            "n",
            "m",
            "a",
            "b",
            "t",
            "e",
            "c",
            "p",
            "node",
            "start",
            "end",
            "source",
            "target",
            "tx",
            "created",
            "exercised",
            "party",
        ];
        for name in &node_names {
            if let Ok(node) = row.get::<neo4rs::Node>(name) {
                let id = node.id().to_string();
                if seen_nodes.insert(id) {
                    nodes.push(extract_node(&node));
                }
            }
        }

        // Try to extract relationships
        let rel_names = ["r", "rel", "r1", "r2", "action", "consequence"];
        for name in &rel_names {
            if let Ok(rel) = row.get::<neo4rs::Relation>(name) {
                let id = rel.id().to_string();
                if seen_edges.insert(id) {
                    edges.push(extract_relation(&rel));
                }
            }
        }

        // Try to extract paths
        let path_names = ["path", "p"];
        for name in &path_names {
            if let Ok(path) = row.get::<neo4rs::Path>(name) {
                let path_nodes = path.nodes();
                for node in &path_nodes {
                    let id = node.id().to_string();
                    if seen_nodes.insert(id) {
                        nodes.push(extract_node(node));
                    }
                }
                // For paths, pair nodes[i] -> rels[i] -> nodes[i+1]
                let path_rels = path.rels();
                for (i, rel) in path_rels.iter().enumerate() {
                    let id = rel.id().to_string();
                    if seen_edges.insert(id.clone()) {
                        let rel_type = parse_rel_type(rel.typ());
                        let keys = rel.keys();
                        let properties: HashMap<String, serde_json::Value> = keys
                            .iter()
                            .filter_map(|k| {
                                rel.get::<serde_json::Value>(&**k)
                                    .ok()
                                    .map(|v| (k.to_string(), v))
                            })
                            .collect();
                        let source = path_nodes
                            .get(i)
                            .map(|n: &neo4rs::Node| n.id().to_string())
                            .unwrap_or_default();
                        let target = path_nodes
                            .get(i + 1)
                            .map(|n: &neo4rs::Node| n.id().to_string())
                            .unwrap_or_default();
                        edges.push(GraphEdge {
                            id,
                            source,
                            target,
                            rel_type,
                            properties,
                        });
                    }
                }
            }
        }
    }

    // Hierarchical tree layout: Transactions in a horizontal row at top,
    // consequences as trees below each transaction.
    layout_tree(&mut nodes, &edges);

    // Debug: dump layout to file
    {
        use std::io::Write;
        if let Ok(mut f) = std::fs::File::create("/tmp/graph-layout.txt") {
            let _ = writeln!(
                f,
                "=== GRAPH LAYOUT ({} nodes, {} edges) ===",
                nodes.len(),
                edges.len()
            );
            for n in &nodes {
                let _ = writeln!(
                    f,
                    "  {:>12} x={:>8.0} y={:>8.0}  {}",
                    n.label.display(),
                    n.x,
                    n.y,
                    n.display_name
                );
            }
            for e in &edges {
                let _ = writeln!(
                    f,
                    "  edge {} -> {} [{}]",
                    e.source,
                    e.target,
                    e.rel_type.display()
                );
            }
        }
    }

    Ok(GraphData { nodes, edges })
}

/// Get schema statistics: node counts by label, relationship counts by type
#[server]
pub async fn get_schema_stats() -> Result<SchemaStats, ServerFnError> {
    use super::neo4j_pool;

    let graph = neo4j_pool::pool();

    let mut node_counts = HashMap::new();
    for label in &["Transaction", "Created", "Exercised", "Party"] {
        let query = neo4rs::query(&format!("MATCH (n:{label}) RETURN count(n) as cnt"));
        let mut result = graph
            .execute(query)
            .await
            .map_err(|e| ServerFnError::new(format!("Neo4j query failed: {e}")))?;
        if let Some(row) = result
            .next()
            .await
            .map_err(|e| ServerFnError::new(format!("Neo4j row fetch failed: {e}")))?
        {
            let cnt: i64 = row.get("cnt").unwrap_or(0);
            node_counts.insert(label.to_string(), cnt as u64);
        }
    }

    let mut rel_counts = HashMap::new();
    for rel in &["ACTION", "CONSEQUENCE", "TARGET", "CONSUMES", "REQUESTED"] {
        let query = neo4rs::query(&format!("MATCH ()-[r:{rel}]->() RETURN count(r) as cnt"));
        let mut result = graph
            .execute(query)
            .await
            .map_err(|e| ServerFnError::new(format!("Neo4j query failed: {e}")))?;
        if let Some(row) = result
            .next()
            .await
            .map_err(|e| ServerFnError::new(format!("Neo4j row fetch failed: {e}")))?
        {
            let cnt: i64 = row.get("cnt").unwrap_or(0);
            rel_counts.insert(rel.to_string(), cnt as u64);
        }
    }

    Ok(SchemaStats {
        node_counts,
        rel_counts,
    })
}
