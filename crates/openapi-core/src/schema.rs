//! Operation → MCP tool input schema.
//!
//! One flat object: every path, query, header and cookie parameter becomes a top-level property,
//! and the request body becomes one more. Flat beats faithful here — an agent filling in
//! `{"recipe_id": "…", "page": 2}` does not care which of those rides in the path and which in the
//! query string, and nesting them under `path`/`query` objects only invites mistakes.
//!
//! Component schemas are **not** inlined. They are copied into `$defs` and the `$ref`s rewritten
//! to point there, which keeps recursive schemas (a tree node containing tree nodes) representable
//! and keeps the schema small when twenty properties share one definition.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::spec::{Body, Param};

const COMPONENT_PREFIX: &str = "#/components/schemas/";
/// Bound on `$ref`-following and on nesting depth. Deeper than any real document; present so a
/// self-referential non-schema `$ref` cannot hang the process at load time.
const MAX_DEPTH: usize = 64;

/// Builds the tool's input schema. Returns it alongside the property name chosen for the request
/// body, which the executor needs in order to find the body in the call's arguments.
pub fn build(
    root: &Value,
    params: &[Param],
    body: Option<&Body>,
) -> (Map<String, Value>, Option<String>) {
    let mut rewriter = Rewriter::new(root);
    let mut properties = Map::new();
    let mut required = Vec::new();

    for param in params {
        let mut schema = rewriter.rewrite(&param.schema, 0);
        // The parameter's own description is the useful one; a shared component schema's
        // description says what the type is, not what this parameter means.
        if let (Some(description), Some(object)) =
            (param.description.as_ref(), schema.as_object_mut())
        {
            object.insert("description".into(), Value::String(description.clone()));
        }
        annotate_location(&mut schema, param);

        if param.required {
            required.push(Value::String(param.name.clone()));
        }
        properties.insert(param.name.clone(), schema);
    }

    let body_property = body.map(|body| {
        let name = free_name(&properties);
        let mut schema = match &body.schema {
            Some(schema) => rewriter.rewrite(schema, 0),
            // A media type with no schema: send whatever string the caller supplies.
            None => Value::Object(Map::from_iter([(
                "type".to_string(),
                Value::String("string".into()),
            )])),
        };
        if let Some(object) = schema.as_object_mut() {
            object.insert(
                "description".into(),
                Value::String(format!("Request body ({}).", body.content_type)),
            );
        }
        if body.required {
            required.push(Value::String(name.clone()));
        }
        properties.insert(name.clone(), schema);
        name
    });

    let mut schema = Map::new();
    schema.insert("type".into(), Value::String("object".into()));
    schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), Value::Array(required));
    }
    // Closed on purpose: a misspelled argument should be a validation error the agent can see and
    // fix, not a parameter silently dropped from the request.
    schema.insert("additionalProperties".into(), Value::Bool(false));

    let defs = rewriter.into_defs();
    if !defs.is_empty() {
        schema.insert(
            "$defs".into(),
            Value::Object(defs.into_iter().collect::<Map<_, _>>()),
        );
    }

    (schema, body_property)
}

/// Says where a non-body parameter travels, but only for the surprising cases. Path and query are
/// the default expectation; a header or cookie parameter is worth calling out.
fn annotate_location(schema: &mut Value, param: &Param) {
    use crate::spec::Location;
    if !matches!(param.location, Location::Header | Location::Cookie) {
        return;
    }
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    let note = format!("Sent as a {} parameter.", param.location.as_str());
    match object.get_mut("description") {
        Some(Value::String(existing)) => {
            existing.push(' ');
            existing.push_str(&note);
        }
        _ => {
            object.insert("description".into(), Value::String(note));
        }
    }
}

