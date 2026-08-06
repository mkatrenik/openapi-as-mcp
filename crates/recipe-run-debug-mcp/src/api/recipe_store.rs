//! recipe-store client — recipes and their per-task configs.

use crate::api::model::{PageRecipe, Recipe};
use crate::http::{ApiError, HttpClient};

pub struct RecipeStore<'a> {
    http: &'a HttpClient,
}

impl<'a> RecipeStore<'a> {
    pub fn new(http: &'a HttpClient) -> Self {
        Self { http }
    }

    /// List recipes. The spec's free-text filter is `search` (with an optional `search_fields`);
    /// there is no `name` parameter, and unknown query params are silently ignored by the service.
    pub async fn list_recipes(
        &self,
        search: Option<&str>,
        page: u32,
        size: u32,
    ) -> Result<PageRecipe, ApiError> {
        let url = format!("{}/recipes", self.http.config().recipe_store_prefix());
        let mut params: Vec<(&str, String)> =
            vec![("page", page.to_string()), ("size", size.to_string())];
        if let Some(q) = search.filter(|q| !q.is_empty()) {
            params.push(("search", q.to_string()));
        }
        self.http.get_json(&url, &params).await
    }

    pub async fn get_recipe(&self, recipe_id: &str) -> Result<Recipe, ApiError> {
        let url = format!(
            "{}/recipes/{}",
            self.http.config().recipe_store_prefix(),
            urlencoding::encode(recipe_id)
        );
        self.http.get_json(&url, &[]).await
    }

    /// Exact-name lookup via the service's dedicated endpoint. Previously this paged `/recipes` and
    /// matched client-side, which quietly failed for any recipe outside the first page.
    pub async fn get_recipe_by_name(&self, recipe_name: &str) -> Result<Recipe, ApiError> {
        let url = format!(
            "{}/recipes/by-name/{}",
            self.http.config().recipe_store_prefix(),
            urlencoding::encode(recipe_name)
        );
        match self.http.get_json(&url, &[]).await {
            Err(ApiError::Upstream { status, .. }) if status.as_u16() == 404 => {
                Err(ApiError::NotFound {
                    what: format!("recipe named {recipe_name:?}"),
                })
            }
            other => other,
        }
    }
}
