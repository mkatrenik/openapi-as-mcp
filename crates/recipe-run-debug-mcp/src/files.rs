//! Content shaping: slice, describe, search.
//!
//! The whole artefact may live in this process, but only a bounded, explicitly-labelled slice ever
//! reaches the agent. Every truncation says what was cut and how to get the rest.

use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(
    Debug, Clone, Copy, Default, Deserialize, Serialize, schemars::JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum ReadMode {
    /// First lines of the file.
    #[default]
    Head,
    /// Last lines of the file.
    Tail,
    /// A window starting at `offset` lines.
    Slice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Json,
    Jsonl,
    Yaml,
    Csv,
    Text,
}

impl Format {
    pub fn detect(name: &str, content: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".jsonl") || lower.ends_with(".ndjson") {
            return Self::Jsonl;
        }
        if lower.ends_with(".json") {
            // A ".json" file of one object per line is really JSONL; agents care about the difference.
            if serde_json::from_str::<Value>(content).is_err() && looks_like_jsonl(content) {
                return Self::Jsonl;
            }
            return Self::Json;
        }
        if lower.ends_with(".yaml") || lower.ends_with(".yml") {
            return Self::Yaml;
        }
        if lower.ends_with(".csv") || lower.ends_with(".tsv") {
            return Self::Csv;
        }
        Self::Text
    }
}

fn looks_like_jsonl(content: &str) -> bool {
    let mut lines = content.lines().filter(|l| !l.trim().is_empty());
    let first_two: Vec<&str> = lines.by_ref().take(2).collect();
    first_two.len() == 2
        && first_two
            .iter()
            .all(|l| serde_json::from_str::<Value>(l).is_ok())
}

/// A line-oriented window over the content, with byte-budget enforcement.
pub fn slice(
    content: &str,
    name: &str,
    mode: ReadMode,
    offset: usize,
    max_lines: usize,
    max_bytes: usize,
) -> Value {
    let format = Format::detect(name, content);
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    // A whole-file JSON document has no meaningful line window; pretty-print it instead.
    if matches!(format, Format::Json)
        && matches!(mode, ReadMode::Head)
        && offset == 0
        && let Ok(parsed) = serde_json::from_str::<Value>(content)
    {
        {
            let pretty = serde_json::to_string_pretty(&parsed).unwrap_or_default();
            let (text, truncated_bytes) = clamp_bytes(&pretty, max_bytes);
            return json!({
                "format": "json",
                "rendered_as": "pretty_printed_json",
                "total_bytes": content.len(),
                "returned_bytes": text.len(),
                "truncated": truncated_bytes,
                "truncation_note": truncated_bytes.then(|| format!(
                    "output capped at {max_bytes} bytes; raise limit_bytes or use search_run_file \
                     to find the part you need"
                )),
                "content": text,
            });
        }
    }

    let (start, end) = match mode {
        ReadMode::Head => (0, max_lines.min(total_lines)),
        ReadMode::Tail => (total_lines.saturating_sub(max_lines), total_lines),
        ReadMode::Slice => {
            let start = offset.min(total_lines);
            (start, (start + max_lines).min(total_lines))
        }
    };

    let window = lines[start..end].join("\n");
    let (text, truncated_bytes) = clamp_bytes(&window, max_bytes);
    let truncated_lines = end - start < total_lines;

    json!({
        "format": format,
        "total_lines": total_lines,
        "total_bytes": content.len(),
        "returned_lines": end - start,
        "line_range": [start + 1, end],
        "returned_bytes": text.len(),
        "truncated": truncated_lines || truncated_bytes,
        "truncation_note": (truncated_lines || truncated_bytes).then(|| format!(
            "showed lines {}-{} of {total_lines}{}; call again with mode=\"slice\" and \
             offset={end} for the next window, or use search_run_file to jump to a match",
            start + 1,
            end,
            if truncated_bytes { " (byte cap also hit)" } else { "" }
        )),
        "content": text,
    })
}

