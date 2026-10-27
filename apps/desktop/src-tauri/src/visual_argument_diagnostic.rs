// SPDX-License-Identifier: MPL-2.0
//! Fixed metadata only; never format a serde/schema error or model value.
use mewu_core::{ArgumentDiagnostic, ArgumentDiagnosticReason as R, ArgumentExpectedType};
use serde_json::{json, Value};

// Value collapses duplicate keys. Before using its schema projection to
// explain a typed failure, prove that projection did not lose a duplicate.
// This check is diagnostic-only and never changes parser admission.
struct UniqueKeys(bool);
impl<'de> serde::Deserialize<'de> for UniqueKeys {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Check;
        impl<'de> serde::de::Visitor<'de> for Check {
            type Value = UniqueKeys;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("JSON value")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                let mut keys = std::collections::HashSet::new();
                let mut unique = true;
                while let Some(key) = map.next_key::<String>()? {
                    unique &= keys.insert(key);
                    unique &= map.next_value::<UniqueKeys>()?.0;
                }
                Ok(UniqueKeys(unique))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut unique = true;
                while let Some(value) = seq.next_element::<UniqueKeys>()? {
                    unique &= value.0;
                }
                Ok(UniqueKeys(unique))
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(UniqueKeys(true))
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(UniqueKeys(true))
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(UniqueKeys(true))
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(UniqueKeys(true))
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(UniqueKeys(true))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueKeys(true))
            }
        }
        deserializer.deserialize_any(Check)
    }
}

