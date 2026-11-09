use crate::models::graph::{GraphNode, NodeLabel};
use dioxus::prelude::*;

const R: f64 = 24.0;

/// Lighter tint of the node color for a subtle gradient effect
fn light_color(label: &NodeLabel) -> &'static str {
    match label {
        NodeLabel::Transaction => "#6DB3F8",
        NodeLabel::Created => "#7EDBA0",
        NodeLabel::Exercised => "#FFD080",
        NodeLabel::Party => "#C49ADB",
    }
}

/// Text color that contrasts with the node fill
fn text_color(label: &NodeLabel) -> &'static str {
    match label {
        NodeLabel::Transaction => "#1a3a5c",
        NodeLabel::Created => "#1a4a2a",
        NodeLabel::Exercised => "#5a3a00",
        NodeLabel::Party => "#3a1a4a",
    }
}

/// SVG path icon rendered at (cx, cy) with given size. White fill, no stroke.
#[component]
fn NodeIcon(label: NodeLabel, cx: f64, cy: f64, size: f64) -> Element {
    let s = size;
    let hs = s / 2.0;
    match label {
        NodeLabel::Transaction => {
            // Checkmark in a box (committed/finalized)
            let b = s * 0.38; // box half-size
            let br = s * 0.08; // box corner radius
            let box_path = format!(
                "M {},{} L {},{} Q {},{} {},{} L {},{} Q {},{} {},{} L {},{} Q {},{} {},{} L {},{} Q {},{} {},{}",
                cx - b,
                cy - b + br,
                cx - b,
                cy + b - br,
                cx - b,
                cy + b,
                cx - b + br,
                cy + b,
                cx + b - br,
                cy + b,
                cx + b,
                cy + b,
                cx + b,
                cy + b - br,
                cx + b,
                cy - b + br,
                cx + b,
                cy - b,
                cx + b - br,
                cy - b,
                cx - b + br,
                cy - b,
                cx - b,
                cy - b,
                cx - b,
                cy - b + br,
            );
            // Checkmark: starts bottom-left, dips to bottom-center, rises to top-right
            let check = format!(
                "M {},{} L {},{} L {},{}",
                cx - b * 0.5,
                cy,
                cx - b * 0.1,
                cy + b * 0.4,
                cx + b * 0.55,
                cy - b * 0.4,
            );
            rsx! {
                path { d: box_path, fill: "none", stroke: "white", stroke_width: 1.5 }
                path { d: check, fill: "none", stroke: "white", stroke_width: 2.5, stroke_linecap: "round", stroke_linejoin: "round" }
            }
        }
        NodeLabel::Created => {
            // Document with large folded corner
            let w = s * 0.6;
            let h = s * 0.8;
            let fold = s * 0.35;
            let doc_path = format!(
                "M {},{} L {},{} L {},{} L {},{} L {},{} Z",
                cx - w / 2.0,
                cy - h / 2.0, // top-left
                cx + w / 2.0 - fold,
                cy - h / 2.0, // top-right before fold
                cx + w / 2.0,
                cy - h / 2.0 + fold, // fold point
                cx + w / 2.0,
                cy + h / 2.0, // bottom-right
                cx - w / 2.0,
                cy + h / 2.0, // bottom-left
            );
            let fold_path = format!(
                "M {},{} L {},{} L {},{}",
                cx + w / 2.0 - fold,
                cy - h / 2.0,
                cx + w / 2.0 - fold,
                cy - h / 2.0 + fold,
                cx + w / 2.0,
                cy - h / 2.0 + fold,
            );
            let fold_fill = format!(
                "M {},{} L {},{} L {},{} Z",
                cx + w / 2.0 - fold,
                cy - h / 2.0,
                cx + w / 2.0 - fold,
                cy - h / 2.0 + fold,
                cx + w / 2.0,
                cy - h / 2.0 + fold,
            );
            rsx! {
                path { d: doc_path, fill: "white", opacity: "0.9" }
                path { d: fold_fill, fill: "rgba(0,0,0,0.08)" }
                path { d: fold_path, fill: "none", stroke: "rgba(0,0,0,0.2)", stroke_width: 1.0 }
            }
        }
        NodeLabel::Exercised => {
            // Lightning bolt / action arrow
            let bolt_path = format!(
                "M {},{} L {},{} L {},{} L {},{} L {},{} L {},{} Z",
                cx - hs * 0.1,
                cy - hs * 0.8, // top
                cx - hs * 0.5,
                cy + hs * 0.05, // left mid
                cx - hs * 0.1,
                cy + hs * 0.05, // inner left
                cx + hs * 0.1,
                cy + hs * 0.8, // bottom
                cx + hs * 0.5,
                cy - hs * 0.05, // right mid
                cx + hs * 0.1,
                cy - hs * 0.05, // inner right
            );
            rsx! {
                path { d: bolt_path, fill: "white", opacity: "0.95" }
            }
        }
        NodeLabel::Party => {
            // Person silhouette: head circle + shoulders arc
            let head_r = s * 0.18;
            let head_cy = cy - s * 0.15;
            let body_path = format!(
                "M {},{} Q {},{} {},{} Q {},{} {},{}",
                cx - s * 0.35,
                cy + s * 0.35, // left shoulder
                cx - s * 0.35,
                cy + s * 0.05, // left curve control
                cx,
                cy + s * 0.05, // neck center
                cx + s * 0.35,
                cy + s * 0.05, // right curve control
                cx + s * 0.35,
                cy + s * 0.35, // right shoulder
            );
            rsx! {
                circle { cx: cx, cy: head_cy, r: head_r, fill: "white", opacity: "0.95" }
                path { d: body_path, fill: "white", opacity: "0.95" }
            }
        }
    }
}