/// Picks a property name for the request body that no parameter has already claimed.
fn free_name(properties: &Map<String, Value>) -> String {
    for candidate in ["body", "request_body", "requestBody_"] {
        if !properties.contains_key(candidate) {
            return candidate.to_string();
        }
    }
    let mut n = 2;
    loop {
        let candidate = format!("body_{n}");
        if !properties.contains_key(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Rewrites `$ref`s and collects the component schemas they reach.
struct Rewriter<'a> {
    root: &'a Value,
    defs: BTreeMap<String, Value>,
    /// Component names currently being expanded, so a recursive schema stops instead of looping.
    in_progress: BTreeSet<String>,
}

impl<'a> Rewriter<'a> {
    fn new(root: &'a Value) -> Self {
        Self {
            root,
            defs: BTreeMap::new(),
            in_progress: BTreeSet::new(),
        }
    }

    fn into_defs(self) -> BTreeMap<String, Value> {
        self.defs
    }

    fn rewrite(&mut self, value: &Value, depth: usize) -> Value {
        if depth > MAX_DEPTH {
            return Value::Object(Map::new());
        }

        match value {
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|item| self.rewrite(item, depth + 1))
                    .collect(),
            ),
            Value::Object(object) => {
                let mut out = Map::with_capacity(object.len());
                for (key, child) in object {
                    if key == "$ref"
                        && let Some(replacement) = self.rewrite_ref(child, depth)
                    {
                        match replacement {
                            // A component reference stays a reference, pointed at `$defs`.
                            Reference::Def(pointer) => {
                                out.insert("$ref".into(), Value::String(pointer));
                            }
                            // Anything else is inlined, and may bring its own keys along.
                            Reference::Inline(Value::Object(inlined)) => out.extend(inlined),
                            Reference::Inline(other) => return other,
                        }
                        continue;
                    }
                    out.insert(key.clone(), self.rewrite(child, depth + 1));
                }
                normalize(out)
            }
            other => other.clone(),
        }
    }

    fn rewrite_ref(&mut self, reference: &Value, depth: usize) -> Option<Reference> {
        let reference = reference.as_str()?;

        if let Some(raw_name) = reference.strip_prefix(COMPONENT_PREFIX) {
            let name = unescape_pointer(raw_name);
            self.ensure_def(&name, depth);
            return Some(Reference::Def(format!("#/$defs/{}", escape_pointer(&name))));
        }

        // A local ref to something other than a component schema — a parameter's inline schema in
        // another operation, say. Inline it; there is nowhere sensible to hoist it to.
        let pointer = reference.strip_prefix('#')?;
        let target = self.root.pointer(pointer)?.clone();
        Some(Reference::Inline(self.rewrite(&target, depth + 1)))
    }

    /// Copies a component schema into `$defs`, following whatever it references in turn.
    fn ensure_def(&mut self, name: &str, depth: usize) {
        if self.defs.contains_key(name) || self.in_progress.contains(name) {
            return;
        }
        let pointer = format!("/components/schemas/{}", escape_pointer(name));
        let Some(target) = self.root.pointer(&pointer).cloned() else {
            // A dangling reference: leave the `$ref` pointing at an absent `$defs` entry rather
            // than inventing a schema, so the gap is visible instead of silently permissive.
            return;
        };
        self.in_progress.insert(name.to_string());
        let rewritten = self.rewrite(&target, depth + 1);
        self.in_progress.remove(name);
        self.defs.insert(name.to_string(), rewritten);
    }
}

enum Reference {
    /// Rewritten to point into `$defs`.
    Def(String),
    /// Resolved and spliced in.
    Inline(Value),
}

/// OpenAPI 3.0 predates JSON Schema 2020-12 in two places agents actually trip over.
fn normalize(mut object: Map<String, Value>) -> Value {
    // `nullable: true` means "or null", which 2020-12 spells as a type union.
    if object.remove("nullable") == Some(Value::Bool(true))
        && let Some(Value::String(single)) = object.get("type").cloned()
    {
        object.insert(
            "type".into(),
            Value::Array(vec![Value::String(single), Value::String("null".into())]),
        );
    }

    // 3.0 spells exclusive bounds as a boolean flag on the inclusive one.
    for (flag, bound) in [
        ("exclusiveMinimum", "minimum"),
        ("exclusiveMaximum", "maximum"),
    ] {
        if object.get(flag) == Some(&Value::Bool(true)) {
            match object.remove(bound) {
                Some(value) => {
                    object.insert(flag.into(), value);
                }
                None => {
                    object.remove(flag);
                }
            }
        } else if object.get(flag) == Some(&Value::Bool(false)) {
            object.remove(flag);
        }
    }

    Value::Object(object)
}

fn escape_pointer(name: &str) -> String {
    name.replace('~', "~0").replace('/', "~1")
}

