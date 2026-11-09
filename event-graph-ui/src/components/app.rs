use crate::components::analytics::Analytics;
use crate::components::graph_canvas::GraphCanvas;
use crate::components::query_editor::QueryEditor;
use crate::components::sidebar::Sidebar;
use crate::components::sync_tab::SyncTab;
use crate::components::toolbar::Toolbar;
use crate::models::graph::{GraphData, NodeLabel, RelType};
use crate::server::analytics::get_max_offset;
use crate::state::graph_state::{Selection, Viewport};
use dioxus::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};

const MAIN_CSS: Asset = asset!("/assets/main.css");

#[derive(Clone, Copy, PartialEq)]
enum ActiveTab {
    Graph,
    Analytics,
    Sync,
}

/// Given full graph data, compute which node indices belong to each transaction's
/// subtree (BFS via ACTION/CONSEQUENCE edges from transaction root).
fn compute_tx_subtrees(data: &GraphData) -> Vec<(usize, Vec<usize>)> {
    let id_to_idx: HashMap<&str, usize> = data
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();

    // Build tree adjacency: source -> children (ACTION/CONSEQUENCE only)
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut has_parent: HashSet<usize> = HashSet::new();
    for edge in &data.edges {
        if !matches!(edge.rel_type, RelType::Action | RelType::Consequence) {
            continue;
        }
        if let (Some(&src), Some(&tgt)) = (
            id_to_idx.get(edge.source.as_str()),
            id_to_idx.get(edge.target.as_str()),
        ) {
            children.entry(src).or_default().push(tgt);
            has_parent.insert(tgt);
        }
    }

    // Find transaction roots (no incoming ACTION/CONSEQUENCE), sorted by offset
    let mut tx_roots: Vec<usize> = data
        .nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| n.label == NodeLabel::Transaction && !has_parent.contains(i))
        .map(|(i, _)| i)
        .collect();

    tx_roots.sort_by(|&a, &b| {
        let oa = data.nodes[a]
            .properties
            .get("offset")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let ob = data.nodes[b]
            .properties
            .get("offset")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        oa.cmp(&ob)
    });

    // BFS each root to collect its subtree
    let mut result = Vec::new();
    for root in tx_roots {
        let mut subtree = Vec::new();
        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();
        queue.push_back(root);
        visited.insert(root);
        while let Some(idx) = queue.pop_front() {
            subtree.push(idx);
            if let Some(kids) = children.get(&idx) {
                for &kid in kids {
                    if visited.insert(kid) {
                        queue.push_back(kid);
                    }
                }
            }
        }
        result.push((root, subtree));
    }
    result
}

/// Compute the visible subgraph at a given step of the replay.
/// Step N means transactions 0..=N are visible, plus all Party nodes.
/// Edges are included if both endpoints are visible.
fn subgraph_at_step(
    data: &GraphData,
    tx_subtrees: &[(usize, Vec<usize>)],
    step: usize,
) -> GraphData {
    let mut visible_indices: HashSet<usize> = HashSet::new();

    // Always include Party nodes
    for (i, n) in data.nodes.iter().enumerate() {
        if n.label == NodeLabel::Party {
            visible_indices.insert(i);
        }
    }

    // Include subtrees for transactions 0..=step
    for (_, subtree) in tx_subtrees.iter().take(step + 1) {
        for &idx in subtree {
            visible_indices.insert(idx);
        }
    }

    let visible_ids: HashSet<&str> = visible_indices
        .iter()
        .map(|&i| data.nodes[i].id.as_str())
        .collect();

    let nodes: Vec<_> = visible_indices
        .iter()
        .map(|&i| data.nodes[i].clone())
        .collect();

    let edges: Vec<_> = data
        .edges
        .iter()
        .filter(|e| {
            visible_ids.contains(e.source.as_str()) && visible_ids.contains(e.target.as_str())
        })
        .cloned()
        .collect();

    GraphData { nodes, edges }
}

