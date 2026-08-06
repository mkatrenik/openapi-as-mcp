//! reciperun-aggregator client.
//!
//! Note the UI's discovery, preserved here: a single run is fetched through
//! `GET /recipe-runs?recipe_run_id=…`, **not** `GET /recipe-runs/{id}` — that path takes a
//! different id (the row `id`, not the `recipe_run_id`).

use std::sync::LazyLock;

use regex::Regex;

use crate::api::model::{PageRecipeRun, RecipeRun};
use crate::http::{ApiError, HttpClient};

/// The spec constrains `recipe_run_task_predicate` to `name:state`, with a closed set of states.
/// Validating here turns a server-side 422 into an actionable message.
static TASK_PREDICATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-zA-Z_-]+:(aborted|done|max_attempts_exceeded|skip_all|skip_next|skipped)$")
        .expect("valid regex")
});

pub const TASK_PREDICATE_STATES: &[&str] = &[
    "aborted",
    "done",
    "max_attempts_exceeded",
    "skip_all",
    "skip_next",
    "skipped",
];

pub fn task_predicate_is_valid(value: &str) -> bool {
    TASK_PREDICATE.is_match(value)
}

#[derive(Debug, Default, Clone)]
pub struct RunQuery {
    pub recipe_id: Option<String>,
    /// The aggregator filters by recipe name directly — no recipe-store lookup needed.
    pub recipe_name: Option<String>,
    pub recipe_run_id: Option<String>,
    pub composite_run_id: Option<String>,
    /// Wire values of `RecipeRunState`.
    pub states: Vec<String>,
    pub task_predicate: Option<String>,
    pub page: Option<u32>,
    pub size: Option<u32>,
}

impl RunQuery {
    fn to_params(&self) -> Vec<(&'static str, String)> {
        let mut params: Vec<(&'static str, String)> = Vec::new();
        if let Some(v) = &self.recipe_id {
            params.push(("recipe_id", v.clone()));
        }
        if let Some(v) = &self.recipe_name {
            params.push(("recipe_name", v.clone()));
        }
        if let Some(v) = &self.recipe_run_id {
            params.push(("recipe_run_id", v.clone()));
        }
        if let Some(v) = &self.composite_run_id {
            params.push(("composite_run_id", v.clone()));
        }
        if let Some(v) = &self.task_predicate {
            params.push(("recipe_run_task_predicate", v.clone()));
        }
        for state in &self.states {
            params.push(("state", state.clone()));
        }
        params.push(("page", self.page.unwrap_or(1).to_string()));
        params.push(("size", self.size.unwrap_or(10).to_string()));
        params
    }
}

pub struct Aggregator<'a> {
    http: &'a HttpClient,
}

impl<'a> Aggregator<'a> {
    pub fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    pub async fn list_runs(&self, query: &RunQuery) -> Result<PageRecipeRun, ApiError> {
        let url = format!("{}/recipe-runs", self.http.config().aggregator_prefix());
        self.http.get_json(&url, &query.to_params()).await
    }

    /// Fetch exactly one run by its `recipe_run_id`.
    pub async fn get_run(&self, recipe_run_id: &str) -> Result<RecipeRun, ApiError> {
        let page = self
            .list_runs(&RunQuery {
                recipe_run_id: Some(recipe_run_id.to_string()),
                size: Some(1),
                ..Default::default()
            })
            .await?;

        page.items
            .into_iter()
            .next()
            .ok_or_else(|| ApiError::NotFound {
                what: format!("recipe run {recipe_run_id}"),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pattern comes straight from the OpenAPI spec; these are the shapes it accepts.
    #[test]
    fn task_predicate_validation_matches_the_spec_pattern() {
        for good in ["extract:done", "my-task:skip_all", "some_task:aborted"] {
            assert!(task_predicate_is_valid(good), "{good} should be valid");
        }
        for bad in [
            "extract",             // no state
            "extract:in_progress", // state not in the allowed set
            "extract:done:extra",  // trailing junk
            "task 1:done",         // space is not allowed in the name
            "",
        ] {
            assert!(!task_predicate_is_valid(bad), "{bad} should be invalid");
        }
    }

    #[test]
    fn query_serialises_repeated_state_params() {
        let query = RunQuery {
            recipe_name: Some("pep".into()),
            states: vec!["failed".into(), "done".into()],
            ..Default::default()
        };
        let params = query.to_params();
        let states: Vec<&String> = params
            .iter()
            .filter(|(k, _)| *k == "state")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(states, vec!["failed", "done"]);
        assert!(params.contains(&("recipe_name", "pep".to_string())));
        // Paging is always explicit, so behaviour doesn't drift with server defaults.
        assert!(params.contains(&("page", "1".to_string())));
        assert!(params.contains(&("size", "10".to_string())));
    }
}
