use crate::models::graph::{GraphData, NodeLabel, RelType};
use crate::state::graph_state::Selection;
use dioxus::prelude::*;

/// Extract a transaction's subtree and generate Mermaid for it.
/// Finds the Transaction node (selected or its parent), then follows
/// ACTION/CONSEQUENCE edges forward and REQUESTED edges backward.
fn transaction_to_mermaid(graph: &GraphData, selected_id: &str) -> String {
    use std::collections::{HashSet, VecDeque};

    // Find the transaction node: either the selected node is a Transaction,
    // or find a Transaction that connects to it via ACTION/CONSEQUENCE
    let tx_id = if graph
        .nodes
        .iter()
        .any(|n| n.id == selected_id && n.label == NodeLabel::Transaction)
    {
        selected_id.to_string()
    } else {
        // Find a Transaction connected to selected node
        graph
            .edges
            .iter()
            .filter(|e| matches!(e.rel_type, RelType::Action | RelType::Consequence))
            .find(|e| e.target == selected_id)
            .map(|e| {
                // Walk back up to find the Transaction root
                let mut current = e.source.clone();
                for _ in 0..20 {
                    if graph
                        .nodes
                        .iter()
                        .any(|n| n.id == current && n.label == NodeLabel::Transaction)
                    {
                        return current;
                    }
                    if let Some(parent_edge) = graph
                        .edges
                        .iter()
                        .filter(|e2| matches!(e2.rel_type, RelType::Action | RelType::Consequence))
                        .find(|e2| e2.target == current)
                    {
                        current = parent_edge.source.clone();
                    } else {
                        break;
                    }
                }
                current
            })
            .unwrap_or_else(|| selected_id.to_string())
    };

    // BFS from tx_id following ACTION/CONSEQUENCE forward, REQUESTED backward
    let mut node_ids: HashSet<String> = HashSet::new();
    let mut edge_ids: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();

    node_ids.insert(tx_id.clone());
    queue.push_back(tx_id.clone());

    while let Some(current) = queue.pop_front() {
        for edge in &graph.edges {
            // Forward: ACTION, CONSEQUENCE from current
            if edge.source == current
                && matches!(
                    edge.rel_type,
                    RelType::Action | RelType::Consequence | RelType::Target | RelType::Consumes
                )
                && edge_ids.insert(edge.id.clone())
                && node_ids.insert(edge.target.clone())
            {
                queue.push_back(edge.target.clone());
            }
            // Backward: REQUESTED pointing to current (Party → Transaction)
            if edge.target == current
                && edge.rel_type == RelType::Requested
                && edge_ids.insert(edge.id.clone())
            {
                node_ids.insert(edge.source.clone());
            }
        }
    }

    // Build subgraph
    let sub = GraphData {
        nodes: graph
            .nodes
            .iter()
            .filter(|n| node_ids.contains(&n.id))
            .cloned()
            .collect(),
        edges: graph
            .edges
            .iter()
            .filter(|e| edge_ids.contains(&e.id))
            .cloned()
            .collect(),
    };

    let mermaid = graph_to_mermaid(&sub);
    let tables = graph_to_markdown_tables(&sub);
    format!("{mermaid}\n\n---\n\n{tables}")
}

fn graph_to_markdown_tables(graph: &GraphData) -> String {
    let mut out = String::new();

    for node in &graph.nodes {
        let type_name = node.label.display();
        let short_id = &node.id;
        out.push_str(&format!("### {type_name}: {}\n\n", node.display_name));
        out.push_str("| Property | Value |\n");
        out.push_str("|----------|-------|\n");

        let mut props: Vec<_> = node.properties.iter().collect();
        props.sort_by_key(|(a, _)| sort_key(a));

        for (key, value) in props {
            let val_str = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            // Truncate long values
            let val_display = if val_str.len() > 80 {
                format!("{}...", &val_str[..80])
            } else {
                val_str
            };
            // Escape pipes for markdown table
            let val_safe = val_display.replace('|', "\\|");
            out.push_str(&format!("| {key} | {val_safe} |\n"));
        }
        out.push_str(&format!("| _node_id_ | {short_id} |\n"));
        out.push('\n');
    }

    out
}

