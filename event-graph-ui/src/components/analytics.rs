use crate::components::analytics_chart::AnalyticsChart;
use crate::components::analytics_queries::AnalyticsQueriesPanel;
use crate::models::analytics::AnalyticsQuery;
use crate::server::analytics::{
    delete_analytics_query, get_max_offset, get_offset_dates, load_analytics_queries,
    run_analytics_query, save_analytics_query,
};
use dioxus::prelude::*;
use std::collections::{HashMap, HashSet};

/// Per-series data points keyed by label, paired with the ordered day-index axis.
type AggregatedByDate = (HashMap<String, Vec<(i64, f64)>>, Vec<(i64, String)>);

/// Aggregate query results by date.
/// ACS queries (label contains "ACS" or "Active Contract"): take the last value per date.
/// All other queries: sum values per date.
/// Returns data keyed by a synthetic "day index" so the chart can plot it.
fn aggregate_by_date(
    results: &HashMap<String, Vec<(i64, f64)>>,
    offset_dates: &[(i64, String)],
    active: &HashSet<String>,
) -> AggregatedByDate {
    // Build offset → date (YYYY-MM-DD) mapping
    let offset_to_date: HashMap<i64, String> = offset_dates
        .iter()
        .map(|(o, d)| (*o, d[..10.min(d.len())].to_string()))
        .collect();

    // Collect all unique dates in order
    let mut all_dates: Vec<String> = offset_dates
        .iter()
        .map(|(_, d)| d[..10.min(d.len())].to_string())
        .collect();
    all_dates.dedup();

    // Map date → day_index (0, 1, 2, ...)
    let date_to_idx: HashMap<&str, i64> = all_dates
        .iter()
        .enumerate()
        .map(|(i, d)| (d.as_str(), i as i64))
        .collect();

    let mut aggregated: HashMap<String, Vec<(i64, f64)>> = HashMap::new();

    for (label, data) in results {
        if !active.contains(label) {
            continue;
        }

        let is_acs = label.contains("ACS") || label.contains("Active Contract");

        // Group values by date
        let mut date_values: HashMap<&str, Vec<f64>> = HashMap::new();
        for &(offset, value) in data {
            if let Some(date) = offset_to_date.get(&offset) {
                date_values.entry(date.as_str()).or_default().push(value);
            }
        }

        // Aggregate per date
        let mut agg_data: Vec<(i64, f64)> = Vec::new();
        for date in &all_dates {
            if let Some(values) = date_values.get(date.as_str()) {
                let agg_value = if is_acs {
                    // Take the last value (latest offset for this date)
                    *values.last().unwrap()
                } else {
                    // Sum all values
                    values.iter().sum()
                };
                if let Some(&idx) = date_to_idx.get(date.as_str()) {
                    agg_data.push((idx, agg_value));
                }
            }
        }

        aggregated.insert(label.clone(), agg_data);
    }

    // Build date labels: (day_index, date_string)
    let date_labels: Vec<(i64, String)> = all_dates
        .iter()
        .enumerate()
        .map(|(i, d)| (i as i64, format!("{d}T00:00:00Z")))
        .collect();

    (aggregated, date_labels)
}

/// Compute zoom range from end_offset input, window_size, and known max offset.
fn apply_window(
    end_offset_input: Signal<String>,
    window_size: Signal<i64>,
    max_offset_cache: Signal<Option<i64>>,
    mut zoom_range: Signal<Option<(i64, i64)>>,
) {
    let Some(data_max) = *max_offset_cache.read() else {
        zoom_range.set(None);
        return;
    };

    use crate::models::analytics::parse_offset;

    let input = end_offset_input.read().clone();
    let win = *window_size.read();

    let end_off = if input.is_empty() {
        data_max
    } else if let Some(v) = parse_offset(&input) {
        if v <= 0 { data_max + v } else { v }
    } else {
        data_max
    };

    let start_off = end_off - win;
    zoom_range.set(Some((start_off, end_off)));
}

