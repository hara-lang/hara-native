use crate::kernel::{parse_forms, Form};

pub const CORPUS_SCHEMA: &str = "hal.code-vm-production-corpus/0-alpha";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Corpus {
    pub id: String,
    pub upstream: String,
    pub cases: Vec<CorpusCase>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorpusCase {
    pub id: String,
    pub upstream_id: String,
    pub source_id: String,
    pub namespace: String,
    pub resource: String,
    pub source: String,
    pub expected: ExpectedOutcome,
    pub interpreter_required: bool,
    pub browser_safe: bool,
    pub steps: usize,
    pub trace_limit: usize,
    pub expect_dropped: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpectedOutcome {
    Display(String),
    ErrorCategory(String),
    CompileError(String),
}

pub fn parse_corpus(source: &str) -> Result<Corpus, String> {
    let mut forms = parse_forms(source).map_err(|error| error.to_string())?;
    if forms.len() != 1 {
        return Err("code.vm production corpus must contain one form".into());
    }
    let Form::Map(manifest) = forms.remove(0) else {
        return Err("code.vm production corpus must be a map".into());
    };
    let schema = required_string(&manifest, "corpus/schema", "corpus")?;
    if schema != CORPUS_SCHEMA {
        return Err(format!(
            "unsupported code.vm production corpus schema: {schema}"
        ));
    }
    let id = required_keyword(&manifest, "corpus/id", "corpus")?;
    let upstream = required_string(&manifest, "corpus/upstream", "corpus")?;
    let Some(Form::Vector(cases)) = entry(&manifest, "cases") else {
        return Err("code.vm production corpus :cases must be a vector".into());
    };
    if cases.is_empty() {
        return Err("code.vm production corpus must not be empty".into());
    }
    let cases = cases
        .iter()
        .map(parse_case)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Corpus {
        id,
        upstream,
        cases,
    })
}

pub fn validate_upstream(corpus: &Corpus, source: &str) -> Result<(), String> {
    let mut forms = parse_forms(source).map_err(|error| error.to_string())?;
    if forms.len() != 1 {
        return Err("code.vm upstream corpus must contain one form".into());
    }
    let Form::Map(document) = forms.remove(0) else {
        return Err("code.vm upstream corpus must be a map".into());
    };
    let Some(Form::Vector(cases)) = entry(&document, "cases") else {
        return Err("code.vm upstream corpus :cases must be a vector".into());
    };

    for case in &corpus.cases {
        let matches = cases
            .iter()
            .filter_map(|form| match form {
                Form::Map(entries)
                    if matches!(entry(entries, "id"), Some(Form::Keyword(id)) if id == &case.upstream_id) =>
                {
                    Some(entries)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(format!(
                "code.vm case :{} expected exactly one upstream case :{}; found {}",
                case.id,
                case.upstream_id,
                matches.len()
            ));
        }
        let upstream = matches[0];
        let upstream_source = required_string(upstream, "source", &case.upstream_id)?;
        if case.source != upstream_source {
            return Err(format!(
                "code.vm case :{} source differs from upstream :{}",
                case.id, case.upstream_id
            ));
        }
        let Form::Map(expectation) = required(upstream, "expect", &case.upstream_id)? else {
            return Err(format!(":{} :expect must be a map", case.upstream_id));
        };
        let upstream_expected = parse_expectation(expectation, &case.upstream_id)?;
        if case.expected != upstream_expected {
            return Err(format!(
                "code.vm case :{} expectation differs from upstream :{}",
                case.id, case.upstream_id
            ));
        }
    }
    Ok(())
}

fn parse_case(form: &Form) -> Result<CorpusCase, String> {
    let Form::Map(entries) = form else {
        return Err("every code.vm corpus case must be a map".into());
    };
    let id = required_keyword(entries, "id", "case")?;
    let upstream_id = required_keyword(entries, "upstream-id", &id)?;
    let source = required_string(entries, "source", &id)?;
    let Form::Map(expect) = required(entries, "expect", &id)? else {
        return Err(format!(":{id} :expect must be a map"));
    };
    let expected = parse_expectation(expect, &id)?;
    let path = id.replace('/', "/");
    let dotted = id.replace('/', ".").replace('-', "_");
    Ok(CorpusCase {
        source_id: format!("code.vm/{id}"),
        namespace: format!("code.vm.fixture.{dotted}"),
        resource: format!("code/vm/fixture/{path}.hal"),
        id,
        upstream_id,
        source,
        expected,
        interpreter_required: optional_bool(entries, "interpreter-required", true)?,
        browser_safe: optional_bool(entries, "browser-safe", false)?,
        steps: optional_usize(entries, "steps", 512)?,
        trace_limit: optional_usize(entries, "trace-limit", 128)?,
        expect_dropped: optional_bool(entries, "expect-dropped", false)?,
    })
}

fn parse_expectation(entries: &[(Form, Form)], id: &str) -> Result<ExpectedOutcome, String> {
    let mut found = Vec::new();
    if let Some(Form::String(value)) = entry(entries, "display") {
        found.push(ExpectedOutcome::Display(value.clone()));
    }
    if let Some(Form::String(value)) = entry(entries, "error-category") {
        found.push(ExpectedOutcome::ErrorCategory(value.clone()));
    }
    if let Some(Form::String(value)) = entry(entries, "compile-error") {
        found.push(ExpectedOutcome::CompileError(value.clone()));
    }
    if found.len() != 1 {
        return Err(format!(
            ":{id} :expect must contain exactly one supported expectation"
        ));
    }
    Ok(found.remove(0))
}

fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries
        .iter()
        .find_map(|(candidate, value)| match candidate {
            Form::Keyword(name) if name == key => Some(value),
            _ => None,
        })
}

fn required<'a>(entries: &'a [(Form, Form)], key: &str, id: &str) -> Result<&'a Form, String> {
    entry(entries, key).ok_or_else(|| format!(":{id} missing :{key}"))
}

