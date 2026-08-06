//! Minimal structural diff over JSON values, used to compare as-executed config against the
//! recipe's current config (and one run's config against another's).

use serde_json::{Value, json};

/// Leaf-level differences, reported as dotted paths. Returns an empty vec when equal.
pub fn diff(left: &Value, right: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    walk("", left, right, &mut out);
    out
}

fn walk(path: &str, left: &Value, right: &Value, out: &mut Vec<Value>) {
    match (left, right) {
        (Value::Object(l), Value::Object(r)) => {
            let mut keys: Vec<&String> = l.keys().chain(r.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                match (l.get(key), r.get(key)) {
                    (Some(lv), Some(rv)) => walk(&child, lv, rv, out),
                    (Some(lv), None) => out.push(change(&child, "removed", Some(lv), None)),
                    (None, Some(rv)) => out.push(change(&child, "added", None, Some(rv))),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(l), Value::Array(r)) if l.len() == r.len() => {
            for (idx, (lv, rv)) in l.iter().zip(r.iter()).enumerate() {
                walk(&format!("{path}[{idx}]"), lv, rv, out);
            }
        }
        (l, r) if l == r => {}
        (l, r) => out.push(change(path, "changed", Some(l), Some(r))),
    }
}

fn change(path: &str, kind: &str, left: Option<&Value>, right: Option<&Value>) -> Value {
    json!({
        "path": if path.is_empty() { "<root>" } else { path },
        "change": kind,
        "left": left,
        "right": right,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_values_produce_no_diff() {
        let v = json!({"a": 1, "b": {"c": [1, 2]}});
        assert!(diff(&v, &v).is_empty());
    }

    #[test]
    fn reports_added_removed_and_changed_leaves_with_paths() {
        let left = json!({"keep": 1, "gone": 2, "nested": {"x": 1}});
        let right = json!({"keep": 1, "fresh": 3, "nested": {"x": 2}});
        let mut changes = diff(&left, &right);
        changes.sort_by_key(|c| c["path"].as_str().unwrap_or_default().to_string());

        let summary: Vec<(String, String)> = changes
            .iter()
            .map(|c| {
                (
                    c["path"].as_str().unwrap().to_string(),
                    c["change"].as_str().unwrap().to_string(),
                )
            })
            .collect();

        assert_eq!(
            summary,
            vec![
                ("fresh".to_string(), "added".to_string()),
                ("gone".to_string(), "removed".to_string()),
                ("nested.x".to_string(), "changed".to_string()),
            ]
        );
    }

    #[test]
    fn arrays_of_equal_length_diff_element_wise() {
        let changes = diff(&json!({"a": [1, 2]}), &json!({"a": [1, 9]}));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["path"], "a[1]");
        assert_eq!(changes[0]["right"], json!(9));
    }

    #[test]
    fn arrays_of_different_length_are_a_single_change() {
        let changes = diff(&json!({"a": [1, 2]}), &json!({"a": [1]}));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["path"], "a");
    }
}