#[component]
pub fn Analytics() -> Element {
    let mut query_results: Signal<HashMap<String, Vec<(i64, f64)>>> = use_signal(HashMap::new);
    let mut offset_dates: Signal<Vec<(i64, String)>> = use_signal(Vec::new);
    let mut active_queries: Signal<HashSet<String>> = use_signal(HashSet::new);
    let mut zoom_range: Signal<Option<(i64, i64)>> = use_signal(|| None);
    let mut window_size = use_signal(|| 100i64);
    let mut end_offset_input = use_signal(String::new); // empty = latest
    let mut date_mode = use_signal(|| false); // false = offset, true = date
    let mut loading_queries: Signal<HashSet<String>> = use_signal(HashSet::new);
    let mut query_errors: Signal<HashMap<String, String>> = use_signal(HashMap::new);
    let mut saved_queries: Signal<Vec<AnalyticsQuery>> = use_signal(Vec::new);
    let mut max_offset_cache: Signal<Option<i64>> = use_signal(|| None);
    let mut dates_loaded = use_signal(|| false);

    // Load saved queries and max offset on mount
    let _load = use_future(move || async move {
        match load_analytics_queries().await {
            Ok(queries) => saved_queries.set(queries),
            Err(e) => tracing::error!("Failed to load analytics queries: {e}"),
        }
        match get_max_offset().await {
            Ok(Some(max)) => {
                max_offset_cache.set(Some(max));
                // Set initial zoom range
                let win = *window_size.read();
                zoom_range.set(Some((max - win, max)));
            }
            Ok(None) => {}
            Err(e) => tracing::error!("Failed to get max offset: {e}"),
        }
    });

    // Helper: activate a query by label (fetch data, update signals).
    // Defined as a plain function-like closure that takes all needed signals explicitly,
    // so it can be called from both on_toggle and on_save without borrowing conflicts.
    let mut activate_query = move |label: String| {
        if !*dates_loaded.read() {
            dates_loaded.set(true);
            spawn(async move {
                match get_offset_dates().await {
                    Ok(dates) => offset_dates.set(dates),
                    Err(e) => tracing::error!("Failed to load offset dates: {e}"),
                }
            });
        }

        let query = saved_queries
            .read()
            .iter()
            .find(|q| q.label == label)
            .cloned();
        if let Some(q) = query {
            // Compute offset bounds from window signals
            let zoom = *zoom_range.read();
            let (min_off, max_off) = match zoom {
                Some((min, max)) => (Some(min), Some(max)),
                None => (None, None),
            };
            loading_queries.write().insert(label.clone());
            query_errors.write().remove(&label);
            let label_done = label.clone();
            spawn(async move {
                match run_analytics_query(q.cypher, min_off, max_off).await {
                    Ok(data) => {
                        query_results.write().insert(label_done.clone(), data);
                    }
                    Err(e) => {
                        query_errors
                            .write()
                            .insert(label_done.clone(), format!("{e}"));
                    }
                }
                loading_queries.write().remove(&label_done);
            });
        }
    };

    let on_toggle = move |label: String| {
        let mut active = active_queries.write();
        if active.contains(&label) {
            active.remove(&label);
            drop(active);
            query_results.write().remove(&label);
            query_errors.write().remove(&label);
        } else {
            active.insert(label.clone());
            drop(active);
            activate_query(label);
        }
    };

    let on_delete = move |label: String| {
        let label_clone = label.clone();
        spawn(async move {
            match delete_analytics_query(label_clone.clone()).await {
                Ok(()) => {
                    saved_queries.write().retain(|q| q.label != label_clone);
                    active_queries.write().remove(&label_clone);
                    query_results.write().remove(&label_clone);
                    query_errors.write().remove(&label_clone);
                }
                Err(e) => tracing::error!("Failed to delete query: {e}"),
            }
        });
    };

    let on_save = move |(label, cypher, min_time, max_time): (
        String,
        String,
        Option<String>,
        Option<String>,
    )| {
        let label_clone = label.clone();
        spawn(async move {
            match save_analytics_query(
                label_clone.clone(),
                cypher.clone(),
                min_time.clone(),
                max_time.clone(),
            )
            .await
            {
                Ok(()) => {
                    match load_analytics_queries().await {
                        Ok(queries) => saved_queries.set(queries),
                        Err(e) => tracing::error!("Failed to reload queries: {e}"),
                    }
                    active_queries.write().insert(label_clone.clone());
                    if !*dates_loaded.read() {
                        dates_loaded.set(true);
                        if let Ok(dates) = get_offset_dates().await {
                            offset_dates.set(dates);
                        }
                    }
                    let q_cypher = cypher.clone();
                    let zoom = *zoom_range.read();
                    let (min_off, max_off) = match zoom {
                        Some((min, max)) => (Some(min), Some(max)),
                        None => (None, None),
                    };
                    loading_queries.write().insert(label_clone.clone());
                    match run_analytics_query(q_cypher, min_off, max_off).await {
                        Ok(data) => {
                            query_results.write().insert(label_clone.clone(), data);
                        }
                        Err(e) => {
                            query_errors
                                .write()
                                .insert(label_clone.clone(), format!("{e}"));
                        }
                    }
                    loading_queries.write().remove(&label_clone);
                }
                Err(e) => {
                    query_errors.write().insert(label_clone, format!("{e}"));
                }
            }
        });
    };

    let on_refresh = move |_| {
        spawn(async move {
            match get_offset_dates().await {
                Ok(dates) => offset_dates.set(dates),
                Err(e) => tracing::error!("Failed to refresh offset dates: {e}"),
            }
        });

        let active = active_queries.read().clone();
        let queries = saved_queries.read().clone();
        let zoom = *zoom_range.read();
        let (min_off, max_off) = match zoom {
            Some((min, max)) => (Some(min), Some(max)),
            None => (None, None),
        };
        for label in active {
            if let Some(q) = queries.iter().find(|q| q.label == label) {
                let q = q.clone();
                let label_done = label.clone();
                loading_queries.write().insert(label.clone());
                query_errors.write().remove(&label);
                spawn(async move {
                    match run_analytics_query(q.cypher, min_off, max_off).await {
                        Ok(data) => {
                            query_results.write().insert(label_done.clone(), data);
                        }
                        Err(e) => {
                            query_errors
                                .write()
                                .insert(label_done.clone(), format!("{e}"));
                        }
                    }
                    loading_queries.write().remove(&label_done);
                });
            }
        }
    };

    // Apply window: compute zoom range from end_offset + window_size
    apply_window(end_offset_input, window_size, max_offset_cache, zoom_range);

    let on_download_csv = move |_| {
        let results = query_results.read().clone();
        let dates = offset_dates.read().clone();
        let active = active_queries.read().clone();
        let queries = saved_queries.read().clone();
        let zoom = *zoom_range.read();

        let csv = build_csv(&results, &dates, &active, &queries, zoom);
        download_csv(&csv);
    };

    let zoom = *zoom_range.read();

    rsx! {
        div { class: "analytics-left-panel",
            AnalyticsQueriesPanel {
                saved_queries: saved_queries.read().clone(),
                active_queries: active_queries,
                loading_queries: loading_queries,
                query_errors: query_errors,
                on_toggle: on_toggle,
                on_delete: on_delete,
                on_save: on_save,
            }
        }
        div { class: "analytics-center-panel",
            {
                let is_date = *date_mode.read();
                let raw_results = query_results.read().clone();
                let raw_dates = offset_dates.read().clone();
                let active = active_queries.read().clone();
                let saved = saved_queries.read().clone();
                let loading = !loading_queries.read().is_empty();

                let (chart_results, chart_dates, chart_zoom, show_offset_ticks) = if is_date {
                    let (agg_results, agg_dates) = aggregate_by_date(&raw_results, &raw_dates, &active);
                    // No zoom in date mode — show all dates
                    (agg_results, agg_dates, None, false)
                } else {
                    (raw_results, raw_dates, zoom, true)
                };

                rsx! {
                    AnalyticsChart {
                        query_results: chart_results,
                        offset_dates: chart_dates,
                        active_queries: active,
                        saved_queries: saved,
                        zoom_range: chart_zoom,
                        is_loading: loading,
                        show_offset_ticks: show_offset_ticks,
                    }
                }
            }
        }
        div { class: "analytics-right-panel",
            div { class: "view-toggle",
                button {
                    class: if !*date_mode.read() { "toggle-btn toggle-active" } else { "toggle-btn" },
                    onclick: move |_| date_mode.set(false),
                    "Offset"
                }
                button {
                    class: if *date_mode.read() { "toggle-btn toggle-active" } else { "toggle-btn" },
                    onclick: move |_| date_mode.set(true),
                    "Date"
                }
            }
            if !*date_mode.read() {
                h3 { "Offset Window" }
            div { class: "zoom-slider",
                div { class: "zoom-row",
                    span { class: "zoom-label", "End offset:" }
                    input {
                        r#type: "text",
                        class: "zoom-date-input",
                        placeholder: "latest",
                        value: "{end_offset_input}",
                        oninput: move |evt| {
                            end_offset_input.set(evt.value());
                            apply_window(end_offset_input, window_size, max_offset_cache, zoom_range);
                        },
                    }
                }
                div { class: "zoom-row",
                    span { class: "zoom-label", "Window:" }
                    input {
                        r#type: "text",
                        class: "zoom-date-input",
                        placeholder: "e.g. 100, 10K, 1M",
                        value: window_size.read().to_string(),
                        oninput: move |evt| {
                            if let Some(v) = crate::models::analytics::parse_offset(&evt.value())
                                && v > 0 {
                                    window_size.set(v);
                                    apply_window(end_offset_input, window_size, max_offset_cache, zoom_range);
                                }
                        },
                    }
                }
                div { class: "zoom-nav",
                    button {
                        class: "analytics-btn",
                        onclick: move |_| {
                            end_offset_input.set(String::new());
                            apply_window(end_offset_input, window_size, max_offset_cache, zoom_range);
                        },
                        "Latest"
                    }
                    button {
                        class: "analytics-btn",
                        onclick: move |_| {
                            let win = *window_size.read();
                            let current = end_offset_input.read().clone();
                            let max_off = max_offset_cache.read().unwrap_or(0);
                            let current_end = if current.is_empty() {
                                max_off
                            } else if let Some(v) = crate::models::analytics::parse_offset(&current) {
                                if v <= 0 { max_off + v } else { v }
                            } else {
                                max_off
                            };
                            let new_end = (current_end - win).max(win);
                            end_offset_input.set(new_end.to_string());
                            apply_window(end_offset_input, window_size, max_offset_cache, zoom_range);
                        },
                        "<< Prev"
                    }
                    button {
                        class: "analytics-btn",
                        onclick: move |_| {
                            let win = *window_size.read();
                            let current = end_offset_input.read().clone();
                            let max_off = max_offset_cache.read().unwrap_or(0);
                            let current_end = if current.is_empty() {
                                max_off
                            } else if let Some(v) = crate::models::analytics::parse_offset(&current) {
                                if v <= 0 { max_off + v } else { v }
                            } else {
                                max_off
                            };
                            let new_end = current_end + win;
                            if new_end >= max_off {
                                end_offset_input.set(String::new()); // back to "latest"
                            } else {
                                end_offset_input.set(new_end.to_string());
                            }
                            apply_window(end_offset_input, window_size, max_offset_cache, zoom_range);
                        },
                        "Next >>"
                    }
                }
                }
            }
            h3 { "Actions" }
            div { class: "analytics-buttons",
                button { class: "analytics-btn", onclick: on_refresh, "Refresh" }
                button { class: "analytics-btn", onclick: on_download_csv, "Download CSV" }
            }
            h4 { "Legend" }
            div { class: "analytics-legend",
                for (idx, query) in saved_queries.read().iter().enumerate() {
                    {
                        let active = active_queries.read();
                        if active.contains(&query.label) {
                            let color = crate::components::analytics_queries::color_for_index(idx);
                            let count = query_results
                                .read()
                                .get(&query.label)
                                .map(|d| {
                                    d.iter()
                                        .filter(|(o, _)| {
                                            zoom.is_none_or(|(min, max)| *o >= min && *o <= max)
                                        })
                                        .count()
                                })
                                .unwrap_or(0);
                            let is_loading = loading_queries.read().contains(&query.label);
                            let row_class = if count == 0 && !is_loading { "legend-row legend-empty" } else { "legend-row" };
                            rsx! {
                                div {
                                    class: row_class,
                                    span {
                                        class: "legend-dot",
                                        style: "background: {color};",
                                    }
                                    span { class: "legend-label", "{query.label}" }
                                    if is_loading {
                                        span { class: "legend-count", "(loading...)" }
                                    } else if count == 0 {
                                        span { class: "legend-no-data", "no data in window" }
                                    } else {
                                        span { class: "legend-count", "({count})" }
                                    }
                                }
                            }
                        } else {
                            rsx! {}
                        }
                    }
                }
            }
        }
    }
}

