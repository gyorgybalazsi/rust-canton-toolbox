use crate::models::analytics::{AnalyticsQuery, AnalyticsQueryFile};
use dioxus::prelude::*;

const SHARED_QUERIES_FILE: &str = "analytics-queries.toml";
const LOCAL_QUERIES_FILE: &str = "analytics-queries.local.toml";

fn queries_dir() -> String {
    // Try event-graph-ui/ subdirectory first, then current dir
    let candidates = ["event-graph-ui", ".", "../event-graph-ui"];
    for dir in &candidates {
        let path = format!("{dir}/{SHARED_QUERIES_FILE}");
        if std::path::Path::new(&path).exists() {
            return dir.to_string();
        }
    }
    ".".to_string()
}

fn shared_queries_path() -> String {
    format!("{}/{SHARED_QUERIES_FILE}", queries_dir())
}

fn local_queries_path() -> String {
    format!("{}/{LOCAL_QUERIES_FILE}", queries_dir())
}

fn load_queries_from_file(path: &str, shared: bool) -> Vec<AnalyticsQuery> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let file: AnalyticsQueryFile = match toml::from_str(&content) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("Failed to parse {path}: {e}");
            return Vec::new();
        }
    };
    file.queries
        .into_iter()
        .map(|mut q| {
            q.shared = shared;
            q
        })
        .collect()
}

fn save_local_queries(queries: &[AnalyticsQuery]) -> Result<(), ServerFnError> {
    let file = AnalyticsQueryFile {
        queries: queries.to_vec(),
    };
    let content = toml::to_string_pretty(&file)
        .map_err(|e| ServerFnError::new(format!("Failed to serialize queries: {e}")))?;
    std::fs::write(local_queries_path(), content)
        .map_err(|e| ServerFnError::new(format!("Failed to write local queries file: {e}")))?;
    Ok(())
}

/// Load all saved analytics queries (shared + local, merged).
/// Local queries override shared queries with the same label.
#[server]
pub async fn load_analytics_queries() -> Result<Vec<AnalyticsQuery>, ServerFnError> {
    let shared = load_queries_from_file(&shared_queries_path(), true);
    let local = load_queries_from_file(&local_queries_path(), false);

    let local_labels: std::collections::HashSet<String> =
        local.iter().map(|q| q.label.clone()).collect();
    let mut merged: Vec<AnalyticsQuery> = shared
        .into_iter()
        .filter(|q| !local_labels.contains(&q.label))
        .collect();
    merged.extend(local);
    Ok(merged)
}

/// Save a new query to the local TOML file.
/// Validates shape by running a limited version of the query.
#[server]
pub async fn save_analytics_query(
    label: String,
    cypher: String,
    min_time: Option<String>,
    max_time: Option<String>,
) -> Result<(), ServerFnError> {
    let pool = super::neo4j_pool::pool();

    // Validate by running the query with extreme offset bounds and LIMIT.
    // Pass $min_off/$max_off since queries may reference them.
    let validation_cypher = format!("CALL {{ {cypher} }} WITH offset, value LIMIT 10");
    let validation_result = pool
        .execute(
            neo4rs::query(&validation_cypher)
                .param("min_off", i64::MIN)
                .param("max_off", i64::MAX),
        )
        .await;

    // Try to validate, but don't block saving if validation fails.
    // Complex queries (nested CALL, ACS) may not validate easily.
    let validated = match validation_result {
        Ok(mut r) => {
            if let Ok(Some(row)) = r.next().await {
                let ok_offset = row.get::<i64>("offset").is_ok();
                let ok_value = row.get::<i64>("value").is_ok() || row.get::<f64>("value").is_ok();
                if !ok_offset || !ok_value {
                    tracing::warn!(
                        "Query validation: columns may not match expected (offset, value) shape"
                    );
                }
            }
            true
        }
        Err(_) => {
            // CALL wrapper failed, try fallback
            let fallback = format!("{cypher} LIMIT 10");
            match pool
                .execute(
                    neo4rs::query(&fallback)
                        .param("min_off", i64::MIN)
                        .param("max_off", i64::MAX),
                )
                .await
            {
                Ok(_) => true,
                Err(_) => {
                    // Both failed — save anyway, errors will show when query runs
                    tracing::warn!(
                        "Query validation skipped for '{label}' — could not validate shape"
                    );
                    false
                }
            }
        }
    };
    let _ = validated; // saved regardless

    let mut local = load_queries_from_file(&local_queries_path(), false);
    local.retain(|q| q.label != label);
    local.push(AnalyticsQuery {
        label,
        cypher,
        min_time,
        max_time,
        shared: false,
    });
    save_local_queries(&local)
}

/// Delete a query from the local TOML file.
#[server]
pub async fn delete_analytics_query(label: String) -> Result<(), ServerFnError> {
    let mut local = load_queries_from_file(&local_queries_path(), false);
    let before = local.len();
    local.retain(|q| q.label != label);
    if local.len() == before {
        return Err(ServerFnError::new(format!(
            "Query '{label}' not found in local queries (shared queries cannot be deleted)"
        )));
    }
    save_local_queries(&local)
}

