use crate::models::graph::{GraphEdge, GraphNode, RelType};
use dioxus::prelude::*;

/// How much to curve the edge. Positive = curve left, negative = curve right
/// relative to the source→target direction.
fn curve_offset(edge: &GraphEdge, source: &GraphNode, target: &GraphNode) -> f64 {
    let same_layer = (source.y - target.y).abs() < 1.0;

    // Base offset by relationship type so overlapping edges between the
    // same pair of nodes fan out rather than stack.
    let type_offset = match edge.rel_type {
        RelType::Action => 0.0,
        RelType::Consequence => 30.0,
        RelType::Target => -40.0,
        RelType::Consumes => 40.0,
        RelType::Requested => 0.0,
    };

    if same_layer {
        // Same-layer edges need a strong arc to be visible
        let dist = (target.x - source.x).abs().max(50.0);
        let base = dist * 0.4; // arc height proportional to distance
        base + type_offset
    } else {
        type_offset
    }
}

#[component]
pub fn GraphEdgeView(edge: GraphEdge, source: GraphNode, target: GraphNode) -> Element {
    let color = edge.rel_type.stroke_color();
    let dash = edge.rel_type.dash_array();
    let width = edge.rel_type.stroke_width();

    let offset = curve_offset(&edge, &source, &target);

    // Direction vector
    let dx = target.x - source.x;
    let dy = target.y - source.y;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / len;
    let uy = dy / len;

    // Perpendicular vector (rotated 90° CCW)
    let px = -uy;
    let py = ux;

    // Control point for quadratic bezier: midpoint + perpendicular offset
    let mx = (source.x + target.x) / 2.0 + px * offset;
    let my = (source.y + target.y) / 2.0 + py * offset;

    let path_d = format!(
        "M {},{} Q {},{} {},{}",
        source.x, source.y, mx, my, target.x, target.y
    );

    // Point on the bezier at t=0.5 for arrowhead and label
    // Q bezier at t=0.5: B = 0.25*P0 + 0.5*C + 0.25*P2
    let bx = 0.25 * source.x + 0.5 * mx + 0.25 * target.x;
    let by = 0.25 * source.y + 0.5 * my + 0.25 * target.y;

    // Tangent at t=0.5: B' = (C - P0)(1-t) + (P2 - C)t at t=0.5 = 0.5*(P2 - P0)
    // Actually for quadratic: B'(t) = 2(1-t)(C-P0) + 2t(P2-C)
    // At t=0.5: B'= (C-P0) + (P2-C) = P2 - P0 ... same direction as straight line
    // Use derivative: tangent_x = 2*(1-0.5)*(mx-source.x) + 2*0.5*(target.x-mx)
    let tx = (mx - source.x) + (target.x - mx);
    let ty = (my - source.y) + (target.y - my);
    let tlen = (tx * tx + ty * ty).sqrt().max(1.0);
    let tux = tx / tlen;
    let tuy = ty / tlen;

    // Arrowhead at midpoint of curve
    let arrow_size = 8.0;
    let ax = bx + tux * arrow_size;
    let ay = by + tuy * arrow_size;
    let b1x = bx - tux * arrow_size + tuy * arrow_size * 0.5;
    let b1y = by - tuy * arrow_size - tux * arrow_size * 0.5;
    let cx = bx - tux * arrow_size - tuy * arrow_size * 0.5;
    let cy = by - tuy * arrow_size + tux * arrow_size * 0.5;
    let arrow_points = format!("{ax},{ay} {b1x},{b1y} {cx},{cy}");

    // Label position: slightly offset from midpoint of curve
    let label_x = bx;
    let label_y = by - 6.0;

    rsx! {
        g {
            path {
                d: path_d,
                stroke: color,
                stroke_width: width,
                stroke_dasharray: dash,
                fill: "none",
            }
            polygon {
                points: arrow_points,
                fill: color,
            }
            text {
                x: label_x,
                y: label_y,
                text_anchor: "middle",
                font_size: "9px",
                fill: "#666",
                {edge.rel_type.display().to_string()}
            }
        }
    }
}