fn graph_to_mermaid(graph: &GraphData) -> String {
    let mut lines = vec!["```mermaid".to_string(), "graph TD".to_string()];

    // Node definitions
    for node in &graph.nodes {
        let id = node.id.replace(|c: char| !c.is_alphanumeric(), "_");
        let label = match node.label {
            NodeLabel::Transaction => {
                let uid = node
                    .properties
                    .get("update_id")
                    .and_then(|v| v.as_str())
                    .map(|s| if s.len() > 16 { &s[..16] } else { s })
                    .unwrap_or("TX");
                format!("T:{uid}")
            }
            NodeLabel::Created => {
                let tmpl = node
                    .properties
                    .get("template_name")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.rsplit('.').next())
                    .unwrap_or("Created");
                tmpl.to_string()
            }
            NodeLabel::Exercised => node
                .properties
                .get("choice_name")
                .and_then(|v| v.as_str())
                .unwrap_or("Exercised")
                .to_string(),
            NodeLabel::Party => {
                let pid = node
                    .properties
                    .get("party_id")
                    .and_then(|v| v.as_str())
                    .map(|s| if s.len() > 20 { &s[..20] } else { s })
                    .unwrap_or("Party");
                pid.to_string()
            }
        };

        // Sanitize label for Mermaid: remove/replace characters that break syntax
        let safe = label
            .replace('"', "'")
            .replace('[', "(")
            .replace(']', ")")
            .replace('{', "(")
            .replace('}', ")")
            .replace(['<', '>'], "")
            .replace('|', "/")
            .replace('#', "");

        let shape = match node.label {
            NodeLabel::Transaction => format!("    {id}[\"{safe}\"]"),
            NodeLabel::Created => format!("    {id}((\"{safe}\"))"),
            NodeLabel::Exercised => format!("    {id}{{\"{safe}\"}}"),
            NodeLabel::Party => format!("    {id}[\"{safe}\"]"),
        };
        lines.push(shape);
    }

    // Edges
    for edge in &graph.edges {
        let src = edge.source.replace(|c: char| !c.is_alphanumeric(), "_");
        let tgt = edge.target.replace(|c: char| !c.is_alphanumeric(), "_");
        let label = edge.rel_type.display();
        let style = match edge.rel_type {
            RelType::Action => format!("    {src} -->|{label}| {tgt}"),
            RelType::Consequence => format!("    {src} -.->|{label}| {tgt}"),
            RelType::Target => format!("    {src} -.->|{label}| {tgt}"),
            RelType::Consumes => format!("    {src} ==>|{label}| {tgt}"),
            RelType::Requested => format!("    {src} -->|{label}| {tgt}"),
        };
        lines.push(style);
    }

    // Styles
    lines.push(String::new());
    lines.push("    classDef transaction fill:#4A90D9,stroke:#fff,color:#fff".to_string());
    lines.push("    classDef created fill:#50C878,stroke:#fff,color:#fff".to_string());
    lines.push("    classDef exercised fill:#F5A623,stroke:#fff,color:#fff".to_string());
    lines.push("    classDef party fill:#9B59B6,stroke:#fff,color:#fff".to_string());

    for node in &graph.nodes {
        let id = node.id.replace(|c: char| !c.is_alphanumeric(), "_");
        let cls = match node.label {
            NodeLabel::Transaction => "transaction",
            NodeLabel::Created => "created",
            NodeLabel::Exercised => "exercised",
            NodeLabel::Party => "party",
        };
        lines.push(format!("    class {id} {cls}"));
    }

    lines.push("```".to_string());
    lines.join("\n")
}

/// Ordered property keys. Keys matching these appear first in this order.
/// Keys starting with "create_arg." are grouped together after template_name.
/// Everything else goes to the end.
fn sort_key(key: &str) -> (usize, String) {
    match key {
        "offset" => (0, String::new()),
        "template_name" => (1, String::new()),
        k if k.starts_with("create_arg.") => (2, k.to_string()),
        "signatories" => (3, String::new()),
        "created_at" => (4, String::new()),
        "contract_id" => (5, String::new()),
        "node_id" => (6, String::new()),
        other => (7, other.to_string()),
    }
}

#[component]
pub fn Sidebar(graph: GraphData, selection: Signal<Selection>) -> Element {
    let sel = selection.read();
    let selected_node = sel.selected_node(&graph);

    let copied = use_signal(|| false);
    let graph_for_mermaid = graph.clone();
    let sel_for_mermaid = selection;

    let copy_mermaid = move |_| {
        let sel = sel_for_mermaid.read();
        let _mermaid = if let Some(ref node_id) = sel.selected_node_id {
            transaction_to_mermaid(&graph_for_mermaid, node_id)
        } else {
            let m = graph_to_mermaid(&graph_for_mermaid);
            let t = graph_to_markdown_tables(&graph_for_mermaid);
            format!("{m}\n\n---\n\n{t}")
        };
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(window) = web_sys::window() {
                let clipboard = window.navigator().clipboard();
                let _ = clipboard.write_text(&mermaid);
                copied.set(true);
                spawn(async move {
                    gloo_timers::future::TimeoutFuture::new(2000).await;
                    copied.set(false);
                });
            }
        }
    };

    let has_graph = !graph.nodes.is_empty();

    rsx! {
        div { class: "sidebar",
            if has_graph {
                div { class: "mermaid-btn-row",
                    button {
                        class: "template-btn",
                        onclick: copy_mermaid,
                        if *copied.read() { "Copied!" } else { "Copy Mermaid" }
                    }
                }
            }
            h3 { "Node Details" }
            match selected_node {
                Some(node) => {
                    let mut props: Vec<_> = node.properties.iter().collect();
                    props.sort_by(|(a, _), (b, _)| {
                        sort_key(a).cmp(&sort_key(b))
                    });
                    rsx! {
                        div { class: "node-detail",
                            div { class: "node-label",
                                span {
                                    class: "label-badge",
                                    style: "background-color: {node.label.color()}",
                                    {node.label.display()}
                                }
                            }
                            div { class: "node-name", "{node.display_name}" }
                            div { class: "node-id", "ID: {node.id}" }
                            h4 { "Properties" }
                            div { class: "properties",
                                for (key, value) in props {
                                    div { class: "property-row",
                                        span { class: "prop-key", "{key}:" }
                                        span { class: "prop-value", "{value}" }
                                    }
                                }
                            }
                        }
                    }
                },
                None => rsx! {
                    p { class: "no-selection", "Click a node to view its details." }
                },
            }
        }
    }
}
