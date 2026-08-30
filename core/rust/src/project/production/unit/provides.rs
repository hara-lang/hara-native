use super::kind::UnitKind;
use crate::kernel::Form;
use std::collections::BTreeSet;

pub fn raw_provided_vars(form: &Form, module: &str) -> BTreeSet<String> {
    let mut output = BTreeSet::new();
    collect_provided_vars(form, module, &mut output);
    output
}

pub(crate) fn unit_kind(form: &Form) -> UnitKind {
    match list_head(without_metadata(form)) {
        Some("defmacro") => UnitKind::Macro,
        Some(
            "defprotocol"
                | "extend-type"
                | "defmulti"
                | "defmethod"
                | "defstruct"
                | "defmutable",
        ) => UnitKind::Registration,
        Some("def" | "defn" | "declare") => UnitKind::Definition,
        _ => UnitKind::Initializer,
    }
}

pub(super) fn provided_vars(form: &Form, module: &str) -> BTreeSet<String> {
    raw_provided_vars(form, module)
}

fn collect_provided_vars(form: &Form, module: &str, output: &mut BTreeSet<String>) {
    let Form::List(values) = without_metadata(form) else {
        return;
    };
    let Some(Form::Symbol(head)) = values.first() else {
        return;
    };
    match head.as_str() {
        "do" => {
            for value in values.iter().skip(1) {
                collect_provided_vars(value, module, output);
            }
        }
        "declare" => {
            for value in values.iter().skip(1) {
                if let Some(name) = binding_name(value) {
                    output.insert(qualify(module, name));
                }
            }
        }
        "def" | "defn" | "defmacro" | "defmulti" => {
            if let Some(name) = values.get(1).and_then(binding_name) {
                output.insert(qualify(module, name));
            }
        }
        "defstruct" | "defmutable" => {
            if let Some(name) = values.get(1).and_then(binding_name) {
                output.insert(qualify(module, name));
                output.insert(qualify(module, &format!("->{name}")));
                output.insert(qualify(module, &format!("map->{name}")));
            }
        }
        "defprotocol" => {
            if let Some(name) = values.get(1).and_then(binding_name) {
                output.insert(qualify(module, name));
            }
            for method in values.iter().skip(2) {
                let Form::List(parts) = without_metadata(method) else {
                    continue;
                };
                if let Some(Form::Symbol(name)) = parts.first() {
                    output.insert(qualify(module, name));
                }
            }
        }
        _ => {}
    }
}

pub(super) fn binding_name(form: &Form) -> Option<&str> {
    match without_metadata(form) {
        Form::Symbol(name) if !name.contains('/') => Some(name),
        _ => None,
    }
}

pub(super) fn list_head(form: &Form) -> Option<&str> {
    match form {
        Form::List(values) => match values.first() {
            Some(Form::Symbol(head)) => Some(head),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn qualify(module: &str, name: &str) -> String {
    if name.contains('/') {
        name.into()
    } else {
        format!("{module}/{name}")
    }
}

pub(super) fn without_metadata(form: &Form) -> &Form {
    match form {
        Form::Metadata(_, value) => without_metadata(value),
        value => value,
    }
}
