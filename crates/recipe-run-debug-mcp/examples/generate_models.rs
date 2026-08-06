//! Generates `src/api/generated.rs` from the vendored OpenAPI specs in `specs/`.
//!
//! Run with:  cargo run --example generate_models
//! Refresh the specs first with: ./specs/refresh.sh
//!
//! Why a hand-rolled converter: typify consumes draft-07-style JSON Schema (`definitions`,
//! `#/definitions/X` refs), while these are FastAPI OpenAPI **3.1** documents whose schemas live
//! under `components/schemas` and express nullability as `anyOf: [T, {"type": "null"}]`. The
//! conversion below is purely mechanical — move the schema bag, rewrite the ref prefix.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use schemars_0_8::schema::RootSchema;
use serde_json::{Map, Value, json};
use typify::{TypeSpace, TypeSpaceSettings};

/// Each spec contributes the schemas we actually consume. Anything not listed (and not reachable by
/// `$ref` from something listed) is skipped, so the generated file stays about our surface area.
const WANTED: &[(&str, &[&str])] = &[
    (
        "reciperun-aggregator",
        &[
            "RecipeRun",
            "TaskRun",
            "TaskRunExecution",
            "RecipeRunState",
            "TaskRunState",
            "TaskRunExecutionState",
            "PublicationState",
            "Page_RecipeRun_",
        ],
    ),
    (
        "recipe-store",
        &[
            "Recipe",
            "RecipeTask",
            "RecipeStatus",
            "TasksOverlayOutput",
            "OverlayTaskOutput",
            "Page_Recipe_",
        ],
    ),
];

/// Fields whose value is a spec enum, but which we deliberately generate as `String`.
///
/// A debugging tool is used exactly when the platform is behaving oddly, so a backend adding an
/// enum member must not make whole runs unparseable. The enums are still generated as types (they
/// are listed in `WANTED`) and `src/api/model.rs` parses these strings into them on demand, falling
/// back to the raw value. Format: `(definition, property)`.
const RELAXED_ENUM_FIELDS: &[(&str, &str)] = &[
    ("RecipeRun", "state"),
    ("TaskRun", "state"),
    ("TaskRun", "publication_state"),
    ("TaskRunExecution", "execution_state"),
    ("Recipe", "status"),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut definitions: Map<String, Value> = Map::new();

    for (spec_name, wanted) in WANTED {
        let path = root.join("specs").join(format!("{spec_name}.openapi.json"));
        let spec: Value = serde_json::from_str(&fs::read_to_string(&path)?)?;

        let schemas = spec
            .get("components")
            .and_then(|c| c.get("schemas"))
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{}: no components.schemas", path.display()))?;

        // Pull in each wanted schema plus everything it references, transitively.
        let mut queue: Vec<String> = wanted.iter().map(|s| s.to_string()).collect();
        while let Some(name) = queue.pop() {
            if definitions.contains_key(&name) {
                continue;
            }
            let schema = schemas
                .get(&name)
                .ok_or_else(|| format!("{}: missing schema {name}", path.display()))?;
            let converted = rewrite_refs(schema);
            collect_refs(&converted, &mut queue);
            let relaxed = relax_enum_fields(&name, rename_refs_in(converted));
            definitions.insert(rust_name(&name), allow_extra_properties(relaxed));
        }
    }

    let root_schema: RootSchema = serde_json::from_value(json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "IngestionPlatformModels",
        "type": "object",
        "definitions": definitions,
    }))?;

    let mut settings = TypeSpaceSettings::default();
    settings
        .with_derive("PartialEq".to_string())
        .with_struct_builder(false);

    let mut type_space = TypeSpace::new(&settings);
    type_space.add_root_schema(root_schema)?;

    let generated = format!(
        "//! GENERATED FILE — do not edit.\n\
         //!\n\
         //! Produced by `cargo run --example generate_models` from the vendored OpenAPI specs in\n\
         //! `specs/`. Adapters and tolerance wrappers live in `src/api/model.rs`.\n\
         #![allow(clippy::all, dead_code, unused_imports)]\n\n\
         {}\n",
        prettyplease::unparse(&syn::parse2(type_space.to_stream())?)
    );

    let out = root.join("src/api/generated.rs");
    fs::write(&out, generated)?;

    // prettyplease and rustfmt disagree on details, and `cargo fmt` would rewrite the file after
    // generation — which makes the "generated file is current" test fail on a clean tree. Format
    // with rustfmt here so both agree and the check is stable.
    let formatted = std::process::Command::new("rustfmt")
        .args(["--edition", "2024"])
        .arg(&out)
        .status()?;
    if !formatted.success() {
        return Err("rustfmt failed on the generated file".into());
    }
    eprintln!(
        "wrote {} ({} definitions)",
        rel(&out, &root),
        type_space.iter_types().count()
    );
    Ok(())
}