#[component]
pub fn GraphNodeView(
    node: GraphNode,
    is_selected: bool,
    is_consumed: bool,
    on_click: EventHandler<String>,
) -> Element {
    let color = node.label.color();
    let light = light_color(&node.label);
    let txt_color = text_color(&node.label);
    let filter = if is_selected {
        "url(#glow)"
    } else {
        "url(#shadow)"
    };
    let stroke = if is_selected {
        "#FFD700"
    } else {
        "rgba(255,255,255,0.6)"
    };
    let stroke_w = if is_selected { 3.0 } else { 1.5 };
    let id = node.id.clone();
    let node_label = node.label.clone();

    match node.label {
        NodeLabel::Transaction => {
            // Pill-shaped rounded rectangle
            let w = 56.0;
            let h = 36.0;
            rsx! {
                g {
                    onclick: move |_| on_click.call(id.clone()),
                    cursor: "pointer",
                    filter: filter,
                    rect {
                        x: node.x - w / 2.0,
                        y: node.y - h / 2.0,
                        width: w,
                        height: h,
                        rx: 10.0,
                        fill: light,
                        stroke: stroke,
                        stroke_width: stroke_w,
                    }
                    rect {
                        x: node.x - w / 2.0,
                        y: node.y - h / 2.0,
                        width: w,
                        height: h,
                        rx: 10.0,
                        fill: color,
                        opacity: "0.7",
                        stroke: "none",
                    }
                    // Icon
                    NodeIcon { label: node_label.clone(), cx: node.x, cy: node.y, size: 22.0 }
                    // Label below
                    text {
                        x: node.x,
                        y: node.y + h / 2.0 + 14.0,
                        text_anchor: "middle",
                        font_size: "10px",
                        font_weight: "500",
                        fill: txt_color,
                        {node.display_name.clone()}
                    }
                }
            }
        }
        NodeLabel::Created => {
            // Consumed/archived nodes get washed-out solid colors
            let (fill_main, fill_overlay, ring_color, label_color) = if is_consumed {
                ("#C8D8C8", "#B0C4B0", "#BFCFBF", "#8a9a8a")
            } else {
                (light, color, light, txt_color)
            };
            rsx! {
                g {
                    onclick: move |_| on_click.call(id.clone()),
                    cursor: "pointer",
                    filter: filter,
                    // Outer ring
                    circle {
                        cx: node.x,
                        cy: node.y,
                        r: R + 2.0,
                        fill: "none",
                        stroke: ring_color,
                        stroke_width: 2.0,
                        opacity: "0.4",
                    }
                    // Main circle
                    circle {
                        cx: node.x,
                        cy: node.y,
                        r: R,
                        fill: fill_main,
                        stroke: stroke,
                        stroke_width: stroke_w,
                    }
                    circle {
                        cx: node.x,
                        cy: node.y,
                        r: R,
                        fill: fill_overlay,
                        opacity: "0.6",
                        stroke: "none",
                    }
                    // Icon
                    NodeIcon { label: node_label.clone(), cx: node.x, cy: node.y, size: 24.0 }
                    // Label below
                    text {
                        x: node.x,
                        y: node.y + R + 16.0,
                        text_anchor: "middle",
                        font_size: "10px",
                        font_weight: "500",
                        fill: label_color,
                        {node.display_name.clone()}
                    }
                }
            }
        }
        NodeLabel::Exercised => {
            // Diamond with rounded feel (larger)
            let r = R + 4.0;
            let points = format!(
                "{},{} {},{} {},{} {},{}",
                node.x,
                node.y - r,
                node.x + r,
                node.y,
                node.x,
                node.y + r,
                node.x - r,
                node.y
            );
            let points_inner = {
                let ri = r - 3.0;
                format!(
                    "{},{} {},{} {},{} {},{}",
                    node.x,
                    node.y - ri,
                    node.x + ri,
                    node.y,
                    node.x,
                    node.y + ri,
                    node.x - ri,
                    node.y
                )
            };
            rsx! {
                g {
                    onclick: move |_| on_click.call(id.clone()),
                    cursor: "pointer",
                    filter: filter,
                    polygon {
                        points: points.clone(),
                        fill: light,
                        stroke: stroke,
                        stroke_width: stroke_w,
                        stroke_linejoin: "round",
                    }
                    polygon {
                        points: points_inner,
                        fill: color,
                        opacity: "0.6",
                        stroke: "none",
                        stroke_linejoin: "round",
                    }
                    // Icon
                    NodeIcon { label: node_label.clone(), cx: node.x, cy: node.y, size: 22.0 }
                    // Label below
                    text {
                        x: node.x,
                        y: node.y + r + 14.0,
                        text_anchor: "middle",
                        font_size: "10px",
                        font_weight: "500",
                        fill: txt_color,
                        {node.display_name.clone()}
                    }
                }
            }
        }
        NodeLabel::Party => {
            // Hexagon (larger, with inner layer)
            let r = R + 4.0;
            let hex_points = |radius: f64| -> String {
                (0..6)
                    .map(|i| {
                        let angle =
                            std::f64::consts::PI / 3.0 * (i as f64) - std::f64::consts::PI / 6.0;
                        format!(
                            "{},{}",
                            node.x + radius * angle.cos(),
                            node.y + radius * angle.sin()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let outer = hex_points(r);
            let inner = hex_points(r - 3.0);
            rsx! {
                g {
                    onclick: move |_| on_click.call(id.clone()),
                    cursor: "pointer",
                    filter: filter,
                    polygon {
                        points: outer,
                        fill: light,
                        stroke: stroke,
                        stroke_width: stroke_w,
                        stroke_linejoin: "round",
                    }
                    polygon {
                        points: inner,
                        fill: color,
                        opacity: "0.6",
                        stroke: "none",
                    }
                    // Icon
                    // Icon
                    NodeIcon { label: node_label.clone(), cx: node.x, cy: node.y, size: 24.0 }
                    // Label below
                    text {
                        x: node.x,
                        y: node.y + r + 14.0,
                        text_anchor: "middle",
                        font_size: "10px",
                        font_weight: "500",
                        fill: txt_color,
                        {node.display_name.clone()}
                    }
                }
            }
        }
    }
}
