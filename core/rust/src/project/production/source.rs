use crate::kernel::{read_forms, Form, GeneratedNamespaceConfig, SpannedForm};
use crate::project::Project;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: String,
    pub operation: String,
    pub module: String,
    pub location: SourceLocation,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct SourceModule {
    pub name: String,
    pub path: String,
    pub source: String,
    pub namespace_form: String,
    pub body_line_base: usize,
    pub forms: Vec<SpannedForm>,
    pub dependencies: Vec<String>,
    pub digest: String,
    pub standard_library: bool,
}

impl SourceModule {
    pub fn parse(
        expected_name: Option<&str>,
        path: impl Into<String>,
        source: impl Into<String>,
        standard_library: bool,
    ) -> Result<Self, String> {
        let path = path.into();
        let source = source.into();
        let parsed = read_forms(&source).map_err(|error| format!("{path}: {error}"))?;
        let namespace = parsed
            .iter()
            .find(|form| namespace_declaration(&form.form).is_some())
            .ok_or_else(|| format!("{path}: HAL module is missing ns form"))?;
        let name = namespace_declaration(&namespace.form).unwrap().to_owned();
        if expected_name.is_some_and(|expected| expected != name.as_str()) {
            return Err(format!(
                "{path}: declared namespace {name} does not match resource {}",
                expected_name.unwrap()
            ));
        }
        let namespace_form = source
            .get(namespace.span.start.offset..namespace.span.end.offset)
            .ok_or_else(|| format!("{path}: invalid namespace source span"))?
            .to_owned();
        let body_start = namespace.span.end.offset;
        let body_line_base = source[..body_start]
            .chars()
            .filter(|character| *character == '\n')
            .count();
        let body = source
            .get(body_start..)
            .ok_or_else(|| format!("{path}: invalid module body span"))?;
        let forms = read_forms(body).map_err(|error| format!("{path}: {error}"))?;
        let dependencies = namespace_dependencies(&namespace.form)?;
        let digest = hex(&Sha256::digest(source.as_bytes()));
        Ok(Self {
            name,
            path,
            source,
            namespace_form,
            body_line_base,
            forms,
            dependencies,
            digest,
            standard_library,
        })
    }

    #[cfg(test)]
    pub fn synthetic(name: &str, source: &str) -> Self {
        Self::parse(Some(name), format!("fixture:{name}"), source, false).unwrap()
    }
}

pub fn collect_project_modules(project: &Project) -> Result<Vec<SourceModule>, String> {
    let mut modules = Vec::new();
    for path in crate::project::files_in(&project.root, &project.source_paths)? {
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        let relative = path
            .strip_prefix(&project.root)
            .unwrap_or(path.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        modules.push(SourceModule::parse(None, relative, source, false)?);
    }
    modules.sort_by(|left, right| left.name.cmp(&right.name));
    reject_duplicate_names(&modules)?;
    Ok(modules)
}

pub fn collect_embedded_modules() -> Result<Vec<SourceModule>, String> {
    let mut modules = crate::EMBEDDED_HAL_RESOURCES
        .iter()
        .filter(|(name, _, _)| standard_library_namespace(name))
        .map(|(name, path, source)| {
            SourceModule::parse(Some(name), (*path).to_owned(), (*source).to_owned(), true)
        })
        .collect::<Result<Vec<_>, _>>()?;
    modules.sort_by(|left, right| left.name.cmp(&right.name));
    reject_duplicate_names(&modules)?;
    Ok(modules)
}

pub fn deterministic_module_order(modules: &[SourceModule]) -> Vec<usize> {
    let positions = modules
        .iter()
        .enumerate()
        .map(|(index, module)| (module.name.as_str(), index))
        .collect::<BTreeMap<_, _>>();
    let mut remaining = (0..modules.len()).collect::<BTreeSet<_>>();
    let mut complete = BTreeSet::new();
    let mut output = Vec::with_capacity(modules.len());
    loop {
        let ready = remaining
            .iter()
            .copied()
            .filter(|index| {
                modules[*index]
                    .dependencies
                    .iter()
                    .filter_map(|dependency| positions.get(dependency.as_str()))
                    .all(|dependency| complete.contains(dependency))
            })
            .collect::<Vec<_>>();
        if ready.is_empty() {
            break;
        }
        for index in ready {
            remaining.remove(&index);
            complete.insert(index);
            output.push(index);
        }
    }
    output.extend(remaining);
    output
}

pub fn aggregate_digest(modules: &[SourceModule]) -> (usize, String) {
    let mut hasher = Sha256::new();
    let mut bytes = 0usize;
    let mut ordered = modules.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.name.cmp(&right.name));
    for module in ordered {
        bytes += module.source.len();
        hasher.update(module.name.as_bytes());
        hasher.update([0]);
        hasher.update(module.source.as_bytes());
        hasher.update([0]);
    }
    (bytes, hex(&hasher.finalize()))
}

fn namespace_declaration(form: &Form) -> Option<&str> {
    let Form::List(values) = without_metadata(form) else {
        return None;
    };
    if !matches!(values.first(), Some(Form::Symbol(head)) if head == "ns" || head == "ns+") {
        return None;
    }
    match values.get(1) {
        Some(Form::Symbol(name)) => Some(name),
        _ => None,
    }
}

fn namespace_dependencies(form: &Form) -> Result<Vec<String>, String> {
    let Form::List(values) = without_metadata(form) else {
        return Err("module namespace declaration must be a list".into());
    };
    if values.len() < 2 {
        return Err("module namespace declaration is incomplete".into());
    }
    let config = GeneratedNamespaceConfig::configure_with(&values[2..], |_| true)?;
    let mut dependencies = config.required_namespaces().to_vec();
    dependencies.extend(config.used_namespaces().iter().cloned());
    dependencies.sort();
    dependencies.dedup();
    Ok(dependencies)
}

fn without_metadata(form: &Form) -> &Form {
    match form {
        Form::Metadata(_, value) => without_metadata(value),
        value => value,
    }
}

fn standard_library_namespace(namespace: &str) -> bool {
    ["std.", "code."]
        .iter()
        .any(|prefix| namespace.starts_with(prefix))
}

fn reject_duplicate_names(modules: &[SourceModule]) -> Result<(), String> {
    for pair in modules.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(format!(
                "duplicate production module {}: {} and {}",
                pair[0].name, pair[0].path, pair[1].path
            ));
        }
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