/// Structural summary — the cheap first look that avoids pulling content into context at all.
pub fn describe(content: &str, name: &str) -> Value {
    let format = Format::detect(name, content);
    let lines: Vec<&str> = content.lines().collect();

    let detail = match format {
        Format::Json => match serde_json::from_str::<Value>(content) {
            Ok(Value::Object(map)) => json!({
                "root": "object",
                "keys": map.keys().collect::<Vec<_>>(),
            }),
            Ok(Value::Array(items)) => json!({
                "root": "array",
                "length": items.len(),
                "first_item_keys": items.first().and_then(|v| v.as_object())
                    .map(|o| o.keys().collect::<Vec<_>>()),
            }),
            Ok(other) => json!({ "root": kind_of(&other) }),
            Err(err) => json!({ "parse_error": err.to_string() }),
        },
        Format::Jsonl => {
            let first = lines.iter().find(|l| !l.trim().is_empty());
            json!({
                "records": lines.iter().filter(|l| !l.trim().is_empty()).count(),
                "first_record_keys": first
                    .and_then(|l| serde_json::from_str::<Value>(l).ok())
                    .and_then(|v| v.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>())),
            })
        }
        Format::Csv => {
            let delimiter = if name.to_ascii_lowercase().ends_with(".tsv") {
                '\t'
            } else {
                ','
            };
            json!({
                "header": lines.first().map(|h| h.split(delimiter).collect::<Vec<_>>()),
                "data_rows": lines.len().saturating_sub(1),
                "first_rows": lines.iter().skip(1).take(3).collect::<Vec<_>>(),
            })
        }
        Format::Yaml => match serde_yaml::from_str::<serde_json::Value>(content) {
            Ok(Value::Object(map)) => {
                json!({ "root": "mapping", "keys": map.keys().collect::<Vec<_>>() })
            }
            Ok(other) => json!({ "root": kind_of(&other) }),
            Err(err) => json!({ "parse_error": err.to_string() }),
        },
        Format::Text => json!({
            "first_lines": lines.iter().take(3).collect::<Vec<_>>(),
        }),
    };

    json!({
        "name": name,
        "format": format,
        "bytes": content.len(),
        "lines": lines.len(),
        "detail": detail,
    })
}

