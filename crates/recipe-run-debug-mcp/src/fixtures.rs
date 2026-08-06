//! Test fixtures built from JSON rather than struct literals.
//!
//! The models are generated from the OpenAPI specs, so a spec change adds struct fields and would
//! break every literal. Deserialising from a spec-complete JSON base means fixtures exercise the
//! same path as real responses, and only tests that care about a field mention it.

#![cfg(test)]

use serde_json::{Value, json};

use crate::api::model::{Recipe, RecipeRun, TaskRun};

/// Recursively overlay `patch` onto `base`.
fn merge(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                match base.get_mut(&key) {
                    Some(slot) => merge(slot, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (slot, patch) => *slot = patch,
    }
}

fn build<T: serde::de::DeserializeOwned>(mut base: Value, overrides: Value) -> T {
    merge(&mut base, overrides);
    serde_json::from_value(base.clone())
        .unwrap_or_else(|e| panic!("fixture does not match the generated model: {e}\n{base:#}"))
}

/// The recipe-task id shared by `task_run` and the recipe tasks built with `recipe_task_json`.
/// `Recipe.tasks[].id` is a UUID in the spec while `TaskRun.task_id` is a string, and the two are
/// matched by value — so fixtures must agree on it.
pub const TASK_ID: &str = "00000000-0000-0000-0000-0000000000a1";

pub fn task_run_json() -> Value {
    json!({
        "id": "00000000-0000-0000-0000-0000000000f1",
        "name": "extract",
        "agent_name": "extraction-agent",
        "agent_version": "1.0.0",
        "task_id": TASK_ID,
        "data_files_path": "some/path",
        "config_file_path": "config/path/config.yaml",
        "started_at_utc": null,
        "ended_at_utc": null,
        "state": "in_progress",
        "task_run_executions": [],
        "created_at_utc": "2026-01-01T00:00:00Z",
        "updated_at_utc": null,
        "last_modified_at_utc": "2026-01-01T00:00:00Z",
        "requirements": [],
        "latest_task_run_execution_id": "exec-1",
        "publication_state": "unpublished"
    })
}

pub fn task_run(overrides: Value) -> TaskRun {
    build(task_run_json(), overrides)
}

pub fn recipe_run_json() -> Value {
    json!({
        "id": "row-1",
        "recipe_run_id": "run-1",
        "previous_recipe_run_id": null,
        "recipe_id": "8f14e45f-ceea-467a-9c6b-000000000001",
        "recipe_name": "pep-recipe",
        "recipe_version": "1",
        "started_at_utc": "2026-01-01T00:00:00Z",
        "ended_at_utc": null,
        "state": "in_progress",
        "created_at_utc": "2026-01-01T00:00:00Z",
        "updated_at_utc": null,
        "composite_run_id": "11111111-1111-1111-1111-111111111111",
        "task_runs": []
    })
}

pub fn recipe_run(overrides: Value) -> RecipeRun {
    build(recipe_run_json(), overrides)
}

pub fn recipe_json() -> Value {
    json!({
        "id": "8f14e45f-ceea-467a-9c6b-000000000001",
        "name": "pep-recipe",
        "version": "1",
        "description": "",
        "status": "active",
        "created_by": "someone",
        "created_at_utc": "2026-01-01T00:00:00Z",
        "updated_at_utc": null,
        "tasks": [],
        "allow_parallel_runs": false,
        "default_overlay": null,
        "tasks_overlays": {}
    })
}

pub fn recipe(overrides: Value) -> Recipe {
    build(recipe_json(), overrides)
}

/// A recipe task, as embedded in `Recipe.tasks`.
pub fn recipe_task_json(id: &str, name: &str, config: Value) -> Value {
    json!({
        "id": id,
        "name": name,
        "config": config,
        "agent_name": "some-agent",
        "agent_version": "1.0.0",
        "task_data_location": "some/path",
        "requirement_ids": [],
        "requirement_names": [],
        "computed_task_overlays": {}
    })
}

/// An overlay with the given task names.
pub fn overlay_json(task_names: &[&str]) -> Value {
    json!({
        "tasks": task_names
            .iter()
            .map(|name| json!({ "name": name, "config": {}, "requirement_names": [] }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_are_merged_not_replaced() {
        let task = task_run(json!({ "name": "validate", "state": "failed" }));
        assert_eq!(task.name, "validate");
        assert_eq!(task.state, "failed");
        // Untouched fields keep their base values.
        assert_eq!(task.agent_name, "extraction-agent");
    }

    #[test]
    fn nested_overrides_merge_deeply() {
        let mut base = json!({ "a": { "b": 1, "c": 2 } });
        merge(&mut base, json!({ "a": { "c": 9 } }));
        assert_eq!(base, json!({ "a": { "b": 1, "c": 9 } }));
    }
}
