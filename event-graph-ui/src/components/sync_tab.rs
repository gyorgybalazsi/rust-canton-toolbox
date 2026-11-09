use crate::server::sync::{
    SyncProfile, SyncStatus, get_sync_profiles, get_sync_status, start_sync, stop_sync,
};
use dioxus::prelude::*;

#[component]
pub fn SyncTab() -> Element {
    let mut profiles: Signal<Vec<SyncProfile>> = use_signal(Vec::new);
    let mut selected_profile = use_signal(String::new);
    let mut fresh = use_signal(|| false);
    let mut starting_offset = use_signal(String::new);
    let mut status: Signal<Option<SyncStatus>> = use_signal(|| None);
    let mut error: Signal<Option<String>> = use_signal(|| None);
    let mut starting = use_signal(|| false);
    let mut stopping = use_signal(|| false);

    // Load profiles on mount
    let _load = use_future(move || async move {
        match get_sync_profiles().await {
            Ok(profs) => {
                if let Some(first) = profs.first() {
                    selected_profile.set(first.name.clone());
                    if let Some(off) = first.starting_offset {
                        starting_offset.set(off.to_string());
                    }
                }
                profiles.set(profs);
            }
            Err(e) => error.set(Some(format!("Failed to load profiles: {e}"))),
        }
    });

    // Poll status every 3 seconds
    let _poll = use_future(move || async move {
        loop {
            match get_sync_status().await {
                Ok(s) => status.set(Some(s)),
                Err(e) => error.set(Some(format!("Status error: {e}"))),
            }
            gloo_timers::future::TimeoutFuture::new(3000).await;
        }
    });

    let on_start = move |_| {
        let profile = selected_profile.read().clone();
        let is_fresh = *fresh.read();
        let offset: Option<i64> = {
            let s = starting_offset.read().clone();
            if s.is_empty() { None } else { s.parse().ok() }
        };
        if profile.is_empty() {
            error.set(Some("Select a profile first".to_string()));
            return;
        }
        starting.set(true);
        error.set(None);
        spawn(async move {
            match start_sync(profile, is_fresh, offset).await {
                Ok(()) => {}
                Err(e) => error.set(Some(format!("{e}"))),
            }
            starting.set(false);
        });
    };

    let on_stop = move |_| {
        stopping.set(true);
        error.set(None);
        spawn(async move {
            match stop_sync().await {
                Ok(()) => {}
                Err(e) => error.set(Some(format!("{e}"))),
            }
            stopping.set(false);
        });
    };

    let st = status.read();
    let is_running = st.as_ref().is_some_and(|s| s.running);
    let prof_list = profiles.read();

    rsx! {
        // Left panel: controls
        div { class: "sync-left-panel",
            h3 { "Sync Control" }

            div { class: "sync-form",
                div { class: "sync-field",
                    label { class: "sync-label", "Profile:" }
                    select {
                        class: "sync-select",
                        value: "{selected_profile}",
                        disabled: is_running,
                        oninput: move |evt| {
                            let name = evt.value();
                            selected_profile.set(name.clone());
                            let off = profiles.read().iter()
                                .find(|p| p.name == name)
                                .and_then(|p| p.starting_offset)
                                .map(|o| o.to_string())
                                .unwrap_or_default();
                            starting_offset.set(off);
                        },
                        for prof in prof_list.iter() {
                            {
                                rsx! {
                                    option {
                                        value: "{prof.name}",
                                        "{prof.name}"
                                    }
                                }
                            }
                        }
                    }
                }

                // Show URL for selected profile
                {
                    let sel = selected_profile.read().clone();
                    let prof = prof_list.iter().find(|p| p.name == sel);
                    let url = prof.map(|p| p.url.as_str()).unwrap_or("");
                    rsx! {
                        div { class: "sync-url", "{url}" }
                    }
                }

                div { class: "sync-field",
                    label { class: "sync-label", "Starting offset:" }
                    input {
                        r#type: "text",
                        class: "sync-select",
                        placeholder: "e.g. -5000000 (empty = default)",
                        value: "{starting_offset}",
                        disabled: is_running,
                        oninput: move |evt| starting_offset.set(evt.value()),
                    }
                }

                div { class: "sync-field",
                    label { class: "sync-checkbox-label",
                        input {
                            r#type: "checkbox",
                            checked: *fresh.read(),
                            disabled: is_running,
                            oninput: move |evt| {
                                fresh.set(evt.value() == "true");
                            },
                        }
                        " Fresh start"
                    }
                    if *fresh.read() {
                        div { class: "sync-warning", "Warning: This will clear all Neo4j data!" }
                    }
                }

                div { class: "sync-buttons",
                    if !is_running {
                        button {
                            class: "sync-start-btn",
                            disabled: *starting.read(),
                            onclick: on_start,
                            if *starting.read() { "Starting..." } else { "Start Sync" }
                        }
                    } else {
                        button {
                            class: "sync-stop-btn",
                            disabled: *stopping.read(),
                            onclick: on_stop,
                            if *stopping.read() { "Stopping..." } else { "Stop Sync" }
                        }
                    }
                }

                if let Some(ref err) = *error.read() {
                    div { class: "sync-error", "{err}" }
                }
            }
        }

        // Center panel: log viewer
        div { class: "sync-center-panel",
            h3 { "Sync Log" }
            div { class: "sync-log-viewer",
                match st.as_ref() {
                    Some(s) if !s.log_lines.is_empty() => rsx! {
                        for (i, line) in s.log_lines.iter().enumerate() {
                            {
                                let cls = if line.contains("ERROR") || line.contains("error") {
                                    "log-line log-error"
                                } else if line.contains("WARN") || line.contains("warn") {
                                    "log-line log-warn"
                                } else if line.contains("[Progress]") {
                                    "log-line log-progress"
                                } else {
                                    "log-line"
                                };
                                rsx! {
                                    div { key: "{i}", class: cls, "{line}" }
                                }
                            }
                        }
                    },
                    _ => rsx! {
                        div { class: "sync-log-empty", "No log output yet. Start sync to see logs." }
                    },
                }
            }
        }

        // Right panel: status
        div { class: "sync-right-panel",
            h3 { "Status" }
            div { class: "sync-status-card",
                div { class: "sync-status-row",
                    span { class: "sync-status-label", "State:" }
                    if is_running {
                        span { class: "sync-status-value sync-running", "Running" }
                    } else {
                        span { class: "sync-status-value sync-stopped", "Stopped" }
                    }
                }

                if let Some(ref s) = *st {
                    if let Some(ref prof) = s.profile {
                        div { class: "sync-status-row",
                            span { class: "sync-status-label", "Profile:" }
                            span { class: "sync-status-value", "{prof}" }
                        }
                    }
                    if let Some(pid) = s.pid {
                        div { class: "sync-status-row",
                            span { class: "sync-status-label", "PID:" }
                            span { class: "sync-status-value", "{pid}" }
                        }
                    }
                    if let Some(offset) = s.neo4j_offset {
                        div { class: "sync-status-row",
                            span { class: "sync-status-label", "Neo4j Offset:" }
                            span { class: "sync-status-value", "{offset}" }
                        }
                    }
                    if let Some(count) = s.transaction_count {
                        div { class: "sync-status-row",
                            span { class: "sync-status-label", "Transactions:" }
                            span { class: "sync-status-value", "{count}" }
                        }
                    }
                }
            }
        }
    }
}
