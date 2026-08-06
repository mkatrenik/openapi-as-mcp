//! Adapters over the generated wire models.
//!
//! The structs themselves come from `generated.rs`, which is produced from the vendored OpenAPI
//! specs (`cargo run --example generate_models`). This module holds the things a spec cannot
//! express: the UI's failure-state classification, tolerant parsing of state strings, and the
//! "simple run" sentinel.

use crate::api::generated;

pub use generated::{
    PageRecipe, PageRecipeRun, PublicationState, Recipe, RecipeRun, RecipeRunState, RecipeStatus,
    TaskRun, TaskRunExecutionState, TaskRunState,
};

/// The all-zero composite id that marks a non-composite ("simple") run, as the UI's resume logic
/// detects it.
pub const SIMPLE_COMPOSITE_RUN_ID: &str = "00000000-0000-0000-0000-000000000000";

/// Task-run states the UI treats as failure (`taskIsFailed` in tasks/utils.ts). This is a smaller
/// set than "not done": `skipped`, `skip_next` and `skip_all` are deliberate, not failures.
pub const FAILED_TASK_STATES: &[&str] = &["failed", "max_attempts_exceeded", "aborted"];

/// Parse a wire string into a generated enum, tolerating values the vendored spec doesn't know.
///
/// State fields are generated as `String` on purpose (see `RELAXED_ENUM_FIELDS` in the generator):
/// a backend adding an enum member must not make a whole run unparseable in a debugging tool.
pub trait FromWire: Sized {
    fn from_wire(value: &str) -> Option<Self>;
}

macro_rules! impl_from_wire {
    ($ty:ty) => {
        impl FromWire for $ty {
            fn from_wire(value: &str) -> Option<Self> {
                value.parse().ok()
            }
        }
    };
}

impl_from_wire!(RecipeRunState);
impl_from_wire!(TaskRunState);
impl_from_wire!(TaskRunExecutionState);
impl_from_wire!(PublicationState);
impl_from_wire!(RecipeStatus);

impl RecipeRun {
    pub fn is_simple(&self) -> bool {
        self.composite_run_id == SIMPLE_COMPOSITE_RUN_ID
    }

    /// The run state as a known enum member, or `None` if the backend reported something the
    /// vendored spec doesn't list.
    pub fn known_state(&self) -> Option<RecipeRunState> {
        RecipeRunState::from_wire(&self.state)
    }

    pub fn is_failed(&self) -> bool {
        self.state == "failed"
    }
}

impl TaskRun {
    /// Port of `taskIsFailed`.
    pub fn is_failed(&self) -> bool {
        FAILED_TASK_STATES.contains(&self.state.as_str())
    }

    pub fn known_state(&self) -> Option<TaskRunState> {
        TaskRunState::from_wire(&self.state)
    }

    /// `agent-name@version`, the form used throughout the tool output.
    pub fn agent(&self) -> String {
        format!("{}@{}", self.agent_name, self.agent_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_run_json(state: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "t-1",
            "name": "extract",
            "agent_name": "extraction-agent",
            "agent_version": "1.0.0",
            "task_id": "task-1",
            "data_files_path": "agents/extraction",
            "config_file_path": "agents/extraction/config.yaml",
            "started_at_utc": "2026-07-01T10:00:00Z",
            "ended_at_utc": "2026-07-01T10:02:00Z",
            "state": state,
            "task_run_executions": [],
            "created_at_utc": "2026-07-01T09:59:00Z",
            "updated_at_utc": null,
            "last_modified_at_utc": "2026-07-01T10:02:00Z",
            "requirements": [],
            "latest_task_run_execution_id": "e-1",
            "publication_state": "unpublished"
        })
    }

    #[test]
    fn failure_state_set_matches_the_ui() {
        for state in FAILED_TASK_STATES {
            let task: TaskRun = serde_json::from_value(task_run_json(state)).unwrap();
            assert!(task.is_failed(), "{state} should be failed");
        }
        for state in [
            "done",
            "in_progress",
            "skipped",
            "skip_next",
            "skip_all",
            "awaiting",
        ] {
            let task: TaskRun = serde_json::from_value(task_run_json(state)).unwrap();
            assert!(!task.is_failed(), "{state} should not be failed");
        }
    }

    /// The whole point of generating state fields as strings: this must parse, not error.
    #[test]
    fn an_unknown_state_still_parses_and_is_reported_as_unknown() {
        let task: TaskRun = serde_json::from_value(task_run_json("quantum_superposition")).unwrap();
        assert_eq!(task.state, "quantum_superposition");
        assert_eq!(task.known_state(), None);
        assert!(!task.is_failed());
    }

    #[test]
    fn known_states_parse_into_the_generated_enum() {
        let task: TaskRun = serde_json::from_value(task_run_json("max_attempts_exceeded")).unwrap();
        assert_eq!(task.known_state(), Some(TaskRunState::MaxAttemptsExceeded));
    }

    #[test]
    fn unknown_response_fields_are_ignored_not_rejected() {
        let mut json = task_run_json("done");
        json["brand_new_backend_field"] = serde_json::json!(42);
        let task: TaskRun = serde_json::from_value(json).expect("tolerant parse");
        assert_eq!(task.name, "extract");
    }

    #[test]
    fn agent_is_rendered_as_name_at_version() {
        let task: TaskRun = serde_json::from_value(task_run_json("done")).unwrap();
        assert_eq!(task.agent(), "extraction-agent@1.0.0");
    }
}