fn build_csv(
    results: &HashMap<String, Vec<(i64, f64)>>,
    dates: &[(i64, String)],
    active: &HashSet<String>,
    queries: &[AnalyticsQuery],
    zoom: Option<(i64, i64)>,
) -> String {
    let date_map: HashMap<i64, &str> = dates.iter().map(|(o, d)| (*o, d.as_str())).collect();

    let labels: Vec<&str> = queries
        .iter()
        .filter(|q| active.contains(&q.label))
        .map(|q| q.label.as_str())
        .collect();

    let mut all_offsets: Vec<i64> = results
        .iter()
        .filter(|(l, _)| active.contains(l.as_str()))
        .flat_map(|(_, data)| data.iter().map(|(o, _)| *o))
        .collect();
    all_offsets.sort();
    all_offsets.dedup();

    if let Some((min, max)) = zoom {
        all_offsets.retain(|o| *o >= min && *o <= max);
    }

    let mut lookup: HashMap<(&str, i64), f64> = HashMap::new();
    for (label, data) in results {
        if active.contains(label.as_str()) {
            for &(o, v) in data {
                lookup.insert((label.as_str(), o), v);
            }
        }
    }

    let mut csv = String::from("offset,effective_at");
    for label in &labels {
        csv.push(',');
        csv.push_str(label);
    }
    csv.push('\n');

    for &offset in &all_offsets {
        csv.push_str(&offset.to_string());
        csv.push(',');
        csv.push_str(date_map.get(&offset).unwrap_or(&""));
        for label in &labels {
            csv.push(',');
            if let Some(val) = lookup.get(&(*label, offset)) {
                if *val == val.floor() {
                    csv.push_str(&(*val as i64).to_string());
                } else {
                    csv.push_str(&format!("{val:.2}"));
                }
            }
        }
        csv.push('\n');
    }

    csv
}

fn download_csv(_csv: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, Url};
        let window = web_sys::window().unwrap();
        let document = window.document().unwrap();

        let array = js_sys::Array::new();
        array.push(&wasm_bindgen::JsValue::from_str(csv));
        let mut opts = BlobPropertyBag::new();
        opts.type_("text/csv");
        let blob = Blob::new_with_str_sequence_and_options(&array, &opts).unwrap();
        let url = Url::create_object_url_with_blob(&blob).unwrap();

        let a: HtmlAnchorElement = document.create_element("a").unwrap().dyn_into().unwrap();
        a.set_href(&url);
        a.set_download("analytics.csv");
        a.click();
        Url::revoke_object_url(&url).unwrap();
    }
}
