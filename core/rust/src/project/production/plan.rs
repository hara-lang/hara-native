use crate::kernel::{parse, Form};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPlan {
    pub project_id: String,
    pub project_version: String,
    pub profile: String,
    pub language: String,
    pub main: String,
    pub entrypoints: Vec<String>,
    pub default_entrypoint: String,
    pub keep_vars: Vec<String>,
    pub keep_namespaces: Vec<String>,
    pub output_bundle: String,
    pub output_report: String,
}

impl BuildPlan {
    pub fn parse(source: &str) -> Result<Self, String> {
        let form =
            parse(source).map_err(|error| format!("invalid production build plan: {error}"))?;
        let entries = map_entries(&form, "production build plan must be an EDN map")?;
        let project_id = scalar(required(entries, "project/id")?, ":project/id")?;
        let project_version = string(required(entries, "project/version")?, ":project/version")?;
        let profile = identifier(required(entries, "profile/name")?, ":profile/name")?;
        let language = identifier(required(entries, "profile/language")?, ":profile/language")?;
        let main = scalar(required(entries, "profile/main")?, ":profile/main")?;
        let tree_shake = boolean(required(entries, "build/tree-shake")?, ":build/tree-shake")?;
        let entrypoints = symbol_vector(
            required(entries, "build/entrypoints")?,
            ":build/entrypoints",
            true,
        )?;
        let default_entrypoint = optional(entries, "build/default-entrypoint")
            .map(|value| qualified_symbol(value, ":build/default-entrypoint"))
            .transpose()?;
        let keep_vars = symbol_vector(
            required(entries, "build/keep-vars")?,
            ":build/keep-vars",
            true,
        )?;
        let keep_namespaces = symbol_vector(
            required(entries, "build/keep-namespaces")?,
            ":build/keep-namespaces",
            false,
        )?;
        let output_bundle = string(
            required(entries, "build/output-bundle")?,
            ":build/output-bundle",
        )?;
        let output_report = string(
            required(entries, "build/output-report")?,
            ":build/output-report",
        )?;
        if language != "hara" {
            return Err("production build plan must use :profile/language :hara".into());
        }
        if !tree_shake {
            return Err("production build plan must opt in with :build/tree-shake true".into());
        }
        if entrypoints.is_empty() {
            return Err("production build plan has no entrypoints".into());
        }
        let default_entrypoint = match default_entrypoint {
            Some(default) => default,
            None if entrypoints.len() == 1 => entrypoints[0].clone(),
            None => {
                return Err(
                    "production build plan with multiple entrypoints requires :build/default-entrypoint"
                        .into(),
                )
            }
        };
        if !entrypoints.contains(&default_entrypoint) {
            return Err(":build/default-entrypoint must name one of :build/entrypoints".into());
        }
        Ok(Self {
            project_id,
            project_version,
            profile,
            language,
            main,
            entrypoints,
            default_entrypoint,
            keep_vars,
            keep_namespaces,
            output_bundle,
            output_report,
        })
    }

    pub fn report_path(&self, root: &std::path::Path) -> PathBuf {
        root.join(&self.output_report)
    }
}

fn map_entries<'a>(form: &'a Form, message: &str) -> Result<&'a [(Form, Form)], String> {
    match form {
        Form::Map(entries) => Ok(entries),
        _ => Err(message.into()),
    }
}

fn required<'a>(entries: &'a [(Form, Form)], key: &str) -> Result<&'a Form, String> {
    entries
        .iter()
        .find_map(|(candidate, value)| {
            matches!(candidate, Form::Keyword(name) if name == key).then_some(value)
        })
        .ok_or_else(|| format!("production build plan is missing :{key}"))
}

fn optional<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries.iter().find_map(|(candidate, value)| {
        matches!(candidate, Form::Keyword(name) if name == key).then_some(value)
    })
}

fn scalar(form: &Form, label: &str) -> Result<String, String> {
    match form {
        Form::Symbol(value) | Form::String(value) => Ok(value.clone()),
        _ => Err(format!("{label} must be a symbol or string")),
    }
}

fn string(form: &Form, label: &str) -> Result<String, String> {
    match form {
        Form::String(value) => Ok(value.clone()),
        _ => Err(format!("{label} must be a string")),
    }
}

fn boolean(form: &Form, label: &str) -> Result<bool, String> {
    match form {
        Form::Bool(value) => Ok(*value),
        _ => Err(format!("{label} must be a boolean")),
    }
}

fn identifier(form: &Form, label: &str) -> Result<String, String> {
    match form {
        Form::Keyword(value) | Form::Symbol(value) | Form::String(value) => Ok(value.clone()),
        _ => Err(format!("{label} must be a keyword, symbol, or string")),
    }
}

fn qualified_symbol(form: &Form, label: &str) -> Result<String, String> {
    let value = scalar(form, label)?;
    if qualified_var(&value) {
        Ok(value)
    } else {
        Err(format!("{label} must be a qualified Var symbol"))
    }
}

fn symbol_vector(form: &Form, label: &str, qualified: bool) -> Result<Vec<String>, String> {
    let Form::Vector(values) = form else {
        return Err(format!("{label} must be a vector"));
    };
    let mut output = values
        .iter()
        .map(|value| scalar(value, label))
        .collect::<Result<Vec<_>, _>>()?;
    if qualified && output.iter().any(|value| !qualified_var(value)) {
        return Err(format!("{label} must contain qualified Var symbols"));
    }
    if !qualified && output.iter().any(|value| value.contains('/')) {
        return Err(format!("{label} must contain namespace symbols"));
    }
    output.sort();
    output.dedup();
    Ok(output)
}

fn qualified_var(value: &str) -> bool {
    let Some((namespace, name)) = value.split_once('/') else {
        return false;
    };
    !namespace.is_empty() && !name.is_empty() && !name.contains('/')
}

#[cfg(test)]
mod tests {
    use super::BuildPlan;

    fn source(entrypoints: &str, default: &str) -> String {
        format!(
            "{{:project/id demo-app \
              :project/version \"0.1.0\" \
              :profile/name :production \
              :profile/language :hara \
              :profile/main app.main \
              :build/tree-shake true \
              :build/entrypoints [{entrypoints}] \
              {default} \
              :build/keep-vars [] \
              :build/keep-namespaces [] \
              :build/output-bundle \"target/app.hbx\" \
              :build/output-report \"target/app.shake.edn\"}}"
        )
    }

    #[test]
    fn infers_the_only_entrypoint_as_default() {
        let plan = BuildPlan::parse(&source("app.main/start", "")).unwrap();
        assert_eq!(plan.default_entrypoint, "app.main/start");
    }

    #[test]
    fn requires_a_default_for_multiple_entrypoints() {
        let error = BuildPlan::parse(&source("app.main/start app.main/worker", "")).unwrap_err();
        assert!(error.contains("requires :build/default-entrypoint"));
    }

    #[test]
    fn validates_the_default_against_the_entrypoint_set() {
        let error = BuildPlan::parse(&source(
            "app.main/start",
            ":build/default-entrypoint app.main/missing",
        ))
        .unwrap_err();
        assert!(error.contains("must name one of"));
    }
}