fn unescape_pointer(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Api, Location, parse};
    use serde_json::json;

    fn param(name: &str, location: Location, required: bool, schema: Value) -> Param {
        Param {
            name: name.into(),
            location,
            required,
            description: None,
            schema,
        }
    }

    fn api(spec: Value) -> Api {
        parse(&spec.to_string(), "test").expect("parses")
    }

    #[test]
    fn parameters_become_flat_properties_with_a_required_list() {
        let params = vec![
            param("id", Location::Path, true, json!({"type": "string"})),
            param("page", Location::Query, false, json!({"type": "integer"})),
        ];
        let (schema, body) = build(&Value::Null, &params, None);

        assert!(body.is_none());
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"]["id"]["type"], "string");
        assert_eq!(schema["properties"]["page"]["type"], "integer");
        assert_eq!(schema["required"], json!(["id"]));
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[test]
    fn header_parameters_say_so_in_their_description() {
        let params = vec![param(
            "X-Trace",
            Location::Header,
            false,
            json!({"type": "string"}),
        )];
        let (schema, _) = build(&Value::Null, &params, None);
        let description = schema["properties"]["X-Trace"]["description"]
            .as_str()
            .expect("description added");
        assert!(description.contains("header"), "got: {description}");
    }

    #[test]
    fn a_body_property_avoids_colliding_with_a_parameter_named_body() {
        let params = vec![param(
            "body",
            Location::Query,
            false,
            json!({"type": "string"}),
        )];
        let body = Body {
            content_type: "application/json".into(),
            required: true,
            schema: Some(json!({"type": "object"})),
        };
        let (schema, name) = build(&Value::Null, &params, Some(&body));

        assert_eq!(name.as_deref(), Some("request_body"));
        assert_eq!(schema["properties"]["body"]["type"], "string");
        assert_eq!(schema["properties"]["request_body"]["type"], "object");
        assert_eq!(schema["required"], json!(["request_body"]));
    }

    #[test]
    fn a_schemaless_media_type_takes_a_string() {
        let body = Body {
            content_type: "text/csv".into(),
            required: false,
            schema: None,
        };
        let (schema, name) = build(&Value::Null, &[], Some(&body));
        assert_eq!(schema["properties"][name.unwrap()]["type"], "string");
        assert!(schema.get("required").is_none());
    }

    #[test]
    fn component_refs_are_hoisted_into_defs_rather_than_inlined() {
        let api = api(json!({
            "paths": {"/r": {"post": {"operationId": "createR", "requestBody": {
                "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Recipe"}}}
            }}}},
            "components": {"schemas": {
                "Recipe": {"type": "object", "properties": {
                    "owner": {"$ref": "#/components/schemas/User"}
                }},
                "User": {"type": "object", "properties": {"name": {"type": "string"}}},
                "Unused": {"type": "object"}
            }}
        }));

        let schema = &api.operations[0].input_schema;
        let defs = schema["$defs"].as_object().expect("defs present");
        assert!(defs.contains_key("Recipe"), "the referenced schema");
        assert!(defs.contains_key("User"), "and what it reaches in turn");
        assert!(
            !defs.contains_key("Unused"),
            "only the transitive closure, or every tool carries the whole spec"
        );
        assert_eq!(
            defs["Recipe"]["properties"]["owner"]["$ref"],
            "#/$defs/User"
        );
    }

    #[test]
    fn a_recursive_component_schema_terminates() {
        let api = api(json!({
            "paths": {"/n": {"post": {"operationId": "n", "requestBody": {
                "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Node"}}}
            }}}},
            "components": {"schemas": {"Node": {"type": "object", "properties": {
                "child": {"$ref": "#/components/schemas/Node"}
            }}}}
        }));

        let defs = &api.operations[0].input_schema["$defs"];
        assert_eq!(defs["Node"]["properties"]["child"]["$ref"], "#/$defs/Node");
    }

    #[test]
    fn a_dangling_component_ref_does_not_invent_a_schema() {
        let api = api(json!({
            "paths": {"/r": {"post": {"operationId": "createR", "requestBody": {
                "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Gone"}}}
            }}}}
        }));
        let schema = &api.operations[0].input_schema;
        assert_eq!(schema["properties"]["body"]["$ref"], "#/$defs/Gone");
        assert!(schema.get("$defs").is_none());
    }

    #[test]
    fn openapi_30_nullable_becomes_a_type_union() {
        let params = vec![param(
            "q",
            Location::Query,
            false,
            json!({"type": "string", "nullable": true}),
        )];
        let (schema, _) = build(&Value::Null, &params, None);
        assert_eq!(schema["properties"]["q"]["type"], json!(["string", "null"]));
        assert!(schema["properties"]["q"].get("nullable").is_none());
    }

    #[test]
    fn openapi_30_exclusive_bounds_become_numeric() {
        let params = vec![param(
            "n",
            Location::Query,
            false,
            json!({"type": "integer", "minimum": 0, "exclusiveMinimum": true, "maximum": 9,
                   "exclusiveMaximum": false}),
        )];
        let (schema, _) = build(&Value::Null, &params, None);
        let n = &schema["properties"]["n"];
        assert_eq!(n["exclusiveMinimum"], 0);
        assert!(n.get("minimum").is_none());
        assert_eq!(n["maximum"], 9, "a false flag just goes away");
        assert!(n.get("exclusiveMaximum").is_none());
    }

    #[test]
    fn a_non_component_local_ref_is_inlined() {
        let api = api(json!({
            "paths": {
                "/a": {"get": {"operationId": "a", "parameters": [
                    {"name": "q", "in": "query",
                     "schema": {"$ref": "#/paths/~1b/get/parameters/0/schema"}}
                ]}},
                "/b": {"get": {"operationId": "b", "parameters": [
                    {"name": "q", "in": "query", "schema": {"type": "string", "maxLength": 3}}
                ]}}
            }
        }));

        let a = api.operations.iter().find(|o| o.name == "a").unwrap();
        assert_eq!(a.input_schema["properties"]["q"]["maxLength"], 3);
    }
}
