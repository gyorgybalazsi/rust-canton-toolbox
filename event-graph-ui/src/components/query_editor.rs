use crate::models::graph::GraphData;
use crate::server::queries::run_cypher;
use dioxus::prelude::*;
use std::collections::HashMap;

#[component]
pub fn QueryEditor(
    on_result: EventHandler<GraphData>,
    on_replay: EventHandler<GraphData>,
    on_step_start: EventHandler<GraphData>,
    on_step_next: EventHandler<()>,
    is_stepping: bool,
    min_offset: Option<i64>,
    max_offset: Option<i64>,
) -> Element {
    let mut cypher = use_signal(|| {
        "MATCH (t:Transaction)\nWHERE t.offset >= $min_off AND t.offset <= $max_off\nAND EXISTS {\n  MATCH (t)-[:ACTION]->(e)\n  WHERE NOT (e:Exercised AND (e.choice_name CONTAINS 'Validator' OR e.choice_name = 'WalletAppInstall_ExecuteBatch'))\n}\nMATCH path = (t)-[:ACTION|CONSEQUENCE|TARGET|CONSUMES*1..]->(e)\nRETURN path".to_string()
    });
    let mut error = use_signal(|| Option::<String>::None);
    let mut loading = use_signal(|| false);

    let execute = move |_| {
        let query = cypher.read().clone();
        loading.set(true);
        error.set(None);
        spawn(async move {
            match run_cypher(query, HashMap::new(), min_offset, max_offset).await {
                Ok(data) => on_result.call(data),
                Err(e) => error.set(Some(format!("{e}"))),
            }
            loading.set(false);
        });
    };

    let replay = move |_| {
        let query = cypher.read().clone();
        loading.set(true);
        error.set(None);
        spawn(async move {
            match run_cypher(query, HashMap::new(), min_offset, max_offset).await {
                Ok(data) => on_replay.call(data),
                Err(e) => error.set(Some(format!("{e}"))),
            }
            loading.set(false);
        });
    };

    let step_start = move |_| {
        let query = cypher.read().clone();
        loading.set(true);
        error.set(None);
        spawn(async move {
            match run_cypher(query, HashMap::new(), min_offset, max_offset).await {
                Ok(data) => on_step_start.call(data),
                Err(e) => error.set(Some(format!("{e}"))),
            }
            loading.set(false);
        });
    };

    let step_next = move |_| {
        on_step_next.call(());
    };

    let is_loading = *loading.read();

    rsx! {
        div { class: "query-editor",
            h3 { "Cypher Query" }
            textarea {
                class: "cypher-input",
                rows: 6,
                value: "{cypher}",
                oninput: move |evt| cypher.set(evt.value()),
            }
            div { class: "query-actions",
                button {
                    class: "execute-btn",
                    disabled: is_loading,
                    onclick: execute,
                    if is_loading { "Running..." } else { "Execute" }
                }
                button {
                    class: "replay-btn",
                    disabled: is_loading,
                    onclick: replay,
                    "Replay"
                }
            }
            div { class: "query-actions",
                if is_stepping {
                    button {
                        class: "step-btn",
                        onclick: step_next,
                        "Next Step"
                    }
                } else {
                    button {
                        class: "step-btn",
                        disabled: is_loading,
                        onclick: step_start,
                        "Step by Step"
                    }
                }
            }
            if let Some(err) = error.read().as_ref() {
                div { class: "query-error", "{err}" }
            }
            div { class: "query-templates",
                h4 { "Templates" }
                button {
                    class: "template-btn",
                    onclick: move |_| {
                        cypher.set("MATCH (t:Transaction)\nWHERE t.offset >= $min_off AND t.offset <= $max_off\nAND EXISTS {\n  MATCH (t)-[:ACTION]->(e)\n  WHERE NOT (e:Exercised AND (e.choice_name CONTAINS 'Validator' OR e.choice_name = 'WalletAppInstall_ExecuteBatch'))\n}\nMATCH path = (t)-[:ACTION|CONSEQUENCE|TARGET|CONSUMES*1..]->(e)\nRETURN path".to_string());
                    },
                    "Query All"
                }
            }
            {
                let mut tx_id = use_signal(String::new);
                let mut tx_lookup = move |_| {
                    let uid = tx_id.read().clone();
                    if uid.is_empty() { return; }
                    cypher.set(format!(
                        "MATCH (t:Transaction {{update_id: '{uid}'}})\n\
                         OPTIONAL MATCH path = (t)-[:ACTION|CONSEQUENCE|TARGET|CONSUMES*1..]->(e)\n\
                         OPTIONAL MATCH (p:Party)-[r:REQUESTED]->(t)\n\
                         RETURN t, path, p, r"
                    ));
                    // Auto-execute
                    let query = cypher.read().clone();
                    loading.set(true);
                    error.set(None);
                    spawn(async move {
                        match run_cypher(query, HashMap::new(), min_offset, max_offset).await {
                            Ok(data) => on_result.call(data),
                            Err(e) => error.set(Some(format!("{e}"))),
                        }
                        loading.set(false);
                    });
                };
                rsx! {
                    div { class: "query-templates",
                        h4 { "Transaction Lookup" }
                        input {
                            class: "cypher-input",
                            r#type: "text",
                            placeholder: "Enter update_id...",
                            value: "{tx_id}",
                            oninput: move |evt| tx_id.set(evt.value()),
                            onkeydown: move |evt| {
                                if evt.key() == Key::Enter {
                                    tx_lookup(());
                                }
                            },
                        }
                        button {
                            class: "template-btn",
                            onclick: move |_| tx_lookup(()),
                            "Lookup"
                        }
                    }
                }
            }
        }
    }
}