fn required_string(entries: &[(Form, Form)], key: &str, id: &str) -> Result<String, String> {
    match required(entries, key, id)? {
        Form::String(value) if !value.is_empty() => Ok(value.clone()),
        _ => Err(format!(":{id} :{key} must be a non-empty string")),
    }
}

fn required_keyword(entries: &[(Form, Form)], key: &str, id: &str) -> Result<String, String> {
    match required(entries, key, id)? {
        Form::Keyword(value) if !value.is_empty() => Ok(value.clone()),
        _ => Err(format!(":{id} :{key} must be a keyword")),
    }
}

fn optional_bool(entries: &[(Form, Form)], key: &str, default: bool) -> Result<bool, String> {
    match entry(entries, key) {
        None => Ok(default),
        Some(Form::Bool(value)) => Ok(*value),
        _ => Err(format!(":{key} must be a boolean")),
    }
}

fn optional_usize(entries: &[(Form, Form)], key: &str, default: usize) -> Result<usize, String> {
    match entry(entries, key) {
        None => Ok(default),
        Some(Form::Number(value)) if *value > 0 => {
            usize::try_from(*value).map_err(|_| format!(":{key} exceeds the host size limit"))
        }
        _ => Err(format!(":{key} must be a positive integer")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_corpus_is_well_formed_and_tracks_upstream_cases() {
        let corpus = parse_corpus(include_str!("../../../assets/code-vm-conformance.edn"))
            .expect("embedded corpus");
        assert_eq!(corpus.id, "code.vm/production");
        assert!(corpus.cases.len() >= 12);
        assert!(corpus.cases.iter().any(|case| case.browser_safe));
        assert!(corpus.cases.iter().all(|case| !case.upstream_id.is_empty()));
    }

    #[test]
    fn upstream_validation_rejects_missing_and_drifted_cases() {
        let corpus = parse_corpus(
            r#"{:corpus/schema "hal.code-vm-production-corpus/0-alpha"
                :corpus/id :code.vm/test
                :corpus/upstream "upstream.edn"
                :cases [{:id :literal/value
                         :upstream-id :literal/value
                         :source "42"
                         :expect {:display "42"}}]}"#,
        )
        .unwrap();
        let matching = r#"{:cases [{:id :literal/value :source "42" :expect {:display "42"}}]}"#;
        validate_upstream(&corpus, matching).unwrap();
        assert!(validate_upstream(&corpus, "{:cases []}")
            .unwrap_err()
            .contains("expected exactly one upstream case"));
        assert!(validate_upstream(
            &corpus,
            r#"{:cases [{:id :literal/value :source "41" :expect {:display "42"}}]}"#
        )
        .unwrap_err()
        .contains("source differs"));
    }
}
