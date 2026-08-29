use super::super::source::SourceLocation;
use crate::kernel::Form;
use std::collections::BTreeSet;

pub fn source_form(location: &SourceLocation) -> Form {
    map(vec![
        ("path", string(&location.path)),
        ("line", number(location.line)),
        ("column", number(location.column)),
        ("end-line", number(location.end_line)),
        ("end-column", number(location.end_column)),
    ])
}

pub fn map(values: Vec<(&str, Form)>) -> Form {
    Form::Map(
        values
            .into_iter()
            .map(|(key, value)| (Form::Keyword(key.into()), value))
            .collect(),
    )
}

pub fn keyword(value: &str) -> Form {
    Form::Keyword(value.into())
}

pub fn symbol(value: &str) -> Form {
    Form::Symbol(value.into())
}

pub fn string(value: &str) -> Form {
    Form::String(value.into())
}

pub fn number(value: usize) -> Form {
    Form::Number(i64::try_from(value).unwrap_or(i64::MAX))
}

pub fn boolean(value: bool) -> Form {
    Form::Bool(value)
}

pub fn nil() -> Form {
    Form::Nil
}

pub fn vector(values: impl IntoIterator<Item = Form>) -> Form {
    Form::Vector(values.into_iter().collect())
}

pub fn symbol_vector(values: impl IntoIterator<Item = String>) -> Form {
    vector(values.into_iter().map(|value| symbol(&value)))
}

pub fn string_vector(values: impl IntoIterator<Item = String>) -> Form {
    vector(values.into_iter().map(|value| string(&value)))
}

pub fn symbols(values: &BTreeSet<String>) -> Form {
    symbol_vector(values.iter().cloned())
}

pub fn strings(values: &BTreeSet<String>) -> Form {
    string_vector(values.iter().cloned())
}
