use serde::{Deserialize, Serialize};

/// Parse an offset string that supports K/M suffixes.
/// Examples: "100", "100K", "2K", "3M", "-500", "-2K", "1.5M"
pub fn parse_offset(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (num_str, multiplier) = if s.ends_with('M') || s.ends_with('m') {
        (&s[..s.len() - 1], 1_000_000i64)
    } else if s.ends_with('K') || s.ends_with('k') {
        (&s[..s.len() - 1], 1_000i64)
    } else {
        (s, 1i64)
    };
    let num: f64 = num_str.parse().ok()?;
    Some((num * multiplier as f64) as i64)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsQuery {
    pub label: String,
    pub cypher: String,
    #[serde(default)]
    pub min_time: Option<String>,
    #[serde(default)]
    pub max_time: Option<String>,
    #[serde(skip)]
    pub shared: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalyticsQueryFile {
    #[serde(rename = "query")]
    pub queries: Vec<AnalyticsQuery>,
}
