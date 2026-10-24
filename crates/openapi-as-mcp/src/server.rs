//! The MCP surface over [`ToolSet`]: one MCP tool per OpenAPI operation.
//!
//! Unlike a hand-written server, the tool list is not known at compile time, so [`ServerHandler`]
//! is implemented directly instead of through the `#[tool]` macros: `list_tools` returns a vector
//! built at startup and `call_tool` dispatches by name.

use std::sync::Arc;

use openapi_core::config::Config;
use openapi_core::spec::{Api, Operation};
use openapi_core::toolset::ToolSet;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData as McpError,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};

#[derive(Debug)]
pub struct OpenApiMcp {
    tool_set: ToolSet,
    /// Built once from the tool set, since `list_tools` hands out the same list every time.
    tools: Vec<Tool>,
}

impl OpenApiMcp {
    /// Builds the server from already-loaded documents. Fails for the same reasons as
    /// [`ToolSet::new`].
    pub fn new(config: &Config, apis: Vec<Api>) -> anyhow::Result<Self> {
        let tool_set = ToolSet::new(config, apis)?;
        let tools = tool_set.operations().map(tool_for).collect();
        Ok(Self { tool_set, tools })
    }

    pub fn tools(&self) -> &[Tool] {
        &self.tools
    }

    pub fn base_urls(&self) -> Vec<&str> {
        self.tool_set.base_urls()
    }
}

fn tool_for(operation: &Operation) -> Tool {
    let read_only = operation.is_read_only();
    Tool::new(
        operation.name.clone(),
        operation.description.clone(),
        Arc::new(operation.input_schema.clone()),
    )
    .annotate(
        ToolAnnotations::new()
            .read_only(read_only)
            // DELETE removes something; PUT and PATCH overwrite in place. POST is the additive
            // one, so it is the only write method that is not flagged destructive.
            .destructive(matches!(
                operation.method.as_str(),
                "DELETE" | "PUT" | "PATCH"
            ))
            .idempotent(matches!(
                operation.method.as_str(),
                "GET" | "HEAD" | "OPTIONS" | "PUT" | "DELETE"
            ))
            .open_world(true),
    )
}

impl ServerHandler for OpenApiMcp {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.instructions = Some(self.tool_set.instructions().to_string());
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        // The whole set fits in one page: even a large spec is a few hundred tools, and paging
        // would only give clients a chance to stop halfway through.
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|tool| tool.name == name).cloned()
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let arguments = request.arguments.unwrap_or_default();
        // Everything the upstream API can do to us — a 404, a stale token, a bad argument — is a
        // *tool-level* error: the model should read the message and try again, not see an opaque
        // protocol failure.
        let result = match self.tool_set.call(&request.name, arguments).await {
            Ok(payload) => CallToolResult::structured(payload),
            Err(message) => CallToolResult::error(vec![ContentBlock::text(message)]),
        };
        Ok(result.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openapi_core::config::ConfigArgs;
    use openapi_core::spec::parse;

    const SPEC: &str = r#"{
      "openapi": "3.0.0",
      "info": {"title": "Recipes", "version": "1"},
      "servers": [{"url": "https://api.example.com/v1"}],
      "paths": {
        "/recipes": {"get": {"operationId": "listRecipes"},
                     "post": {"operationId": "createRecipe"}},
        "/recipes/{id}": {
          "delete": {"operationId": "deleteRecipe", "parameters": [
            {"name": "id", "in": "path", "schema": {"type": "string"}}]},
          "put": {"operationId": "replaceRecipe", "parameters": [
            {"name": "id", "in": "path", "schema": {"type": "string"}}]}
        }
      }
    }"#;

    #[test]
    fn annotations_describe_what_each_method_does() {
        let config = ConfigArgs {
            specs: vec!["unused.json".into()],
            // Empty means "no config file": the developer's own must not leak into a unit test.
            config: Some(String::new()),
            ..ConfigArgs::default()
        }
        .resolve()
        .expect("resolves");
        let server =
            OpenApiMcp::new(&config, vec![parse(SPEC, "test").expect("parses")]).expect("builds");
        let annotations = |name: &str| {
            server
                .get_tool(name)
                .expect("tool exists")
                .annotations
                .expect("annotated")
        };

        assert_eq!(annotations("listRecipes").read_only_hint, Some(true));
        assert_eq!(annotations("deleteRecipe").read_only_hint, Some(false));
        assert_eq!(annotations("deleteRecipe").destructive_hint, Some(true));
        assert_eq!(
            annotations("createRecipe").destructive_hint,
            Some(false),
            "POST is additive"
        );
        assert_eq!(annotations("replaceRecipe").idempotent_hint, Some(true));
    }
}
