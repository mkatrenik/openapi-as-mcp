//! Gateway fixtures shared by the protocol and CLI end-to-end tests.
//!
//! Payloads are **spec-complete** on purpose: the models are generated from the vendored OpenAPI
//! specs, so a fixture missing a required field fails to deserialise exactly as a real truncated
//! response would.

use serde_json::{Value, json};

pub const RECIPE_ID: &str = "8f14e45f-ceea-467a-9c6b-000000000001";
pub const EXTRACT_TASK_ID: &str = "8f14e45f-ceea-467a-9c6b-0000000000a1";

pub fn page(items: Vec<Value>) -> Value {
    json!({ "items": items, "total": items.len(), "pages": 1, "page": 1, "size": 10 })
}

pub fn task_run(
    id: &str,
    name: &str,
    agent: &str,
    state: &str,
    started: &str,
    ended: &str,
    executions: Vec<Value>,
) -> Value {
    json!({
        "id": id,
        "name": name,
        "agent_name": agent,
        "agent_version": "2.0.0",
        "task_id": if name == "extract" { EXTRACT_TASK_ID } else { "8f14e45f-ceea-467a-9c6b-0000000000a2" },
        "data_files_path": format!("agents/{name}"),
        "config_file_path": format!("agents/{name}/config.yaml"),
        "started_at_utc": started,
        "ended_at_utc": ended,
        "state": state,
        "task_run_executions": executions,
        "created_at_utc": started,
        "updated_at_utc": null,
        "last_modified_at_utc": ended,
        "requirements": [],
        "latest_task_run_execution_id": "e-1",
        "publication_state": "unpublished"
    })
}

pub fn failed_run() -> Value {
    json!({
        "id": "row-1",
        "recipe_run_id": "run-1",
        "previous_recipe_run_id": null,
        "recipe_id": RECIPE_ID,
        "recipe_name": "pep-recipe",
        "recipe_version": "3",
        "started_at_utc": "2026-07-01T10:00:00Z",
        "ended_at_utc": "2026-07-01T10:05:00Z",
        "state": "failed",
        "created_at_utc": "2026-07-01T09:59:00Z",
        "updated_at_utc": null,
        "composite_run_id": "00000000-0000-0000-0000-000000000000",
        // Deliberately out of execution order: the tool must sort by started_at_utc.
        "task_runs": [
            task_run(
                "t-2", "validate", "validation-agent", "failed",
                "2026-07-01T10:03:00Z", "2026-07-01T10:05:00Z",
                vec![json!({
                    "id": "e-1",
                    "task_run_id": "t-2",
                    "execution_state": "failed",
                    "reason": "row 42: missing required field `name`",
                    "error_type": "ValidationError",
                    "started_at_utc": "2026-07-01T10:03:00Z",
                    "ended_at_utc": "2026-07-01T10:05:00Z"
                })],
            ),
            task_run(
                "t-1", "extract", "extraction-agent", "done",
                "2026-07-01T10:00:00Z", "2026-07-01T10:02:00Z", vec![],
            ),
        ]
    })
}

pub fn recipe() -> Value {
    json!({
        "id": RECIPE_ID,
        "name": "pep-recipe",
        "version": "3",
        "description": "PEP Recipe for Someplace S:XYZ99",
        "status": "active",
        "created_by": "someone",
        "created_at_utc": "2026-01-01T00:00:00Z",
        "updated_at_utc": null,
        "allow_parallel_runs": false,
        "default_overlay": null,
        "tasks_overlays": {},
        "tasks": [{
            "id": EXTRACT_TASK_ID,
            "name": "extract",
            "agent_name": "extraction-agent",
            "agent_version": "1.0.0",
            "task_data_location": "agents/extraction",
            "requirement_ids": [],
            "requirement_names": [],
            "computed_task_overlays": {},
            "config": { "output": { "data": "entities.json", "threshold": 5 } }
        }]
    })
}
