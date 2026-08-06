//! Task-run ordering, failure classification and overlay identification — ports of the UI's
//! `tasks/utils.ts` and `RecipeDetailStore`.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

use crate::api::model::{Recipe, RecipeRun, TaskRun};

pub const VALIDATION_AGENT: &str = "validation-agent";

/// Port of `sortTasks`: by `started_at_utc` ascending, nulls last.
pub fn sort_tasks(task_runs: &[TaskRun]) -> Vec<TaskRun> {
    let mut sorted = task_runs.to_vec();
    sorted.sort_by(|a, b| match (a.started_at_utc, b.started_at_utc) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(x), Some(y)) => x.cmp(&y),
    });
    sorted
}

/// First failing task in execution order — the one worth reading logs for.
pub fn first_failed(sorted: &[TaskRun]) -> Option<&TaskRun> {
    sorted.iter().find(|t| t.is_failed())
}

/// Port of `isFailedOnValidation`: did the run fail specifically on the validation agent?
pub fn is_failed_on_validation(sorted: &[TaskRun]) -> bool {
    first_failed(sorted).is_some_and(|t| t.agent_name == VALIDATION_AGENT)
}

/// Port of `RecipeDetailStore.runOverlayName`: the overlay whose task-name set is identical to the
/// run's (symmetric difference empty).
pub fn overlay_name(recipe: &Recipe, run: &RecipeRun) -> Option<String> {
    let run_names: BTreeSet<&str> = run.task_runs.iter().map(|t| t.name.as_str()).collect();

    recipe
        .tasks_overlays
        .iter()
        .find(|(_, overlay)| {
            let overlay_names: BTreeSet<&str> =
                overlay.tasks.iter().map(|t| t.name.as_str()).collect();
            overlay_names == run_names
        })
        .map(|(name, _)| name.clone())
}

static SOURCE_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"S:[A-Z0-9]+").expect("valid regex"));

/// Port of `RecipeDetailStore.sourceId` — the only place a source id is available.
pub fn source_id(recipe: &Recipe) -> Option<String> {
    SOURCE_ID
        .find(&recipe.description)
        .map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use serde_json::json;

    fn task(name: &str, started: Option<&str>, state: &str, agent: &str) -> TaskRun {
        fixtures::task_run(json!({
            "name": name,
            "started_at_utc": started,
            "state": state,
            "agent_name": agent,
        }))
    }

    fn run(tasks: Vec<TaskRun>) -> RecipeRun {
        fixtures::recipe_run(json!({
            "state": "failed",
            "task_runs": tasks.iter().map(|t| serde_json::to_value(t).unwrap()).collect::<Vec<_>>(),
        }))
    }

    fn recipe(overlays: Vec<(&str, Vec<&str>)>, description: &str) -> Recipe {
        let overlays: serde_json::Map<String, serde_json::Value> = overlays
            .into_iter()
            .map(|(name, tasks)| (name.to_string(), fixtures::overlay_json(&tasks)))
            .collect();
        fixtures::recipe(json!({
            "description": description,
            "tasks_overlays": overlays,
        }))
    }

    /// Mirrors the UI's sortTasks test: ascending, nulls last.
    #[test]
    fn sorts_by_start_time_with_nulls_last() {
        let tasks = vec![
            task("c", None, "done", "a"),
            task("b", Some("2024-01-02T00:00:00Z"), "done", "a"),
            task("a", Some("2024-01-01T00:00:00Z"), "done", "a"),
        ];
        let names: Vec<String> = sort_tasks(&tasks).into_iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn validation_failure_requires_the_first_failure_to_be_the_validation_agent() {
        let validation_first = vec![
            task(
                "a",
                Some("2024-01-01T00:00:00Z"),
                "failed",
                VALIDATION_AGENT,
            ),
            task("b", Some("2024-01-02T00:00:00Z"), "failed", "other-agent"),
        ];
        assert!(is_failed_on_validation(&sort_tasks(&validation_first)));

        let other_first = vec![
            task("a", Some("2024-01-01T00:00:00Z"), "failed", "other-agent"),
            task(
                "b",
                Some("2024-01-02T00:00:00Z"),
                "failed",
                VALIDATION_AGENT,
            ),
        ];
        assert!(!is_failed_on_validation(&sort_tasks(&other_first)));

        assert!(!is_failed_on_validation(&[]));
    }

    /// `skipped` and the `skip_*` states are deliberate, not failures — the spec has 11 task states
    /// where the UI's filter offers only a handful.
    #[test]
    fn only_the_three_ui_failure_states_count_as_failed() {
        for state in ["failed", "max_attempts_exceeded", "aborted"] {
            let tasks = vec![task("a", Some("2024-01-01T00:00:00Z"), state, "agent")];
            assert!(
                first_failed(&tasks).is_some(),
                "{state} should be a failure"
            );
        }
        for state in [
            "skipped",
            "skip_next",
            "skip_all",
            "created",
            "published",
            "done",
        ] {
            let tasks = vec![task("a", Some("2024-01-01T00:00:00Z"), state, "agent")];
            assert!(
                first_failed(&tasks).is_none(),
                "{state} should not be a failure"
            );
        }
    }

    #[test]
    fn overlay_matches_only_on_an_identical_task_name_set() {
        let r = run(vec![
            task("extract", None, "done", "a"),
            task("validate", None, "done", "a"),
        ]);

        let exact = recipe(
            vec![
                ("pep", vec!["extract", "validate"]),
                ("sanction", vec!["extract"]),
            ],
            "",
        );
        assert_eq!(overlay_name(&exact, &r), Some("pep".to_string()));

        // A superset must not match — the UI uses symmetric difference, not subset.
        let superset = recipe(vec![("pep", vec!["extract", "validate", "publish"])], "");
        assert_eq!(overlay_name(&superset, &r), None);
    }

    #[test]
    fn source_id_is_extracted_from_the_description() {
        let r = recipe(vec![], "PEP Recipe for Some Country S:ABC123");
        assert_eq!(source_id(&r), Some("S:ABC123".to_string()));
        assert_eq!(source_id(&recipe(vec![], "no source here")), None);
    }
}