#[component]
pub fn App() -> Element {
    let mut graph = use_signal(GraphData::default);
    let viewport = use_signal(Viewport::default);
    let mut selection = use_signal(Selection::default);
    let mut active_tab = use_signal(|| ActiveTab::Graph);

    // Graph tab offset window
    let mut graph_window_size = use_signal(|| 100i64);
    let mut graph_end_offset = use_signal(String::new); // empty = no filter
    let mut graph_max_offset: Signal<Option<i64>> = use_signal(|| None);

    // Fetch max offset for graph tab
    let _graph_max = use_future(move || async move {
        if let Ok(Some(max)) = get_max_offset().await {
            graph_max_offset.set(Some(max));
            // Set initial end offset to latest
            graph_end_offset.set(max.to_string());
        }
    });

    // Replay state
    let mut full_data = use_signal(GraphData::default);
    let mut replay_step = use_signal(|| Option::<usize>::None); // None = not replaying
    let mut replay_total = use_signal(|| 0usize);
    let mut auto_replay = use_signal(|| false); // true = auto-advance, false = manual step

    // Replay timer: when auto_replay and replay_step is Some, advance every 3s
    let _replay_timer = use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(3000).await;
            if !*auto_replay.read() {
                continue;
            }
            let step = *replay_step.read();
            let total = *replay_total.read();
            if let Some(s) = step {
                if s < total.saturating_sub(1) {
                    let next = s + 1;
                    replay_step.set(Some(next));
                    let data = full_data.read().clone();
                    let subtrees = compute_tx_subtrees(&data);
                    let sub = subgraph_at_step(&data, &subtrees, next);
                    graph.set(sub);
                } else {
                    // Replay finished
                    replay_step.set(None);
                    auto_replay.set(false);
                }
            }
        }
    });

    // Advance one step manually
    let advance_step = move || {
        let step = *replay_step.read();
        let total = *replay_total.read();
        if let Some(s) = step {
            if s < total.saturating_sub(1) {
                let next = s + 1;
                replay_step.set(Some(next));
                let data = full_data.read().clone();
                let subtrees = compute_tx_subtrees(&data);
                let sub = subgraph_at_step(&data, &subtrees, next);
                graph.set(sub);
            } else {
                replay_step.set(None);
                auto_replay.set(false);
            }
        }
    };

    let on_result = move |data: GraphData| {
        replay_step.set(None);
        auto_replay.set(false);
        graph.set(data.clone());
        full_data.set(data);
        selection.set(Selection::default());
    };

    let mut start_replay = move |data: GraphData, auto: bool| {
        let subtrees = compute_tx_subtrees(&data);
        let total = subtrees.len();
        if total == 0 {
            graph.set(data.clone());
            full_data.set(data);
            return;
        }
        full_data.set(data.clone());
        replay_total.set(total);
        auto_replay.set(auto);
        let sub = subgraph_at_step(&data, &subtrees, 0);
        graph.set(sub);
        selection.set(Selection::default());
        replay_step.set(Some(0));
    };

    let on_replay = move |data: GraphData| {
        start_replay(data, true);
    };

    let on_step_start = move |data: GraphData| {
        start_replay(data, false);
    };

    let mut advance_step = advance_step;
    let on_step_next = move |_: ()| {
        advance_step();
    };

    let is_replaying = replay_step.read().is_some();
    let step_display = (*replay_step.read()).map(|s| s + 1).unwrap_or(0);
    let total_display = *replay_total.read();

    rsx! {
        document::Stylesheet { href: MAIN_CSS }
        div { class: "app-container",
            div { class: "top-bar",
                h1 { "Ledger Graph UI" }
                div { class: "top-bar-right",
                    if is_replaying {
                        span { class: "replay-status", "Replaying {step_display}/{total_display}" }
                    }
                    Toolbar { viewport }
                }
            }
            div { class: "tab-bar",
                button {
                    class: if *active_tab.read() == ActiveTab::Graph { "tab-btn active" } else { "tab-btn" },
                    onclick: move |_| active_tab.set(ActiveTab::Graph),
                    "Graph"
                }
                button {
                    class: if *active_tab.read() == ActiveTab::Analytics { "tab-btn active" } else { "tab-btn" },
                    onclick: move |_| active_tab.set(ActiveTab::Analytics),
                    "Analytics"
                }
                button {
                    class: if *active_tab.read() == ActiveTab::Sync { "tab-btn active" } else { "tab-btn" },
                    onclick: move |_| active_tab.set(ActiveTab::Sync),
                    "Sync"
                }
            }
            if *active_tab.read() == ActiveTab::Graph {
                {
                    // Compute offset bounds from window signals
                    let end_str = graph_end_offset.read().clone();
                    let win = *graph_window_size.read();
                    let (gmin, gmax) = if end_str.is_empty() {
                        (None, None)
                    } else if let Some(v) = crate::models::analytics::parse_offset(&end_str) {
                        if v <= 0 {
                            // Negative = relative to max
                            let max_off = graph_max_offset.read().unwrap_or(0);
                            let end_off = max_off + v;
                            (Some(end_off - win), Some(end_off))
                        } else {
                            (Some(v - win), Some(v))
                        }
                    } else {
                        (None, None)
                    };

                    rsx! {
                        div { class: "main-content",
                            div { class: "left-panel",
                                QueryEditor {
                                    on_result: on_result,
                                    on_replay: on_replay,
                                    on_step_start: on_step_start,
                                    on_step_next: on_step_next,
                                    is_stepping: is_replaying && !*auto_replay.read(),
                                    min_offset: gmin,
                                    max_offset: gmax,
                                }
                            }
                            div { class: "center-panel",
                                GraphCanvas {
                                    graph: graph.read().clone(),
                                    viewport,
                                    selection,
                                }
                            }
                            div { class: "right-panel",
                                div { class: "graph-offset-window",
                                    h4 { "Offset Window" }
                                    div { class: "zoom-slider",
                                        div { class: "zoom-row",
                                            span { class: "zoom-label", "End offset:" }
                                            input {
                                                r#type: "text",
                                                class: "zoom-date-input",
                                                placeholder: "all (no filter)",
                                                value: "{graph_end_offset}",
                                                oninput: move |evt| graph_end_offset.set(evt.value()),
                                            }
                                        }
                                        div { class: "zoom-row",
                                            span { class: "zoom-label", "Window:" }
                                            input {
                                                r#type: "text",
                                                class: "zoom-date-input",
                                                placeholder: "e.g. 100, 10K, 1M",
                                                value: graph_window_size.read().to_string(),
                                                oninput: move |evt| {
                                                    if let Some(v) = crate::models::analytics::parse_offset(&evt.value())
                                                        && v > 0 { graph_window_size.set(v); }
                                                },
                                            }
                                        }
                                        div { class: "zoom-nav",
                                            button {
                                                class: "analytics-btn",
                                                onclick: move |_| {
                                                    if let Ok(max) = graph_max_offset.read().ok_or(()) {
                                                        graph_end_offset.set(max.to_string());
                                                    }
                                                },
                                                "Latest"
                                            }
                                            button {
                                                class: "analytics-btn",
                                                onclick: move |_| {
                                                    let win = *graph_window_size.read();
                                                    let current = graph_end_offset.read().clone();
                                                    if let Some(v) = crate::models::analytics::parse_offset(&current) {
                                                        let new_end = (v - win).max(win);
                                                        graph_end_offset.set(new_end.to_string());
                                                    }
                                                },
                                                "<< Prev"
                                            }
                                            button {
                                                class: "analytics-btn",
                                                onclick: move |_| {
                                                    let win = *graph_window_size.read();
                                                    let max_off = graph_max_offset.read().unwrap_or(i64::MAX);
                                                    let current = graph_end_offset.read().clone();
                                                    if let Some(v) = crate::models::analytics::parse_offset(&current) {
                                                        let new_end = (v + win).min(max_off);
                                                        graph_end_offset.set(new_end.to_string());
                                                    }
                                                },
                                                "Next >>"
                                            }
                                        }
                                    }
                                }
                                Sidebar {
                                    graph: graph.read().clone(),
                                    selection,
                                }
                                div { class: "nav-help",
                                    h4 { "Navigation" }
                                    div { class: "nav-help-items",
                                        div { class: "nav-help-item", span { class: "nav-key", "Arrows" } " Pan" }
                                        div { class: "nav-help-item", span { class: "nav-key", "+" } " Zoom in" }
                                        div { class: "nav-help-item", span { class: "nav-key", "-" } " Zoom out" }
                                        div { class: "nav-help-item", span { class: "nav-key", "0" } " Reset view" }
                                        div { class: "nav-help-item", span { class: "nav-key", "Dbl-click" } " Center on point" }
                                        div { class: "nav-help-item", span { class: "nav-key", "Drag" } " Pan" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if *active_tab.read() == ActiveTab::Analytics {
                div { class: "main-content", Analytics {} }
            }
            if *active_tab.read() == ActiveTab::Sync {
                div { class: "main-content", SyncTab {} }
            }
        }
    }
}