#[derive(Debug)]
pub(super) struct Rejection {
    pub message: String,
    pub diagnostic: Option<ArgumentDiagnostic>,
}
impl Rejection {
    pub fn field(message: impl Into<String>, path: impl Into<String>, reason: R) -> Self {
        Self {
            message: message.into(),
            diagnostic: ArgumentDiagnostic::new(path.into(), reason, None),
        }
    }
    pub fn diagnosed(message: impl Into<String>, diagnostic: Option<ArgumentDiagnostic>) -> Self {
        Self {
            message: message.into(),
            diagnostic,
        }
    }
}
impl From<String> for Rejection {
    fn from(message: String) -> Self {
        Self {
            message,
            diagnostic: None,
        }
    }
}
impl From<&str> for Rejection {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

fn known(field: &str) -> bool {
    matches!(
        field,
        "groups"
            | "target"
            | "objects"
            | "coordinateSpace"
            | "styleUnit"
            | "kind"
            | "color"
            | "strokeWidth"
            | "points"
            | "x"
            | "y"
            | "text"
            | "fontSize"
            | "anchor"
            | "table"
            | "version"
            | "header"
            | "rows"
            | "align"
            | "tex"
    )
}

fn project(error: &jsonschema::ValidationError<'_>, prefix: &str) -> Option<ArgumentDiagnostic> {
    use jsonschema::{error::ValidationErrorKind as K, paths::LocationSegment};
    // There are two real unions: groups select target, objects select kind.
    // Prefer the branch whose selector actually matched. If none matched,
    // report an actual selector failure instead of another branch's missing field.
    fn selector(error: &jsonschema::ValidationError<'_>) -> bool {
        matches!(error.kind(), K::Constant { .. } | K::Enum { .. })
            && matches!(error.instance_path().segments().last(), Some(LocationSegment::Property(field)) if field == "target" || field == "kind")
    }
    if let K::OneOfNotValid { context } = error.kind() {
        if let Some(branch) = context
            .iter()
            .find(|errors| !errors.is_empty() && !errors.iter().any(selector))
        {
            // A primitive type error is more actionable than the accompanying
            // const failure (for example string "1" instead of integer 1).
            let failure = branch.iter().find(|error| matches!(error.kind(), K::Type { .. }))
                .unwrap_or(&branch[0]);
            return project(failure, prefix);
        }
        if let Some(failure) = context.iter().flatten().find(|error| selector(error)) {
            return project(failure, prefix);
        }
        if let Some(branch) = context
            .iter()
            .filter(|errors| !errors.is_empty())
            .min_by_key(|errors| errors.len())
        {
            return project(&branch[0], prefix);
        }
    }
    let mut path = prefix.to_owned();
    for segment in error.instance_path().segments() {
        match segment {
            LocationSegment::Property(field) if known(&field) => {
                path.push('/');
                path.push_str(&field);
            }
            LocationSegment::Index(index) if index < 199 => {
                path.push('/');
                path.push_str(&index.to_string());
            }
            _ => {
                path = prefix.into();
                break;
            }
        }
    }
    if let K::Required { property } = error.kind() {
        if let Some(field) = property.as_str().filter(|field| known(field)) {
            path.push('/');
            path.push_str(field);
        }
    }
    let reason = match error.kind().keyword() {
        "required" => R::Required,
        "type" => R::Type,
        "additionalProperties" => R::AdditionalProperties,
        "const" => R::Const,
        "enum" => R::Enum,
        "minimum" => R::Minimum,
        "maximum" => R::Maximum,
        "exclusiveMinimum" => R::ExclusiveMinimum,
        "exclusiveMaximum" => R::ExclusiveMaximum,
        "minLength" => R::MinLength,
        "maxLength" => R::MaxLength,
        "minItems" => R::MinItems,
        "maxItems" => R::MaxItems,
        "oneOf" => R::OneOf,
        "pattern" => R::Pattern,
        _ => return None,
    };
    let expected = match error.kind() {
        K::Type {
            kind: jsonschema::error::TypeKind::Single(kind),
        } => {
            use jsonschema::types::JsonType;
            Some(match kind {
                JsonType::Null => ArgumentExpectedType::Null,
                JsonType::Boolean => ArgumentExpectedType::Boolean,
                JsonType::Number => ArgumentExpectedType::Number,
                JsonType::Integer => ArgumentExpectedType::Integer,
                JsonType::String => ArgumentExpectedType::String,
                JsonType::Array => ArgumentExpectedType::Array,
                JsonType::Object => ArgumentExpectedType::Object,
            })
        }
        _ => None,
    };
    // Core validates field names, exact array-specific bounds and path length.
    ArgumentDiagnostic::new(path, reason.clone(), expected.clone())
        .or_else(|| ArgumentDiagnostic::new(prefix.into(), reason, expected))
}

pub(super) fn schema_failure(
    schema: &Value,
    value: &Value,
    prefix: &str,
) -> Option<ArgumentDiagnostic> {
    let validator = jsonschema::validator_for(schema).ok()?;
    let failures: Vec<_> = validator.iter_errors(value).collect();
    // A const/enum may also fail because the JSON primitive type is wrong.
    // Prefer that real type error so the expected type can be returned.
    let failure = failures
        .iter()
        .find(|error| {
            matches!(
                error.kind(),
                jsonschema::error::ValidationErrorKind::Type { .. }
            )
        })
        .or_else(|| failures.first());
    failure.and_then(|error| project(error, prefix))
}

pub(super) fn wire_failure(arguments: &str) -> Option<ArgumentDiagnostic> {
    if arguments.len() > 32 * 1024 {
        return None;
    }
    let value = match serde_json::from_str::<Value>(arguments) {
        Ok(value) => value,
        Err(error) => {
            return match error.classify() {
                serde_json::error::Category::Syntax => {
                    ArgumentDiagnostic::new("$".into(), R::Syntax, None)
                }
                serde_json::error::Category::Eof => {
                    ArgumentDiagnostic::new("$".into(), R::Eof, None)
                }
                _ => None,
            }
        }
    };
    if !serde_json::from_str::<UniqueKeys>(arguments).ok()?.0 {
        return None;
    }
    // Only the actual Arguments/Group envelope is typed at this point.
    // Objects are still Value and may contain the supported MiniMax wire
    // forms. Their fields must not be diagnosed before that adapter runs.
    // String enum/target-identity failures stay generic here; only the
    // unambiguous envelope primitive shape is projected from a typed error.
    let schema = json!({"type":"object","additionalProperties":false,"required":["groups"],"properties":{
        "groups":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["target","objects"],"properties":{
            "target":{"type":"string"},"objects":{"type":"array"},
            "coordinateSpace":{"type":"string"},
            "styleUnit":{"type":"string"}
        }}}
    }});
    // A duplicate-field typed failure can have a schema-valid Value projection.
    // In that case there is no inferred field diagnostic.
    schema_failure(&schema, &value, "$")
}

pub(super) fn object_failure(
    value: &Value,
    target: &mewu_core::VisualInputTarget,
    prefix: &str,
) -> Option<ArgumentDiagnostic> {
    schema_failure(
        &super::group_definition(target)["properties"]["objects"]["items"],
        value,
        prefix,
    )
}
