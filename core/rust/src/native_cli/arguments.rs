use crate::core::Value;
use crate::lang::data::Keyword;

pub(super) fn string<'a>(
    arguments: &'a [Value],
    index: usize,
    operation: &str,
) -> Result<&'a str, String> {
    match arguments.get(index) {
        Some(Value::String(value)) => Ok(value),
        _ => Err(format!(
            "foundation.kernel/{operation} expects string arguments"
        )),
    }
}

pub(super) fn optional_string<'a>(
    arguments: &'a [Value],
    index: usize,
    operation: &str,
) -> Result<Option<&'a str>, String> {
    match arguments.get(index) {
        None | Some(Value::Nil) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        _ => Err(format!(
            "foundation.kernel/{operation} expects an optional string argument"
        )),
    }
}

pub(super) fn strings(
    arguments: &[Value],
    index: usize,
    operation: &str,
) -> Result<Vec<String>, String> {
    match arguments.get(index) {
        Some(Value::Vector(values)) => values
            .iter()
            .map(|value| match value {
                Value::String(value) => Ok(value.clone()),
                _ => Err(format!(
                    "foundation.kernel/{operation} expects vectors of strings"
                )),
            })
            .collect(),
        _ => Err(format!(
            "foundation.kernel/{operation} expects a vector argument"
        )),
    }
}

pub(super) fn tap_value(tap: &crate::tap::Tap) -> Value {
    Value::Map(
        [
            (keyword("name"), Value::String(tap.name.clone())),
            (keyword("registry"), strings_value(tap.registry.clone())),
            (keyword("identity"), strings_value(tap.identity.clone())),
            (
                keyword("identity-key"),
                Value::String(tap.identity_key.clone()),
            ),
            (
                keyword("trust"),
                keyword(match tap.trust {
                    crate::tap::TrustMode::SignedRoot => "signed-root",
                    crate::tap::TrustMode::GithubGoverned => "github-governed",
                }),
            ),
        ]
        .into_iter()
        .collect(),
    )
}

pub(super) fn keyword(name: &str) -> Value {
    Value::Keyword(Keyword::from(name))
}

pub(super) fn strings_value(values: Vec<String>) -> Value {
    Value::Vector(values.into_iter().map(Value::String).collect())
}
