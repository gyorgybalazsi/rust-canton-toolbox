use crate::components::graph_edge::GraphEdgeView;
use crate::components::graph_node::GraphNodeView;
use crate::models::graph::GraphData;
use crate::state::graph_state::{Selection, Viewport};
use dioxus::prelude::*;

const PADDING: f64 = 80.0;
const PAN_FRACTION: f64 = 0.2; // pan 20% of viewBox per keypress
const ZOOM_FACTOR: f64 = 0.6; // zoom to 60% of current viewBox

fn compute_default_view_box(graph: &GraphData) -> (f64, f64, f64, f64) {
    if graph.nodes.is_empty() {
        return (0.0, 0.0, 800.0, 600.0);
    }

    let min_x = graph
        .nodes
        .iter()
        .map(|n| n.x)
        .fold(f64::INFINITY, f64::min);
    let max_x = graph
        .nodes
        .iter()
        .map(|n| n.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = graph
        .nodes
        .iter()
        .map(|n| n.y)
        .fold(f64::INFINITY, f64::min);
    let max_y = graph
        .nodes
        .iter()
        .map(|n| n.y)
        .fold(f64::NEG_INFINITY, f64::max);

    let x = min_x - PADDING;
    let y = min_y - PADDING;
    let w = (max_x - min_x + PADDING * 2.0).max(200.0);
    let h = (max_y - min_y + PADDING * 2.0).max(200.0);

    (x, y, w, h)
}

#[component]
pub fn GraphCanvas(
    graph: GraphData,
    viewport: Signal<Viewport>,
    selection: Signal<Selection>,
) -> Element {
    // Use viewBox for all navigation (pan + zoom).
    // No <g transform> needed — viewBox IS the camera.
    let default_vb = compute_default_view_box(&graph);
    let mut vb = use_signal(move || default_vb);

    // Reset viewBox when graph data changes (new query result)
    let node_count = graph.nodes.len();
    let mut last_node_count = use_signal(|| 0usize);
    if node_count != *last_node_count.read() {
        last_node_count.set(node_count);
        vb.set(compute_default_view_box(&graph));
    }

    let (vb_x, vb_y, vb_w, vb_h) = *vb.read();
    let view_box_str = format!("{vb_x} {vb_y} {vb_w} {vb_h}");

    // Pan: mouse drag
    let mut dragging = use_signal(|| false);
    let mut last_mouse = use_signal(|| (0.0f64, 0.0f64));
    // Store SVG element screen size for drag-to-viewBox mapping
    let svg_screen_size = use_signal(|| (800.0f64, 600.0f64));

    let on_mouse_down = move |evt: MouseEvent| {
        dragging.set(true);
        let coords = evt.client_coordinates();
        last_mouse.set((coords.x, coords.y));
        // Try to get element size (approximate from initial render or previous)
        #[cfg(target_arch = "wasm32")]
        {
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                if let Some(el) = doc.query_selector("svg.graph-canvas").ok().flatten() {
                    let rect = el.get_bounding_client_rect();
                    if rect.width() > 0.0 {
                        svg_screen_size.set((rect.width(), rect.height()));
                    }
                }
            }
        }
    };

    let on_mouse_move = move |evt: MouseEvent| {
        if *dragging.read() {
            let coords = evt.client_coordinates();
            let (lx, ly) = *last_mouse.read();
            let screen_dx = coords.x - lx;
            let screen_dy = coords.y - ly;
            // Convert screen pixel drag to viewBox units
            let (sw, sh) = *svg_screen_size.read();
            let (_, _, vw, vh) = *vb.read();
            let vb_dx = screen_dx * vw / sw;
            let vb_dy = screen_dy * vh / sh;
            let (ox, oy, ow, oh) = *vb.read();
            vb.set((ox - vb_dx, oy - vb_dy, ow, oh));
            last_mouse.set((coords.x, coords.y));
        }
    };

    let on_mouse_up = move |_: MouseEvent| {
        dragging.set(false);
    };

    // Double-click: center viewBox on clicked point
    let on_dblclick = move |evt: MouseEvent| {
        let _coords = evt.client_coordinates();
        let (sw, sh) = *svg_screen_size.read();
        let (_ox, _oy, _ow, _oh) = *vb.read();
        if sw > 0.0 && sh > 0.0 {
            // Map screen click to viewBox coordinates
            // Get SVG element position on screen
            #[cfg(target_arch = "wasm32")]
            {
                if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                    if let Some(el) = doc.query_selector("svg.graph-canvas").ok().flatten() {
                        let rect = el.get_bounding_client_rect();
                        let frac_x = (coords.x - rect.x()) / rect.width();
                        let frac_y = (coords.y - rect.y()) / rect.height();
                        let content_x = ox + frac_x * ow;
                        let content_y = oy + frac_y * oh;
                        // Re-center viewBox on this point
                        vb.set((content_x - ow / 2.0, content_y - oh / 2.0, ow, oh));
                    }
                }
            }
        }
    };

    // Keyboard: arrows pan, +/- zoom
    let graph_for_reset = graph.clone();
    let on_key_down = move |evt: KeyboardEvent| {
        let key = evt.key();
        evt.prevent_default();

        let (ox, oy, ow, oh) = *vb.read();

        match key {
            // Zoom in: shrink viewBox (see less, bigger)
            Key::Character(ref c) if c == "+" || c == "=" => {
                let new_w = ow * ZOOM_FACTOR;
                let new_h = oh * ZOOM_FACTOR;
                // Keep center fixed
                let cx = ox + ow / 2.0;
                let cy = oy + oh / 2.0;
                vb.set((cx - new_w / 2.0, cy - new_h / 2.0, new_w, new_h));
            }
            // Zoom out: grow viewBox (see more, smaller)
            Key::Character(ref c) if c == "-" || c == "_" => {
                let new_w = ow / ZOOM_FACTOR;
                let new_h = oh / ZOOM_FACTOR;
                let cx = ox + ow / 2.0;
                let cy = oy + oh / 2.0;
                vb.set((cx - new_w / 2.0, cy - new_h / 2.0, new_w, new_h));
            }
            // Reset
            Key::Character(ref c) if c == "0" => {
                vb.set(compute_default_view_box(&graph_for_reset));
            }
            // Pan
            Key::ArrowLeft => {
                vb.set((ox - ow * PAN_FRACTION, oy, ow, oh));
            }
            Key::ArrowRight => {
                vb.set((ox + ow * PAN_FRACTION, oy, ow, oh));
            }
            Key::ArrowUp => {
                vb.set((ox, oy - oh * PAN_FRACTION, ow, oh));
            }
            Key::ArrowDown => {
                vb.set((ox, oy + oh * PAN_FRACTION, ow, oh));
            }
            _ => {}
        }
    };

    let sel = selection.read().clone();

    // Compute set of node IDs that have an incoming CONSUMES edge
    let consumed_ids: std::collections::HashSet<&str> = graph
        .edges
        .iter()
        .filter(|e| e.rel_type == crate::models::graph::RelType::Consumes)
        .map(|e| e.target.as_str())
        .collect();

    rsx! {
        svg {
            class: "graph-canvas",
            view_box: view_box_str,
            preserve_aspect_ratio: "xMidYMid meet",
            tabindex: "0",
            onmousedown: on_mouse_down,
            onmousemove: on_mouse_move,
            onmouseup: on_mouse_up,
            ondoubleclick: on_dblclick,
            onkeydown: on_key_down,

            // Definitions: drop shadow, selection glow
            defs {
                dangerous_inner_html: "<filter id=\"shadow\" x=\"-20%\" y=\"-20%\" width=\"140%\" height=\"140%\"><feGaussianBlur in=\"SourceAlpha\" stdDeviation=\"3\" result=\"blur\"/><feOffset dx=\"2\" dy=\"2\" result=\"shifted\"/><feFlood flood-color=\"rgba(0,0,0,0.25)\" result=\"color\"/><feComposite in=\"color\" in2=\"shifted\" operator=\"in\" result=\"shadow\"/><feMerge><feMergeNode in=\"shadow\"/><feMergeNode in=\"SourceGraphic\"/></feMerge></filter><filter id=\"glow\" x=\"-30%\" y=\"-30%\" width=\"160%\" height=\"160%\"><feGaussianBlur in=\"SourceAlpha\" stdDeviation=\"4\" result=\"blur\"/><feFlood flood-color=\"gold\" result=\"color\"/><feComposite in=\"color\" in2=\"blur\" operator=\"in\" result=\"glow\"/><feMerge><feMergeNode in=\"glow\"/><feMergeNode in=\"SourceGraphic\"/></feMerge></filter>",
            }

            // Background
            rect {
                width: "10000",
                height: "10000",
                x: "-5000",
                y: "-5000",
                fill: "#f8f9fa",
            }

            // Render edges first (below nodes) — no transform group needed
            for edge in graph.edges.iter() {
                {
                    let source = graph.nodes.iter().find(|n| n.id == edge.source);
                    let target = graph.nodes.iter().find(|n| n.id == edge.target);
                    if let (Some(s), Some(t)) = (source, target) {
                        rsx! {
                            GraphEdgeView {
                                key: "{edge.id}",
                                edge: edge.clone(),
                                source: s.clone(),
                                target: t.clone(),
                            }
                        }
                    } else {
                        rsx! {}
                    }
                }
            }

            // Render nodes
            for node in graph.nodes.iter() {
                {
                    let is_selected = sel.selected_node_id.as_ref() == Some(&node.id);
                    let is_consumed = consumed_ids.contains(node.id.as_str());
                    let mut sel_signal = selection;
                    rsx! {
                        GraphNodeView {
                            key: "{node.id}",
                            node: node.clone(),
                            is_selected: is_selected,
                            is_consumed: is_consumed,
                            on_click: move |id: String| {
                                sel_signal.write().selected_node_id = Some(id);
                            },
                        }
                    }
                }
            }
        }
    }
}