/// Force `additionalProperties: true` on every object schema.
///
/// The aggregator spec sets `additionalProperties: false`, and typify honours that with
/// `#[serde(deny_unknown_fields)]` — meaning a single field added by the backend would make every
/// run unparseable until this crate is regenerated. We deliberately diverge: strictness protects a
/// *producer*, and we are a consumer whose whole job is to still work when the platform is odd.
/// Unknown fields are then ignored by serde (not surfaced), which is the accepted cost.
fn allow_extra_properties(mut schema: Value) -> Value {
    if schema.get("type").and_then(Value::as_str) == Some("object")
        && schema.get("properties").is_some()
    {
        schema
            .as_object_mut()
            .expect("object schema")
            .insert("additionalProperties".to_string(), json!(true));
    }
    schema
}

/// Replace `$ref`s to enum schemas with a plain string type for the fields in
/// `RELAXED_ENUM_FIELDS`, so an unrecognised value degrades instead of failing the parse.
fn relax_enum_fields(definition: &str, mut schema: Value) -> Value {
    let relaxed: Vec<&str> = RELAXED_ENUM_FIELDS
        .iter()
        .filter(|(def, _)| *def == definition)
        .map(|(_, prop)| *prop)
        .collect();

    if relaxed.is_empty() {
        return schema;
    }

    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return schema;
    };

    for property in relaxed {
        if let Some(slot) = properties.get_mut(property) {
            let original = slot
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|r| r.rsplit('/').next())
                .unwrap_or("unknown")
                .to_string();
            *slot = json!({
                "type": "string",
                "description": format!(
                    "One of the `{original}` values; kept as a string so an unrecognised value \
                     degrades gracefully. Parse with `{original}::from_wire`."
                ),
            });
        }
    }

    schema
}

/// `#/components/schemas/X` → `#/definitions/X`.
fn rewrite_refs(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, val) in map {
                if key == "$ref"
                    && let Some(target) = val.as_str()
                {
                    let name = target.rsplit('/').next().unwrap_or(target);
                    out.insert(key.clone(), json!(format!("#/definitions/{name}")));
                    continue;
                }
                out.insert(key.clone(), rewrite_refs(val));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(rewrite_refs).collect()),
        other => other.clone(),
    }
}

/// Queue every `#/definitions/X` name found in a converted schema.
fn collect_refs(value: &Value, queue: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                if key == "$ref" {
                    if let Some(name) = val.as_str().and_then(|r| r.rsplit('/').next()) {
                        queue.push(original_name(name));
                    }
                    continue;
                }
                collect_refs(val, queue);
            }
        }
        Value::Array(items) => items.iter().for_each(|i| collect_refs(i, queue)),
        _ => {}
    }
}

/// Apply the same name cleanup to refs that `rust_name` applies to definitions.
fn rename_refs_in(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, val)| {
                    if key == "$ref"
                        && let Some(name) = val.as_str().and_then(|r| r.rsplit('/').next())
                    {
                        let renamed = rust_name(name);
                        return (key, json!(format!("#/definitions/{renamed}")));
                    }
                    (key, rename_refs_in(val))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(rename_refs_in).collect()),
        other => other,
    }
}

/// FastAPI names generic pages `Page_RecipeRun_`; give them idiomatic Rust names.
fn rust_name(name: &str) -> String {
    let cleaned = name.trim_end_matches('_');
    match cleaned.split_once('_') {
        Some(("Page", inner)) => format!("Page{inner}"),
        _ => cleaned.to_string(),
    }
}

/// Inverse of `rust_name`, used when queueing referenced schemas by their spec name.
fn original_name(rust: &str) -> String {
    let names: BTreeMap<&str, &str> = BTreeMap::from([
        ("PageRecipeRun", "Page_RecipeRun_"),
        ("PageRecipe", "Page_Recipe_"),
    ]);
    names
        .get(rust)
        .map(|s| s.to_string())
        .unwrap_or_else(|| rust.to_string())
}

fn rel(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
