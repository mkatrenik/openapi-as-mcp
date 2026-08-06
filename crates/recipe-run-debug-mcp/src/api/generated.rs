//! GENERATED FILE — do not edit.
//!
//! Produced by `cargo run --example generate_models` from the vendored OpenAPI specs in
//! `specs/`. Adapters and tolerance wrappers live in `src/api/model.rs`.
#![allow(clippy::all, dead_code, unused_imports)]

/// Error types.
pub mod error {
    /// Error from a `TryFrom` or `FromStr` implementation.
    pub struct ConversionError(::std::borrow::Cow<'static, str>);
    impl ::std::error::Error for ConversionError {}
    impl ::std::fmt::Display for ConversionError {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> Result<(), ::std::fmt::Error> {
            ::std::fmt::Display::fmt(&self.0, f)
        }
    }
    impl ::std::fmt::Debug for ConversionError {
        fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> Result<(), ::std::fmt::Error> {
            ::std::fmt::Debug::fmt(&self.0, f)
        }
    }
    impl From<&'static str> for ConversionError {
        fn from(value: &'static str) -> Self {
            Self(value.into())
        }
    }
    impl From<String> for ConversionError {
        fn from(value: String) -> Self {
            Self(value.into())
        }
    }
}
///`IngestionPlatformModels`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "IngestionPlatformModels",
///  "type": "object"
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
#[serde(transparent)]
pub struct IngestionPlatformModels(
    pub ::serde_json::Map<::std::string::String, ::serde_json::Value>,
);
impl ::std::ops::Deref for IngestionPlatformModels {
    type Target = ::serde_json::Map<::std::string::String, ::serde_json::Value>;
    fn deref(&self) -> &::serde_json::Map<::std::string::String, ::serde_json::Value> {
        &self.0
    }
}
impl ::std::convert::From<IngestionPlatformModels>
    for ::serde_json::Map<::std::string::String, ::serde_json::Value>
{
    fn from(value: IngestionPlatformModels) -> Self {
        value.0
    }
}
impl ::std::convert::From<::serde_json::Map<::std::string::String, ::serde_json::Value>>
    for IngestionPlatformModels
{
    fn from(value: ::serde_json::Map<::std::string::String, ::serde_json::Value>) -> Self {
        Self(value)
    }
}
///Schema that represents a task in a tasks overlay.
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "OverlayTaskOutput",
///  "description": "Schema that represents a task in a tasks overlay.",
///  "type": "object",
///  "required": [
///    "name"
///  ],
///  "properties": {
///    "config": {
///      "title": "Config",
///      "type": "object",
///      "additionalProperties": true
///    },
///    "name": {
///      "title": "Name",
///      "type": "string"
///    },
///    "requirement_names": {
///      "title": "Requirement Names",
///      "type": "array",
///      "items": {
///        "type": "string"
///      }
///    }
///  },
///  "additionalProperties": true
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
pub struct OverlayTaskOutput {
    #[serde(default, skip_serializing_if = "::serde_json::Map::is_empty")]
    pub config: ::serde_json::Map<::std::string::String, ::serde_json::Value>,
    pub name: ::std::string::String,
    #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
    pub requirement_names: ::std::vec::Vec<::std::string::String>,
}
///`PageRecipe`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "Page[Recipe]",
///  "type": "object",
///  "required": [
///    "items",
///    "page",
///    "pages",
///    "size",
///    "total"
///  ],
///  "properties": {
///    "items": {
///      "title": "Items",
///      "type": "array",
///      "items": {
///        "$ref": "#/definitions/Recipe"
///      }
///    },
///    "page": {
///      "title": "Page",
///      "type": "integer",
///      "minimum": 1.0
///    },
///    "pages": {
///      "title": "Pages",
///      "type": "integer",
///      "minimum": 0.0
///    },
///    "size": {
///      "title": "Size",
///      "type": "integer",
///      "minimum": 1.0
///    },
///    "total": {
///      "title": "Total",
///      "type": "integer",
///      "minimum": 0.0
///    }
///  },
///  "additionalProperties": true
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
pub struct PageRecipe {
    pub items: ::std::vec::Vec<Recipe>,
    pub page: ::std::num::NonZeroU64,
    pub pages: u64,
    pub size: ::std::num::NonZeroU64,
    pub total: u64,
}
///`PageRecipeRun`
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "Page[RecipeRun]",
///  "type": "object",
///  "required": [
///    "items",
///    "page",
///    "pages",
///    "size",
///    "total"
///  ],
///  "properties": {
///    "items": {
///      "title": "Items",
///      "type": "array",
///      "items": {
///        "$ref": "#/definitions/RecipeRun"
///      }
///    },
///    "page": {
///      "title": "Page",
///      "type": "integer",
///      "minimum": 1.0
///    },
///    "pages": {
///      "title": "Pages",
///      "type": "integer",
///      "minimum": 0.0
///    },
///    "size": {
///      "title": "Size",
///      "type": "integer",
///      "minimum": 1.0
///    },
///    "total": {
///      "title": "Total",
///      "type": "integer",
///      "minimum": 0.0
///    }
///  },
///  "additionalProperties": true
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
pub struct PageRecipeRun {
    pub items: ::std::vec::Vec<RecipeRun>,
    pub page: ::std::num::NonZeroU64,
    pub pages: u64,
    pub size: ::std::num::NonZeroU64,
    pub total: u64,
}
///Represents the different Kafka publication states.
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "PublicationState",
///  "description": "Represents the different Kafka publication states.",
///  "type": "string",
///  "enum": [
///    "unpublished",
///    "to_be_published",
///    "published",
///    "publication_failed"
///  ]
///}
/// ```
/// </details>
#[derive(
    ::serde::Deserialize,
    ::serde::Serialize,
    Clone,
    Copy,
    Debug,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
)]
pub enum PublicationState {
    #[serde(rename = "unpublished")]
    Unpublished,
    #[serde(rename = "to_be_published")]
    ToBePublished,
    #[serde(rename = "published")]
    Published,
    #[serde(rename = "publication_failed")]
    PublicationFailed,
}
impl ::std::fmt::Display for PublicationState {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        match *self {
            Self::Unpublished => f.write_str("unpublished"),
            Self::ToBePublished => f.write_str("to_be_published"),
            Self::Published => f.write_str("published"),
            Self::PublicationFailed => f.write_str("publication_failed"),
        }
    }
}
impl ::std::str::FromStr for PublicationState {
    type Err = self::error::ConversionError;
    fn from_str(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        match value {
            "unpublished" => Ok(Self::Unpublished),
            "to_be_published" => Ok(Self::ToBePublished),
            "published" => Ok(Self::Published),
            "publication_failed" => Ok(Self::PublicationFailed),
            _ => Err("invalid value".into()),
        }
    }
}
impl ::std::convert::TryFrom<&str> for PublicationState {
    type Error = self::error::ConversionError;
    fn try_from(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<&::std::string::String> for PublicationState {
    type Error = self::error::ConversionError;
    fn try_from(
        value: &::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<::std::string::String> for PublicationState {
    type Error = self::error::ConversionError;
    fn try_from(
        value: ::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
///Schema that represents a created Recipe object.
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "Recipe",
///  "description": "Schema that represents a created Recipe object.",
///  "type": "object",
///  "required": [
///    "allow_parallel_runs",
///    "created_at_utc",
///    "created_by",
///    "default_overlay",
///    "description",
///    "id",
///    "name",
///    "status",
///    "tasks",
///    "tasks_overlays",
///    "updated_at_utc",
///    "version"
///  ],
///  "properties": {
///    "allow_parallel_runs": {
///      "title": "Allow Parallel Runs",
///      "type": "boolean"
///    },
///    "created_at_utc": {
///      "title": "Created At Utc",
///      "type": "string",
///      "format": "date-time"
///    },
///    "created_by": {
///      "title": "Created By",
///      "type": "string"
///    },
///    "default_overlay": {
///      "title": "Default Overlay",
///      "anyOf": [
///        {
///          "type": "string"
///        },
///        {
///          "type": "null"
///        }
///      ]
///    },
///    "description": {
///      "title": "Description",
///      "type": "string"
///    },
///    "id": {
///      "title": "Id",
///      "type": "string",
///      "format": "uuid"
///    },
///    "name": {
///      "title": "Name",
///      "type": "string"
///    },
///    "recipe_metadata": {
///      "title": "Recipe Metadata",
///      "anyOf": [
///        {
///          "type": "object",
///          "additionalProperties": true
///        },
///        {
///          "type": "null"
///        }
///      ]
///    },
///    "status": {
///      "description": "One of the `RecipeStatus` values; kept as a string so an unrecognised value degrades gracefully. Parse with `RecipeStatus::from_wire`.",
///      "type": "string"
///    },
///    "tasks": {
///      "title": "Tasks",
///      "type": "array",
///      "items": {
///        "$ref": "#/definitions/RecipeTask"
///      }
///    },
///    "tasks_overlays": {
///      "title": "Tasks Overlays",
///      "type": "object",
///      "additionalProperties": {
///        "$ref": "#/definitions/TasksOverlayOutput"
///      }
///    },
///    "updated_at_utc": {
///      "title": "Updated At Utc",
///      "anyOf": [
///        {
///          "type": "string",
///          "format": "date-time"
///        },
///        {
///          "type": "null"
///        }
///      ]
///    },
///    "version": {
///      "title": "Version",
///      "type": "string"
///    }
///  },
///  "additionalProperties": true
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
pub struct Recipe {
    pub allow_parallel_runs: bool,
    pub created_at_utc: ::chrono::DateTime<::chrono::offset::Utc>,
    pub created_by: ::std::string::String,
    pub default_overlay: ::std::option::Option<::std::string::String>,
    pub description: ::std::string::String,
    pub id: ::uuid::Uuid,
    pub name: ::std::string::String,
    #[serde(default, skip_serializing_if = "::std::option::Option::is_none")]
    pub recipe_metadata:
        ::std::option::Option<::serde_json::Map<::std::string::String, ::serde_json::Value>>,
    ///One of the `RecipeStatus` values; kept as a string so an unrecognised value degrades gracefully. Parse with `RecipeStatus::from_wire`.
    pub status: ::std::string::String,
    pub tasks: ::std::vec::Vec<RecipeTask>,
    pub tasks_overlays: ::std::collections::HashMap<::std::string::String, TasksOverlayOutput>,
    pub updated_at_utc: ::std::option::Option<::chrono::DateTime<::chrono::offset::Utc>>,
    pub version: ::std::string::String,
}
///Data structure representing a Recipe Run.
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "RecipeRun",
///  "description": "Data structure representing a Recipe Run.",
///  "type": "object",
///  "required": [
///    "composite_run_id",
///    "created_at_utc",
///    "ended_at_utc",
///    "id",
///    "previous_recipe_run_id",
///    "recipe_id",
///    "recipe_name",
///    "recipe_run_id",
///    "recipe_version",
///    "started_at_utc",
///    "state",
///    "task_runs",
///    "updated_at_utc"
///  ],
///  "properties": {
///    "composite_run_id": {
///      "title": "Composite Run Id",
///      "type": "string"
///    },
///    "created_at_utc": {
///      "title": "Created At Utc",
///      "type": "string",
///      "format": "date-time"
///    },
///    "ended_at_utc": {
///      "title": "Ended At Utc",
///      "anyOf": [
///        {
///          "type": "string",
///          "format": "date-time"
///        },
///        {
///          "type": "null"
///        }
///      ]
///    },
///    "id": {
///      "title": "Id",
///      "type": "string"
///    },
///    "previous_recipe_run_id": {
///      "title": "Previous Recipe Run Id",
///      "anyOf": [
///        {
///          "type": "string"
///        },
///        {
///          "type": "null"
///        }
///      ]
///    },
///    "recipe_id": {
///      "title": "Recipe Id",
///      "type": "string"
///    },
///    "recipe_name": {
///      "title": "Recipe Name",
///      "type": "string"
///    },
///    "recipe_run_id": {
///      "title": "Recipe Run Id",
///      "type": "string"
///    },
///    "recipe_version": {
///      "title": "Recipe Version",
///      "type": "string"
///    },
///    "started_at_utc": {
///      "title": "Started At Utc",
///      "type": "string",
///      "format": "date-time"
///    },
///    "state": {
///      "description": "One of the `RecipeRunState` values; kept as a string so an unrecognised value degrades gracefully. Parse with `RecipeRunState::from_wire`.",
///      "type": "string"
///    },
///    "task_runs": {
///      "title": "Task Runs",
///      "type": "array",
///      "items": {
///        "$ref": "#/definitions/TaskRun"
///      }
///    },
///    "updated_at_utc": {
///      "title": "Updated At Utc",
///      "anyOf": [
///        {
///          "type": "string",
///          "format": "date-time"
///        },
///        {
///          "type": "null"
///        }
///      ]
///    }
///  },
///  "additionalProperties": true
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
pub struct RecipeRun {
    pub composite_run_id: ::std::string::String,
    pub created_at_utc: ::chrono::DateTime<::chrono::offset::Utc>,
    pub ended_at_utc: ::std::option::Option<::chrono::DateTime<::chrono::offset::Utc>>,
    pub id: ::std::string::String,
    pub previous_recipe_run_id: ::std::option::Option<::std::string::String>,
    pub recipe_id: ::std::string::String,
    pub recipe_name: ::std::string::String,
    pub recipe_run_id: ::std::string::String,
    pub recipe_version: ::std::string::String,
    pub started_at_utc: ::chrono::DateTime<::chrono::offset::Utc>,
    ///One of the `RecipeRunState` values; kept as a string so an unrecognised value degrades gracefully. Parse with `RecipeRunState::from_wire`.
    pub state: ::std::string::String,
    pub task_runs: ::std::vec::Vec<TaskRun>,
    pub updated_at_utc: ::std::option::Option<::chrono::DateTime<::chrono::offset::Utc>>,
}
/**Represents the different recipe run states, from the moment it is picked up from the Recipe Run
Kafka Topic (in progress) until all its tasks have finished executing.*/
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "RecipeRunState",
///  "description": "Represents the different recipe run states, from the moment it is picked up from the Recipe Run\nKafka Topic (in progress) until all its tasks have finished executing.",
///  "type": "string",
///  "enum": [
///    "in_progress",
///    "done",
///    "skipped",
///    "awaiting",
///    "failed"
///  ]
///}
/// ```
/// </details>
#[derive(
    ::serde::Deserialize,
    ::serde::Serialize,
    Clone,
    Copy,
    Debug,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
)]
pub enum RecipeRunState {
    #[serde(rename = "in_progress")]
    InProgress,
    #[serde(rename = "done")]
    Done,
    #[serde(rename = "skipped")]
    Skipped,
    #[serde(rename = "awaiting")]
    Awaiting,
    #[serde(rename = "failed")]
    Failed,
}
impl ::std::fmt::Display for RecipeRunState {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        match *self {
            Self::InProgress => f.write_str("in_progress"),
            Self::Done => f.write_str("done"),
            Self::Skipped => f.write_str("skipped"),
            Self::Awaiting => f.write_str("awaiting"),
            Self::Failed => f.write_str("failed"),
        }
    }
}
impl ::std::str::FromStr for RecipeRunState {
    type Err = self::error::ConversionError;
    fn from_str(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        match value {
            "in_progress" => Ok(Self::InProgress),
            "done" => Ok(Self::Done),
            "skipped" => Ok(Self::Skipped),
            "awaiting" => Ok(Self::Awaiting),
            "failed" => Ok(Self::Failed),
            _ => Err("invalid value".into()),
        }
    }
}
impl ::std::convert::TryFrom<&str> for RecipeRunState {
    type Error = self::error::ConversionError;
    fn try_from(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<&::std::string::String> for RecipeRunState {
    type Error = self::error::ConversionError;
    fn try_from(
        value: &::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<::std::string::String> for RecipeRunState {
    type Error = self::error::ConversionError;
    fn try_from(
        value: ::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
///Enum for known Recipe Statuses.
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "RecipeStatus",
///  "description": "Enum for known Recipe Statuses.",
///  "type": "string",
///  "enum": [
///    "active",
///    "inactive"
///  ]
///}
/// ```
/// </details>
#[derive(
    ::serde::Deserialize,
    ::serde::Serialize,
    Clone,
    Copy,
    Debug,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
)]
pub enum RecipeStatus {
    #[serde(rename = "active")]
    Active,
    #[serde(rename = "inactive")]
    Inactive,
}
impl ::std::fmt::Display for RecipeStatus {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        match *self {
            Self::Active => f.write_str("active"),
            Self::Inactive => f.write_str("inactive"),
        }
    }
}
impl ::std::str::FromStr for RecipeStatus {
    type Err = self::error::ConversionError;
    fn from_str(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        match value {
            "active" => Ok(Self::Active),
            "inactive" => Ok(Self::Inactive),
            _ => Err("invalid value".into()),
        }
    }
}
impl ::std::convert::TryFrom<&str> for RecipeStatus {
    type Error = self::error::ConversionError;
    fn try_from(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<&::std::string::String> for RecipeStatus {
    type Error = self::error::ConversionError;
    fn try_from(
        value: &::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<::std::string::String> for RecipeStatus {
    type Error = self::error::ConversionError;
    fn try_from(
        value: ::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
///Schema that represents a Task belonging to a Recipe.
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "RecipeTask",
///  "description": "Schema that represents a Task belonging to a Recipe.",
///  "type": "object",
///  "required": [
///    "agent_name",
///    "agent_version",
///    "id",
///    "name",
///    "requirement_ids",
///    "task_data_location"
///  ],
///  "properties": {
///    "agent_name": {
///      "title": "Agent Name",
///      "type": "string"
///    },
///    "agent_version": {
///      "title": "Agent Version",
///      "type": "string"
///    },
///    "computed_task_overlays": {
///      "title": "Computed Task Overlays",
///      "type": "object",
///      "additionalProperties": true
///    },
///    "config": {
///      "title": "Config",
///      "type": "object",
///      "additionalProperties": true
///    },
///    "id": {
///      "title": "Id",
///      "type": "string",
///      "format": "uuid"
///    },
///    "name": {
///      "title": "Name",
///      "type": "string"
///    },
///    "requirement_ids": {
///      "title": "Requirement Ids",
///      "anyOf": [
///        {
///          "type": "array",
///          "items": {
///            "type": "string",
///            "format": "uuid"
///          }
///        },
///        {
///          "type": "null"
///        }
///      ]
///    },
///    "requirement_names": {
///      "title": "Requirement Names",
///      "type": "array",
///      "items": {
///        "type": "string"
///      }
///    },
///    "task_data_location": {
///      "title": "Task Data Location",
///      "type": "string"
///    }
///  },
///  "additionalProperties": true
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
pub struct RecipeTask {
    pub agent_name: ::std::string::String,
    pub agent_version: ::std::string::String,
    #[serde(default, skip_serializing_if = "::serde_json::Map::is_empty")]
    pub computed_task_overlays: ::serde_json::Map<::std::string::String, ::serde_json::Value>,
    #[serde(default, skip_serializing_if = "::serde_json::Map::is_empty")]
    pub config: ::serde_json::Map<::std::string::String, ::serde_json::Value>,
    pub id: ::uuid::Uuid,
    pub name: ::std::string::String,
    pub requirement_ids: ::std::option::Option<::std::vec::Vec<::uuid::Uuid>>,
    #[serde(default, skip_serializing_if = "::std::vec::Vec::is_empty")]
    pub requirement_names: ::std::vec::Vec<::std::string::String>,
    pub task_data_location: ::std::string::String,
}
///Data structure representing a Task Run.
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "TaskRun",
///  "description": "Data structure representing a Task Run.",
///  "type": "object",
///  "required": [
///    "agent_name",
///    "agent_version",
///    "config_file_path",
///    "created_at_utc",
///    "data_files_path",
///    "ended_at_utc",
///    "id",
///    "last_modified_at_utc",
///    "latest_task_run_execution_id",
///    "name",
///    "publication_state",
///    "requirements",
///    "started_at_utc",
///    "state",
///    "task_id",
///    "task_run_executions",
///    "updated_at_utc"
///  ],
///  "properties": {
///    "agent_name": {
///      "title": "Agent Name",
///      "type": "string"
///    },
///    "agent_version": {
///      "title": "Agent Version",
///      "type": "string"
///    },
///    "config_file_path": {
///      "title": "Config File Path",
///      "type": "string"
///    },
///    "created_at_utc": {
///      "title": "Created At Utc",
///      "type": "string",
///      "format": "date-time"
///    },
///    "data_files_path": {
///      "title": "Data Files Path",
///      "type": "string"
///    },
///    "ended_at_utc": {
///      "title": "Ended At Utc",
///      "anyOf": [
///        {
///          "type": "string",
///          "format": "date-time"
///        },
///        {
///          "type": "null"
///        }
///      ]
///    },
///    "id": {
///      "title": "Id",
///      "type": "string"
///    },
///    "last_modified_at_utc": {
///      "title": "Last Modified At Utc",
///      "type": "string",
///      "format": "date-time"
///    },
///    "latest_task_run_execution_id": {
///      "title": "Latest Task Run Execution Id",
///      "type": "string"
///    },
///    "name": {
///      "title": "Name",
///      "type": "string"
///    },
///    "publication_state": {
///      "description": "One of the `PublicationState` values; kept as a string so an unrecognised value degrades gracefully. Parse with `PublicationState::from_wire`.",
///      "type": "string"
///    },
///    "requirements": {
///      "title": "Requirements",
///      "type": "array",
///      "items": {
///        "type": "string",
///        "format": "uuid"
///      }
///    },
///    "started_at_utc": {
///      "title": "Started At Utc",
///      "anyOf": [
///        {
///          "type": "string",
///          "format": "date-time"
///        },
///        {
///          "type": "null"
///        }
///      ]
///    },
///    "state": {
///      "description": "One of the `TaskRunState` values; kept as a string so an unrecognised value degrades gracefully. Parse with `TaskRunState::from_wire`.",
///      "type": "string"
///    },
///    "task_id": {
///      "title": "Task Id",
///      "type": "string"
///    },
///    "task_run_executions": {
///      "title": "Task Run Executions",
///      "type": "array",
///      "items": {
///        "$ref": "#/definitions/TaskRunExecution"
///      }
///    },
///    "updated_at_utc": {
///      "title": "Updated At Utc",
///      "anyOf": [
///        {
///          "type": "string",
///          "format": "date-time"
///        },
///        {
///          "type": "null"
///        }
///      ]
///    }
///  },
///  "additionalProperties": true
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
pub struct TaskRun {
    pub agent_name: ::std::string::String,
    pub agent_version: ::std::string::String,
    pub config_file_path: ::std::string::String,
    pub created_at_utc: ::chrono::DateTime<::chrono::offset::Utc>,
    pub data_files_path: ::std::string::String,
    pub ended_at_utc: ::std::option::Option<::chrono::DateTime<::chrono::offset::Utc>>,
    pub id: ::std::string::String,
    pub last_modified_at_utc: ::chrono::DateTime<::chrono::offset::Utc>,
    pub latest_task_run_execution_id: ::std::string::String,
    pub name: ::std::string::String,
    ///One of the `PublicationState` values; kept as a string so an unrecognised value degrades gracefully. Parse with `PublicationState::from_wire`.
    pub publication_state: ::std::string::String,
    pub requirements: ::std::vec::Vec<::uuid::Uuid>,
    pub started_at_utc: ::std::option::Option<::chrono::DateTime<::chrono::offset::Utc>>,
    ///One of the `TaskRunState` values; kept as a string so an unrecognised value degrades gracefully. Parse with `TaskRunState::from_wire`.
    pub state: ::std::string::String,
    pub task_id: ::std::string::String,
    pub task_run_executions: ::std::vec::Vec<TaskRunExecution>,
    pub updated_at_utc: ::std::option::Option<::chrono::DateTime<::chrono::offset::Utc>>,
}
///Data structure representing a Task Run Execution.
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "TaskRunExecution",
///  "description": "Data structure representing a Task Run Execution.",
///  "type": "object",
///  "required": [
///    "ended_at_utc",
///    "error_type",
///    "execution_state",
///    "id",
///    "reason",
///    "started_at_utc",
///    "task_run_id"
///  ],
///  "properties": {
///    "ended_at_utc": {
///      "title": "Ended At Utc",
///      "anyOf": [
///        {
///          "type": "string",
///          "format": "date-time"
///        },
///        {
///          "type": "null"
///        }
///      ]
///    },
///    "error_type": {
///      "title": "Error Type",
///      "anyOf": [
///        {
///          "type": "string"
///        },
///        {
///          "type": "null"
///        }
///      ]
///    },
///    "execution_state": {
///      "description": "One of the `TaskRunExecutionState` values; kept as a string so an unrecognised value degrades gracefully. Parse with `TaskRunExecutionState::from_wire`.",
///      "type": "string"
///    },
///    "id": {
///      "title": "Id",
///      "type": "string"
///    },
///    "reason": {
///      "title": "Reason",
///      "anyOf": [
///        {
///          "type": "string"
///        },
///        {
///          "type": "null"
///        }
///      ]
///    },
///    "started_at_utc": {
///      "title": "Started At Utc",
///      "anyOf": [
///        {
///          "type": "string",
///          "format": "date-time"
///        },
///        {
///          "type": "null"
///        }
///      ]
///    },
///    "task_run_id": {
///      "title": "Task Run Id",
///      "type": "string"
///    }
///  },
///  "additionalProperties": true
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
pub struct TaskRunExecution {
    pub ended_at_utc: ::std::option::Option<::chrono::DateTime<::chrono::offset::Utc>>,
    pub error_type: ::std::option::Option<::std::string::String>,
    ///One of the `TaskRunExecutionState` values; kept as a string so an unrecognised value degrades gracefully. Parse with `TaskRunExecutionState::from_wire`.
    pub execution_state: ::std::string::String,
    pub id: ::std::string::String,
    pub reason: ::std::option::Option<::std::string::String>,
    pub started_at_utc: ::std::option::Option<::chrono::DateTime<::chrono::offset::Utc>>,
    pub task_run_id: ::std::string::String,
}
/**Represents the different execution states.
The IN_PROGRESS state is not included as the service doesn't expect to ever receive a message
with such state.*/
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "TaskRunExecutionState",
///  "description": "Represents the different execution states.\nThe IN_PROGRESS state is not included as the service doesn't expect to ever receive a message\nwith such state.",
///  "type": "string",
///  "enum": [
///    "created",
///    "published",
///    "failed",
///    "skipped",
///    "done"
///  ]
///}
/// ```
/// </details>
#[derive(
    ::serde::Deserialize,
    ::serde::Serialize,
    Clone,
    Copy,
    Debug,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
)]
pub enum TaskRunExecutionState {
    #[serde(rename = "created")]
    Created,
    #[serde(rename = "published")]
    Published,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "skipped")]
    Skipped,
    #[serde(rename = "done")]
    Done,
}
impl ::std::fmt::Display for TaskRunExecutionState {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        match *self {
            Self::Created => f.write_str("created"),
            Self::Published => f.write_str("published"),
            Self::Failed => f.write_str("failed"),
            Self::Skipped => f.write_str("skipped"),
            Self::Done => f.write_str("done"),
        }
    }
}
impl ::std::str::FromStr for TaskRunExecutionState {
    type Err = self::error::ConversionError;
    fn from_str(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        match value {
            "created" => Ok(Self::Created),
            "published" => Ok(Self::Published),
            "failed" => Ok(Self::Failed),
            "skipped" => Ok(Self::Skipped),
            "done" => Ok(Self::Done),
            _ => Err("invalid value".into()),
        }
    }
}
impl ::std::convert::TryFrom<&str> for TaskRunExecutionState {
    type Error = self::error::ConversionError;
    fn try_from(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<&::std::string::String> for TaskRunExecutionState {
    type Error = self::error::ConversionError;
    fn try_from(
        value: &::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<::std::string::String> for TaskRunExecutionState {
    type Error = self::error::ConversionError;
    fn try_from(
        value: ::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
/**Represents the different task run states, from the moment a task is created until it's
finished executing.*/
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "TaskRunState",
///  "description": "Represents the different task run states, from the moment a task is created until it's\nfinished executing.",
///  "type": "string",
///  "enum": [
///    "created",
///    "published",
///    "in_progress",
///    "failed",
///    "aborted",
///    "skip_next",
///    "skip_all",
///    "skipped",
///    "max_attempts_exceeded",
///    "awaiting",
///    "done"
///  ]
///}
/// ```
/// </details>
#[derive(
    ::serde::Deserialize,
    ::serde::Serialize,
    Clone,
    Copy,
    Debug,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
)]
pub enum TaskRunState {
    #[serde(rename = "created")]
    Created,
    #[serde(rename = "published")]
    Published,
    #[serde(rename = "in_progress")]
    InProgress,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "aborted")]
    Aborted,
    #[serde(rename = "skip_next")]
    SkipNext,
    #[serde(rename = "skip_all")]
    SkipAll,
    #[serde(rename = "skipped")]
    Skipped,
    #[serde(rename = "max_attempts_exceeded")]
    MaxAttemptsExceeded,
    #[serde(rename = "awaiting")]
    Awaiting,
    #[serde(rename = "done")]
    Done,
}
impl ::std::fmt::Display for TaskRunState {
    fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        match *self {
            Self::Created => f.write_str("created"),
            Self::Published => f.write_str("published"),
            Self::InProgress => f.write_str("in_progress"),
            Self::Failed => f.write_str("failed"),
            Self::Aborted => f.write_str("aborted"),
            Self::SkipNext => f.write_str("skip_next"),
            Self::SkipAll => f.write_str("skip_all"),
            Self::Skipped => f.write_str("skipped"),
            Self::MaxAttemptsExceeded => f.write_str("max_attempts_exceeded"),
            Self::Awaiting => f.write_str("awaiting"),
            Self::Done => f.write_str("done"),
        }
    }
}
impl ::std::str::FromStr for TaskRunState {
    type Err = self::error::ConversionError;
    fn from_str(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        match value {
            "created" => Ok(Self::Created),
            "published" => Ok(Self::Published),
            "in_progress" => Ok(Self::InProgress),
            "failed" => Ok(Self::Failed),
            "aborted" => Ok(Self::Aborted),
            "skip_next" => Ok(Self::SkipNext),
            "skip_all" => Ok(Self::SkipAll),
            "skipped" => Ok(Self::Skipped),
            "max_attempts_exceeded" => Ok(Self::MaxAttemptsExceeded),
            "awaiting" => Ok(Self::Awaiting),
            "done" => Ok(Self::Done),
            _ => Err("invalid value".into()),
        }
    }
}
impl ::std::convert::TryFrom<&str> for TaskRunState {
    type Error = self::error::ConversionError;
    fn try_from(value: &str) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<&::std::string::String> for TaskRunState {
    type Error = self::error::ConversionError;
    fn try_from(
        value: &::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
impl ::std::convert::TryFrom<::std::string::String> for TaskRunState {
    type Error = self::error::ConversionError;
    fn try_from(
        value: ::std::string::String,
    ) -> ::std::result::Result<Self, self::error::ConversionError> {
        value.parse()
    }
}
///Schema that represents a tasks overlay.
///
/// <details><summary>JSON schema</summary>
///
/// ```json
///{
///  "title": "TasksOverlayOutput",
///  "description": "Schema that represents a tasks overlay.",
///  "type": "object",
///  "required": [
///    "tasks"
///  ],
///  "properties": {
///    "tasks": {
///      "title": "Tasks",
///      "type": "array",
///      "items": {
///        "$ref": "#/definitions/OverlayTaskOutput"
///      }
///    }
///  },
///  "additionalProperties": true
///}
/// ```
/// </details>
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, PartialEq)]
pub struct TasksOverlayOutput {
    pub tasks: ::std::vec::Vec<OverlayTaskOutput>,
}
