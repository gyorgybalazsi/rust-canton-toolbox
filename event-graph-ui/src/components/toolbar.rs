use crate::state::graph_state::Viewport;
use dioxus::prelude::*;

#[component]
pub fn Toolbar(viewport: Signal<Viewport>) -> Element {
    let zoom_in = {
        let mut viewport = viewport;
        move |_| {
            let new_zoom = (viewport.read().zoom * 1.2).clamp(0.1, 5.0);
            viewport.write().zoom = new_zoom;
        }
    };

    let zoom_out = {
        let mut viewport = viewport;
        move |_| {
            let new_zoom = (viewport.read().zoom / 1.2).clamp(0.1, 5.0);
            viewport.write().zoom = new_zoom;
        }
    };

    let reset = {
        let mut viewport = viewport;
        move |_| {
            let mut vp = viewport.write();
            vp.offset_x = 0.0;
            vp.offset_y = 0.0;
            vp.zoom = 1.0;
        }
    };

    let zoom_pct = (viewport.read().zoom * 100.0) as u32;

    rsx! {
        div { class: "toolbar",
            button { class: "tool-btn", onclick: zoom_in, "+" }
            span { class: "zoom-level", "{zoom_pct}%" }
            button { class: "tool-btn", onclick: zoom_out, "-" }
            button { class: "tool-btn", onclick: reset, "Reset" }
        }
    }
}
