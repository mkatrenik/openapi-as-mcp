//! Artefact path derivation — a port of the UI's `tasks/utils.ts` and `tasks/Files.tsx`.
//!
//! This is replicated backend convention, not a contract (the UI says so in a comment). Treat
//! every derived path as a *hypothesis* to be checked against the bucket, never as fact.

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::api::model::{Recipe, TaskRun};

/// Port of `isFilenameOrPath`: `/^(?!.*\.\.)(?!\/)[^\s]+\.[^\s/.]+$/`.
///
/// Rust's regex crate has no lookahead, so the two negative lookaheads become explicit guards.
static FILENAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^\s]+\.[^\s/.]+$").expect("valid regex"));

pub fn is_filename_or_path(value: &str) -> bool {
    if value.contains("..") || value.starts_with('/') {
        return false;
    }
    FILENAME.is_match(value)
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ArtefactFile {
    pub name: String,
    /// `gs://` form, for humans and for a future direct-GCS backend.
    pub gcs_uri: String,
    /// Bucket-less path — what `GET /gcs/file/{path}` expects.
    pub path: String,
    /// Browser link, matching the UI's `authenticated_url`.
    pub console_url: String,
    /// True when this path was derived from the recipe's `config.output` rather than observed.
    pub expected_from_config: bool,
}

/// Port of `createGCSFilesPaths`: `{bucket}/{data_files_path}/{recipe_run_id}/{name}`.
pub fn output_file(bucket: &str, task: &TaskRun, recipe_run_id: &str, name: &str) -> ArtefactFile {
    let path = format!("{}/{}/{}", task.data_files_path, recipe_run_id, name);
    ArtefactFile {
        name: name.to_string(),
        gcs_uri: format!("gs://{bucket}/{path}"),
        console_url: format!("https://storage.cloud.google.com/{bucket}/{path}"),
        path,
        expected_from_config: true,
    }
}

/// The as-executed config artefact: `{bucket}/{config_file_path}`, with no run id segment.
pub fn config_file(bucket: &str, task: &TaskRun) -> ArtefactFile {
    let path = task.config_file_path.clone();
    ArtefactFile {
        name: path.rsplit('/').next().unwrap_or("config.yaml").to_string(),
        gcs_uri: format!("gs://{bucket}/{path}"),
        console_url: format!("https://storage.cloud.google.com/{bucket}/{path}"),
        path,
        expected_from_config: false,
    }
}

/// Output file names for a task, read from the *recipe* task's `config.output` map, keeping values
/// that look like filenames — the UI's `Files.tsx` logic.
pub fn expected_output_names(recipe: &Recipe, task: &TaskRun) -> Vec<String> {
    let Some(recipe_task) = recipe
        .tasks
        .iter()
        .find(|t| t.id.to_string() == task.task_id)
    else {
        return Vec::new();
    };

    let Some(output) = recipe_task.config.get("output").and_then(|o| o.as_object()) else {
        return Vec::new();
    };

    let mut names: Vec<String> = output
        .values()
        .filter_map(|v| v.as_str())
        .filter(|v| is_filename_or_path(v))
        .map(str::to_string)
        .collect();

    names.sort();
    names.dedup();
    names
}

/// Split a `gs://bucket/path` URI into its parts.
pub fn parse_gcs_uri(uri: &str) -> Option<(String, String)> {
    let rest = uri.strip_prefix("gs://")?;
    let (bucket, path) = rest.split_once('/')?;
    if bucket.is_empty() || path.is_empty() {
        return None;
    }
    Some((bucket.to_string(), path.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use serde_json::json;

    /// The task run whose `task_id` matches the recipe task in the fixtures below.
    fn task_run() -> TaskRun {
        fixtures::task_run(json!({ "name": "test-task" }))
    }

    /// Mirrors `createGCSFilesPaths` expectations in the UI's tasks/utils.test.ts.
    #[test]
    fn output_path_matches_the_ui() {
        let file = output_file(
            "ca-gcp-agent-platform-artefacts",
            &task_run(),
            "run-123",
            "file.csv",
        );
        assert_eq!(file.path, "some/path/run-123/file.csv");
        assert_eq!(
            file.console_url,
            "https://storage.cloud.google.com/ca-gcp-agent-platform-artefacts/some/path/run-123/file.csv"
        );
        assert_eq!(
            file.gcs_uri,
            "gs://ca-gcp-agent-platform-artefacts/some/path/run-123/file.csv"
        );
    }

    #[test]
    fn config_path_has_no_run_id_segment() {
        let file = config_file("bucket", &task_run());
        assert_eq!(file.path, "config/path/config.yaml");
        assert_eq!(file.name, "config.yaml");
    }

    /// Same cases the UI regex accepts and rejects.
    #[test]
    fn filename_heuristic_matches_the_ui_regex() {
        for good in ["file.csv", "nested/dir/file.json", "a.b"] {
            assert!(is_filename_or_path(good), "{good} should be accepted");
        }
        for bad in [
            "no-extension",
            "/absolute/file.csv",
            "../escape/file.csv",
            "has space.csv",
            "trailing.dot.",
            "dir/",
        ] {
            assert!(!is_filename_or_path(bad), "{bad} should be rejected");
        }
    }

    #[test]
    fn expected_outputs_come_from_matching_recipe_task_only() {
        let recipe = fixtures::recipe(json!({
            "tasks": [
                fixtures::recipe_task_json(
                    fixtures::TASK_ID,
                    "test-task",
                    json!({
                        "output": {
                            "data": "entities.json",
                            "report": "report.csv",
                            "threshold": 12,
                            "mode": "not-a-file"
                        }
                    }),
                ),
                fixtures::recipe_task_json(
                    "00000000-0000-0000-0000-0000000000a2",
                    "other",
                    json!({ "output": { "x": "other.json" } }),
                ),
            ]
        }));

        let names = expected_output_names(&recipe, &task_run());
        assert_eq!(names, vec!["entities.json", "report.csv"]);
    }

    #[test]
    fn missing_output_key_yields_no_files_rather_than_an_error() {
        let recipe = fixtures::recipe(json!({
            "tasks": [fixtures::recipe_task_json(
                fixtures::TASK_ID,
                "test-task",
                json!({ "input": { "x": "in.json" } }),
            )]
        }));
        assert!(expected_output_names(&recipe, &task_run()).is_empty());
    }

    #[test]
    fn gcs_uri_roundtrip() {
        assert_eq!(
            parse_gcs_uri("gs://bucket/a/b/c.json"),
            Some(("bucket".into(), "a/b/c.json".into()))
        );
        assert_eq!(parse_gcs_uri("bucket/a"), None);
        assert_eq!(parse_gcs_uri("gs://bucket"), None);
    }
}