/// Run a Cypher query and return (offset, value) tuples.
/// Filters results to [min_offset, max_offset] range server-side.
#[server]
pub async fn run_analytics_query(
    cypher: String,
    min_offset: Option<i64>,
    max_offset: Option<i64>,
) -> Result<Vec<(i64, f64)>, ServerFnError> {
    let pool = super::neo4j_pool::pool();

    // Always pass min_off and max_off as Neo4j query parameters.
    // Use extreme defaults when no bounds specified, so $min_off/$max_off in
    // queries always resolve to valid values (never null).
    let q = neo4rs::query(&cypher)
        .param("min_off", min_offset.unwrap_or(i64::MIN))
        .param("max_off", max_offset.unwrap_or(i64::MAX));
    let mut result = pool
        .execute(q)
        .await
        .map_err(|e| ServerFnError::new(format!("Analytics query failed: {e}")))?;

    let mut data: Vec<(i64, f64)> = Vec::new();
    while let Some(row) = result
        .next()
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to read analytics row: {e}")))?
    {
        let offset: i64 = match row.get("offset") {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Filter by offset bounds (queries return ORDER BY offset,
        // so we can skip early and break early)
        if let Some(min_o) = min_offset {
            if offset < min_o {
                continue;
            }
        }
        if let Some(max_o) = max_offset {
            if offset > max_o {
                break;
            }
        }
        // Neo4j count() returns i64. Try i64 first, then f64.
        let value: f64 = row
            .get::<i64>("value")
            .map(|v| v as f64)
            .or_else(|_| row.get::<f64>("value"))
            .unwrap_or(0.0);
        data.push((offset, value));
    }

    data.sort_by_key(|(offset, _)| *offset);

    Ok(data)
}

/// Fetch offset → effective_at mapping for all transactions.
#[server]
pub async fn get_offset_dates() -> Result<Vec<(i64, String)>, ServerFnError> {
    let pool = super::neo4j_pool::pool();
    let query = neo4rs::query(
        "MATCH (t:Transaction) RETURN t.offset AS offset, t.effective_at AS date ORDER BY t.offset",
    );
    let mut result = pool
        .execute(query)
        .await
        .map_err(|e| ServerFnError::new(format!("Offset dates query failed: {e}")))?;

    let mut dates: Vec<(i64, String)> = Vec::new();
    while let Some(row) = result
        .next()
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to read offset date row: {e}")))?
    {
        let offset: i64 = match row.get("offset") {
            Ok(v) => v,
            Err(_) => continue,
        };
        let date: String = row.get("date").unwrap_or_default();
        if !date.is_empty() {
            dates.push((offset, date));
        }
    }

    Ok(dates)
}

/// Get the maximum transaction offset in Neo4j.
#[server]
pub async fn get_max_offset() -> Result<Option<i64>, ServerFnError> {
    let pool = super::neo4j_pool::pool();
    let q = neo4rs::query("MATCH (t:Transaction) RETURN max(t.offset) AS max_off");
    let mut result = pool
        .execute(q)
        .await
        .map_err(|e| ServerFnError::new(format!("Max offset query failed: {e}")))?;
    if let Some(row) = result
        .next()
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to read max offset: {e}")))?
    {
        Ok(row.get::<i64>("max_off").ok())
    } else {
        Ok(None)
    }
}

/// Return the current server time as ISO 8601 string.
#[server]
pub async fn get_server_time() -> Result<String, ServerFnError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let secs = now % 60;
    let mins = (now / 60) % 60;
    let hours = (now / 3600) % 24;
    let mut days = (now / 86400) as i64;

    // Convert days since epoch to YYYY-MM-DD
    let mut year = 1970i64;
    let is_leap = |y: i64| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    loop {
        let dy = if is_leap(year) { 366 } else { 365 };
        if days < dy {
            break;
        }
        days -= dy;
        year += 1;
    }
    let days_in_month = [0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1i64;
    loop {
        let mut dm = days_in_month[month as usize];
        if month == 2 && is_leap(year) {
            dm += 1;
        }
        if days < dm {
            break;
        }
        days -= dm;
        month += 1;
    }
    let day = days + 1;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hours:02}:{mins:02}:{secs:02}Z"
    ))
}

/// Fetch all distinct template names from Created nodes.
#[server]
pub async fn get_template_names() -> Result<Vec<String>, ServerFnError> {
    let pool = super::neo4j_pool::pool();
    let query = neo4rs::query(
        "MATCH (c:Created) RETURN DISTINCT c.template_name AS template ORDER BY template",
    );
    let mut result = pool
        .execute(query)
        .await
        .map_err(|e| ServerFnError::new(format!("Template names query failed: {e}")))?;

    let mut names: Vec<String> = Vec::new();
    while let Some(row) = result
        .next()
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to read template name row: {e}")))?
    {
        let name: String = row.get("template").unwrap_or_default();
        if !name.is_empty() {
            names.push(name);
        }
    }

    Ok(names)
}

/// Fetch distinct choice names for a given template.
#[server]
pub async fn get_choice_names(template_name: String) -> Result<Vec<String>, ServerFnError> {
    let pool = super::neo4j_pool::pool();
    let query = neo4rs::query(
        "MATCH (x:Exercised)-[:TARGET]->(c:Created) \
         WHERE c.template_name = $template \
         RETURN DISTINCT x.choice_name AS choice ORDER BY choice",
    )
    .param("template", template_name.as_str());
    let mut result = pool
        .execute(query)
        .await
        .map_err(|e| ServerFnError::new(format!("Choice names query failed: {e}")))?;

    let mut names: Vec<String> = Vec::new();
    while let Some(row) = result
        .next()
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to read choice name row: {e}")))?
    {
        let name: String = row.get("choice").unwrap_or_default();
        if !name.is_empty() {
            names.push(name);
        }
    }

    Ok(names)
}