/// Server-side grep. Returns matches with line numbers so a megabyte artefact never has to be
/// pulled into the agent's context to find one error string.
pub fn search(
    content: &str,
    pattern: &str,
    max_matches: usize,
    context_lines: usize,
    case_insensitive: bool,
) -> Result<Value, String> {
    let regex = RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| format!("invalid regex {pattern:?}: {e}"))?;

    let lines: Vec<&str> = content.lines().collect();
    let mut matches = Vec::new();
    let mut total = 0usize;

    for (idx, line) in lines.iter().enumerate() {
        if !regex.is_match(line) {
            continue;
        }
        total += 1;
        if matches.len() >= max_matches {
            continue;
        }
        let from = idx.saturating_sub(context_lines);
        let to = (idx + context_lines + 1).min(lines.len());
        matches.push(json!({
            "line": idx + 1,
            "text": clamp_bytes(line, 2_000).0,
            "context": (context_lines > 0).then(|| lines[from..to].join("\n")),
        }));
    }

    Ok(json!({
        "pattern": pattern,
        "total_matching_lines": total,
        "returned_matches": matches.len(),
        "truncated": total > matches.len(),
        "truncation_note": (total > matches.len()).then(|| format!(
            "{total} lines matched; showing the first {}. Raise max_matches or narrow the pattern.",
            matches.len()
        )),
        "matches": matches,
    }))
}

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Clamp to a byte budget on a char boundary. Returns the text and whether anything was cut.
fn clamp_bytes(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_string(), false);
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_and_tail_select_opposite_ends() {
        let content = (1..=10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");

        let head = slice(&content, "a.log", ReadMode::Head, 0, 3, 10_000);
        assert_eq!(head["content"], "line1\nline2\nline3");
        assert_eq!(head["line_range"], json!([1, 3]));
        assert_eq!(head["truncated"], json!(true));

        let tail = slice(&content, "a.log", ReadMode::Tail, 0, 2, 10_000);
        assert_eq!(tail["content"], "line9\nline10");
        assert_eq!(tail["line_range"], json!([9, 10]));
    }

    #[test]
    fn slice_offset_pages_through_the_file() {
        let content = (1..=10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let window = slice(&content, "a.log", ReadMode::Slice, 4, 2, 10_000);
        assert_eq!(window["content"], "line5\nline6");
        assert_eq!(window["line_range"], json!([5, 6]));
    }

    #[test]
    fn truncation_is_always_labelled_with_how_to_continue() {
        let content = (1..=100)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let head = slice(&content, "a.log", ReadMode::Head, 0, 5, 10_000);
        let note = head["truncation_note"].as_str().expect("note present");
        assert!(note.contains("of 100"), "{note}");
        assert!(note.contains("offset=5"), "{note}");
    }

    #[test]
    fn byte_cap_is_enforced_and_reported() {
        let content = "x".repeat(1_000);
        let out = slice(&content, "a.log", ReadMode::Head, 0, 10, 100);
        assert_eq!(out["returned_bytes"], json!(100));
        assert_eq!(out["truncated"], json!(true));
    }

    #[test]
    fn whole_json_documents_are_pretty_printed() {
        let out = slice(r#"{"b":1,"a":2}"#, "x.json", ReadMode::Head, 0, 100, 10_000);
        assert_eq!(out["rendered_as"], "pretty_printed_json");
        assert!(out["content"].as_str().unwrap().contains("\n  \"b\": 1"));
    }

    #[test]
    fn json_file_containing_records_per_line_is_detected_as_jsonl() {
        let content = "{\"a\":1}\n{\"a\":2}\n";
        assert_eq!(Format::detect("out.json", content), Format::Jsonl);
        assert_eq!(Format::detect("out.json", r#"{"a":1}"#), Format::Json);
    }

    #[test]
    fn describe_reports_structure_without_the_payload() {
        let out = describe(r#"{"entities":[],"meta":{"x":1}}"#, "out.json");
        assert_eq!(out["detail"]["keys"], json!(["entities", "meta"]));
        assert_eq!(out["format"], "json");

        let csv = describe("id,name\n1,a\n2,b\n", "out.csv");
        assert_eq!(csv["detail"]["header"], json!(["id", "name"]));
        assert_eq!(csv["detail"]["data_rows"], json!(2));

        let jsonl = describe("{\"a\":1}\n{\"a\":2}\n", "out.jsonl");
        assert_eq!(jsonl["detail"]["records"], json!(2));
        assert_eq!(jsonl["detail"]["first_record_keys"], json!(["a"]));
    }

    #[test]
    fn describe_surfaces_parse_errors_instead_of_failing() {
        let out = describe("{not json", "broken.json");
        assert!(out["detail"]["parse_error"].is_string());
    }

    #[test]
    fn search_returns_line_numbers_and_counts_all_matches() {
        let content = "ok\nERROR: boom\nok\nERROR: again\n";
        let out = search(content, "ERROR", 1, 0, false).unwrap();
        assert_eq!(out["total_matching_lines"], json!(2));
        assert_eq!(out["returned_matches"], json!(1));
        assert_eq!(out["matches"][0]["line"], json!(2));
        assert!(
            out["truncation_note"]
                .as_str()
                .unwrap()
                .contains("2 lines matched")
        );
    }

    #[test]
    fn search_context_lines_are_included_when_requested() {
        let out = search("a\nb\nMATCH\nd\ne\n", "MATCH", 5, 1, false).unwrap();
        assert_eq!(out["matches"][0]["context"], "b\nMATCH\nd");
    }

    #[test]
    fn invalid_regex_is_a_caller_error_not_a_panic() {
        assert!(search("x", "[unclosed", 1, 0, false).is_err());
    }

    #[test]
    fn clamp_respects_utf8_boundaries() {
        let (text, truncated) = clamp_bytes("héllo", 2);
        assert!(truncated);
        assert_eq!(text, "h");
    }
}
