//! Shared application-command routing.
//!
//! The host-facing `std.native.Command` implementation and the native
//! maintenance launcher deliberately share this model. The model knows how to
//! validate routes and turn argv into a deterministic request; it does not
//! know how a host invokes a handler or writes a response.

use std::collections::{BTreeMap, HashSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandError {
    pub code: &'static str,
    pub message: String,
}

impl CommandError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub id: String,
    pub desc: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RouteHandle(u64);

impl RouteHandle {
    pub fn from_id(id: u64) -> Self {
        Self(id)
    }

    pub fn id(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    Boolean,
    String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedValue {
    Boolean(bool),
    String(String),
    Strings(Vec<String>),
}

impl ParsedValue {
    fn type_name(&self) -> &'static str {
        match self {
            Self::Boolean(_) => "boolean",
            Self::String(_) => "string",
            Self::Strings(_) => "strings",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionSpec {
    pub id: String,
    pub long: Option<String>,
    pub short: Option<char>,
    pub kind: OptionKind,
    pub many: bool,
    pub default: Option<ParsedValue>,
}

impl OptionSpec {
    pub fn long_name(&self) -> String {
        self.long
            .clone()
            .unwrap_or_else(|| format!("--{}", self.id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgumentSpec {
    pub id: String,
    pub required: bool,
    pub many: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteSpec {
    pub id: String,
    pub path: Vec<String>,
    pub aliases: Vec<Vec<String>>,
    pub desc: String,
    pub options: Vec<OptionSpec>,
    pub arguments: Vec<ArgumentSpec>,
    /// Preserve all remaining argv values as positional arguments. This is
    /// for delegated tools whose flags are owned by a downstream handler.
    pub passthrough: bool,
}

#[derive(Debug, Clone)]
pub struct Route<H> {
    pub spec: RouteSpec,
    pub handler: H,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub app_id: String,
    pub route: RouteHandle,
    pub route_id: String,
    pub route_path: Vec<String>,
    pub argv: Vec<String>,
    pub arguments: BTreeMap<String, ParsedValue>,
    pub options: BTreeMap<String, ParsedValue>,
    generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub stdout: String,
    pub stderr: String,
    pub exit: i64,
}

impl Response {
    pub fn checked(self) -> Result<Self, CommandError> {
        if !(0..=255).contains(&self.exit) {
            return Err(CommandError::new(
                ":command/invalid-response",
                ":exit must be an integer between 0 and 255",
            ));
        }
        Ok(self)
    }

    pub fn failure(exit: i64, error: impl fmt::Display) -> Self {
        Self {
            stdout: String::new(),
            stderr: format!("{error}\n"),
            exit,
        }
    }
}

#[derive(Debug, Clone)]
struct StoredRoute<H> {
    handle: RouteHandle,
    route: Route<H>,
}

#[derive(Debug, Clone)]
pub struct Snapshot<H> {
    routes: Vec<StoredRoute<H>>,
}

/// A context-local command application. `H` is intentionally opaque so native
/// hosts can use guest functions while the Rust CLI uses Rust handlers against
/// exactly the same router.
#[derive(Debug, Clone)]
pub struct App<H> {
    config: AppConfig,
    routes: Vec<StoredRoute<H>>,
    next_handle: u64,
    generation: u64,
    closed: bool,
}

impl<H> App<H> {
    pub fn create(config: AppConfig) -> Result<Self, CommandError> {
        validate_config(&config)?;
        Ok(Self {
            config,
            routes: Vec::new(),
            next_handle: 1,
            generation: 0,
            closed: false,
        })
    }

    pub fn config(&self) -> AppConfig {
        self.config.clone()
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    pub fn routes(&self) -> Result<Vec<RouteSpec>, CommandError> {
        self.require_open()?;
        Ok(self
            .routes
            .iter()
            .map(|stored| stored.route.spec.clone())
            .collect())
    }

    pub fn install(&mut self, route: Route<H>) -> Result<RouteHandle, CommandError> {
        self.require_open()?;
        validate_route(&route.spec)?;
        let new_paths = route_paths(&route.spec);
        for existing in &self.routes {
            for existing_path in route_paths(&existing.route.spec) {
                if new_paths
                    .iter()
                    .any(|candidate| candidate == &existing_path)
                {
                    return Err(CommandError::new(
                        ":command/route-conflict",
                        format!(
                            "route {} conflicts with {} at {}",
                            route.spec.id,
                            existing.route.spec.id,
                            display_path(&existing_path)
                        ),
                    ));
                }
            }
        }
        if self
            .routes
            .iter()
            .any(|stored| stored.route.spec.id == route.spec.id)
        {
            return Err(CommandError::new(
                ":command/route-conflict",
                format!("route id is already installed: {}", route.spec.id),
            ));
        }
        let handle = RouteHandle(self.next_handle);
        self.next_handle += 1;
        self.routes.push(StoredRoute { handle, route });
        self.generation += 1;
        Ok(handle)
    }

    pub fn uninstall(&mut self, handle: RouteHandle) -> Result<bool, CommandError> {
        self.require_open()?;
        let Some(index) = self.routes.iter().position(|route| route.handle == handle) else {
            return Ok(false);
        };
        self.routes.remove(index);
        self.generation += 1;
        Ok(true)
    }

    pub fn snapshot(&self) -> Result<Snapshot<H>, CommandError>
    where
        H: Clone,
    {
        self.require_open()?;
        Ok(Snapshot {
            routes: self.routes.clone(),
        })
    }

    pub fn restore(&mut self, snapshot: Snapshot<H>) -> Result<(), CommandError> {
        self.require_open()?;
        self.routes = snapshot.routes;
        self.next_handle = self
            .routes
            .iter()
            .map(|stored| stored.handle.id())
            .max()
            .unwrap_or(0)
            + 1;
        self.generation += 1;
        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), CommandError> {
        self.require_open()?;
        if !self.routes.is_empty() {
            self.routes.clear();
            self.generation += 1;
        }
        Ok(())
    }

    pub fn close(&mut self) {
        if !self.closed {
            self.routes.clear();
            self.generation += 1;
            self.closed = true;
        }
    }

    pub fn parse(&self, argv: Vec<String>) -> Result<Request, CommandError> {
        self.require_open()?;
        let (stored, path_length) = self.match_route(&argv)?;
        let (options, arguments) = parse_route_arguments(&stored.route.spec, &argv[path_length..])?;
        Ok(Request {
            app_id: self.config.id.clone(),
            route: stored.handle,
            route_id: stored.route.spec.id.clone(),
            route_path: stored.route.spec.path.clone(),
            argv,
            arguments,
            options,
            generation: self.generation,
        })
    }

    pub fn handler(&self, request: &Request) -> Result<&H, CommandError> {
        self.require_open()?;
        if request.app_id != self.config.id {
            return Err(CommandError::new(
                ":command/foreign-request",
                "request belongs to a different application",
            ));
        }
        if request.generation != self.generation {
            return Err(CommandError::new(
                ":command/stale-request",
                "routes changed after this request was parsed",
            ));
        }
        self.routes
            .iter()
            .find(|stored| stored.handle == request.route)
            .map(|stored| &stored.route.handler)
            .ok_or_else(|| {
                CommandError::new(
                    ":command/stale-request",
                    "the matched route is no longer installed",
                )
            })
    }

    pub fn run<F>(&self, argv: Vec<String>, invoke: F) -> Response
    where
        F: FnOnce(&H, &Request) -> Result<Response, CommandError>,
    {
        let request = match self.parse(argv) {
            Ok(request) => request,
            Err(error) => return Response::failure(2, error),
        };
        let handler = match self.handler(&request) {
            Ok(handler) => handler,
            Err(error) => return Response::failure(1, error),
        };
        match invoke(handler, &request).and_then(Response::checked) {
            Ok(response) => response,
            Err(error) => Response::failure(1, error),
        }
    }

    fn require_open(&self) -> Result<(), CommandError> {
        (!self.closed)
            .then_some(())
            .ok_or_else(|| CommandError::new(":command/closed", "application is closed"))
    }

    fn match_route(&self, argv: &[String]) -> Result<(&StoredRoute<H>, usize), CommandError> {
        let mut matched: Option<(&StoredRoute<H>, usize)> = None;
        for stored in &self.routes {
            for candidate in route_paths(&stored.route.spec) {
                if candidate.is_empty() {
                    if argv.is_empty() {
                        matched = Some((stored, 0));
                    }
                    continue;
                }
                if argv.starts_with(&candidate) {
                    let replace = matched
                        .as_ref()
                        .is_none_or(|(_, length)| candidate.len() > *length);
                    if replace {
                        matched = Some((stored, candidate.len()));
                    }
                }
            }
        }
        matched.ok_or_else(|| {
            CommandError::new(
                ":command/unknown-route",
                match argv.first() {
                    Some(value) => format!("unknown command: {value}"),
                    None => "no root route is installed".into(),
                },
            )
        })
    }
}

fn validate_config(config: &AppConfig) -> Result<(), CommandError> {
    if config.id.trim().is_empty() {
        return Err(CommandError::new(
            ":command/invalid-app",
            ":id must be non-empty",
        ));
    }
    if config.desc.trim().is_empty() {
        return Err(CommandError::new(
            ":command/invalid-app",
            ":desc must be non-empty",
        ));
    }
    Ok(())
}

fn validate_route(route: &RouteSpec) -> Result<(), CommandError> {
    if route.id.trim().is_empty() {
        return Err(CommandError::new(
            ":command/invalid-route",
            ":id must be non-empty",
        ));
    }
    if route.desc.trim().is_empty() {
        return Err(CommandError::new(
            ":command/invalid-route",
            ":desc must be non-empty",
        ));
    }
    for path in route_paths(route) {
        if path.iter().any(|part| part.trim().is_empty()) {
            return Err(CommandError::new(
                ":command/invalid-route",
                ":path and :aliases may not contain empty segments",
            ));
        }
    }
    let mut option_ids = HashSet::new();
    let mut option_long = HashSet::new();
    let mut option_short = HashSet::new();
    for option in &route.options {
        if option.id.trim().is_empty() || !option_ids.insert(option.id.clone()) {
            return Err(CommandError::new(
                ":command/invalid-route",
                "option ids must be unique and non-empty",
            ));
        }
        let long = option.long_name();
        if !long.starts_with("--") || long.len() <= 2 || !option_long.insert(long.clone()) {
            return Err(CommandError::new(
                ":command/invalid-route",
                format!("invalid or duplicate long option: {long}"),
            ));
        }
        if let Some(short) = option.short {
            if !option_short.insert(short) {
                return Err(CommandError::new(
                    ":command/invalid-route",
                    format!("duplicate short option: -{short}"),
                ));
            }
        }
        if option.many && matches!(option.kind, OptionKind::Boolean) {
            return Err(CommandError::new(
                ":command/invalid-route",
                "boolean options may not be :many?",
            ));
        }
        if let Some(default) = &option.default {
            let valid = match option.kind {
                OptionKind::Boolean => matches!(default, ParsedValue::Boolean(_)),
                OptionKind::String if option.many => matches!(default, ParsedValue::Strings(_)),
                OptionKind::String => matches!(default, ParsedValue::String(_)),
            };
            if !valid {
                return Err(CommandError::new(
                    ":command/invalid-route",
                    format!(
                        "default for option {} has type {}",
                        option.id,
                        default.type_name()
                    ),
                ));
            }
        }
    }
    if route.passthrough && !route.options.is_empty() {
        return Err(CommandError::new(
            ":command/invalid-route",
            ":passthrough? routes may not declare options",
        ));
    }
    let mut argument_ids = HashSet::new();
    for (index, argument) in route.arguments.iter().enumerate() {
        if argument.id.trim().is_empty() || !argument_ids.insert(argument.id.clone()) {
            return Err(CommandError::new(
                ":command/invalid-route",
                "argument ids must be unique and non-empty",
            ));
        }
        if argument.many && index + 1 != route.arguments.len() {
            return Err(CommandError::new(
                ":command/invalid-route",
                "only the final positional argument may be :many?",
            ));
        }
    }
    Ok(())
}

fn route_paths(route: &RouteSpec) -> Vec<Vec<String>> {
    let mut paths = vec![route.path.clone()];
    paths.extend(route.aliases.clone());
    paths
}

fn display_path(path: &[String]) -> String {
    if path.is_empty() {
        "<root>".into()
    } else {
        path.join(" ")
    }
}

fn parse_route_arguments(
    route: &RouteSpec,
    argv: &[String],
) -> Result<(BTreeMap<String, ParsedValue>, BTreeMap<String, ParsedValue>), CommandError> {
    let mut options = BTreeMap::new();
    for option in &route.options {
        let default = option.default.clone().unwrap_or_else(|| match option.kind {
            OptionKind::Boolean => ParsedValue::Boolean(false),
            OptionKind::String if option.many => ParsedValue::Strings(Vec::new()),
            OptionKind::String => ParsedValue::String(String::new()),
        });
        options.insert(option.id.clone(), default);
    }
    if route.passthrough {
        return Ok((options, parse_positionals(route, argv.to_vec())?));
    }
    let mut positional = Vec::new();
    let mut supplied = HashSet::new();
    let mut options_enabled = true;
    let mut index = 0;
    while index < argv.len() {
        let value = &argv[index];
        if options_enabled && value == "--" {
            options_enabled = false;
            index += 1;
            continue;
        }
        if options_enabled && value.starts_with("--") && value.len() > 2 {
            let (name, inline) = value
                .split_once('=')
                .map_or((value.as_str(), None), |(name, value)| {
                    (name, Some(value.to_owned()))
                });
            let option = route
                .options
                .iter()
                .find(|option| option.long_name() == name)
                .ok_or_else(|| {
                    CommandError::new(":command/unknown-option", format!("unknown option: {name}"))
                })?;
            index = parse_option(option, inline, argv, index, &mut options, &mut supplied)?;
            continue;
        }
        if options_enabled && value.starts_with('-') && value.len() == 2 {
            let short = value.chars().nth(1).expect("two-character option");
            let option = route
                .options
                .iter()
                .find(|option| option.short == Some(short))
                .ok_or_else(|| {
                    CommandError::new(
                        ":command/unknown-option",
                        format!("unknown option: -{short}"),
                    )
                })?;
            index = parse_option(option, None, argv, index, &mut options, &mut supplied)?;
            continue;
        }
        positional.push(value.clone());
        index += 1;
    }
    Ok((options, parse_positionals(route, positional)?))
}

fn parse_positionals(
    route: &RouteSpec,
    positional: Vec<String>,
) -> Result<BTreeMap<String, ParsedValue>, CommandError> {
    let mut arguments = BTreeMap::new();
    let mut cursor = 0;
    for argument in &route.arguments {
        if argument.many {
            let values = positional[cursor..].to_vec();
            if argument.required && values.is_empty() {
                return Err(CommandError::new(
                    ":command/missing-argument",
                    format!("missing argument: {}", argument.id),
                ));
            }
            arguments.insert(argument.id.clone(), ParsedValue::Strings(values));
            cursor = positional.len();
            break;
        }
        let value = positional.get(cursor).cloned();
        match value {
            Some(value) => {
                arguments.insert(argument.id.clone(), ParsedValue::String(value));
                cursor += 1;
            }
            None if argument.required => {
                return Err(CommandError::new(
                    ":command/missing-argument",
                    format!("missing argument: {}", argument.id),
                ));
            }
            None => {
                arguments.insert(argument.id.clone(), ParsedValue::String(String::new()));
            }
        }
    }
    if cursor != positional.len() {
        return Err(CommandError::new(
            ":command/unexpected-argument",
            format!("unexpected argument: {}", positional[cursor]),
        ));
    }
    Ok(arguments)
}

fn parse_option(
    option: &OptionSpec,
    inline: Option<String>,
    argv: &[String],
    index: usize,
    output: &mut BTreeMap<String, ParsedValue>,
    supplied: &mut HashSet<String>,
) -> Result<usize, CommandError> {
    match option.kind {
        OptionKind::Boolean => {
            if inline.is_some() {
                return Err(CommandError::new(
                    ":command/invalid-option",
                    format!("{} does not accept a value", option.long_name()),
                ));
            }
            if !supplied.insert(option.id.clone()) {
                return Err(CommandError::new(
                    ":command/duplicate-option",
                    format!("{} may be supplied only once", option.long_name()),
                ));
            }
            output.insert(option.id.clone(), ParsedValue::Boolean(true));
            Ok(index + 1)
        }
        OptionKind::String => {
            let (value, next) = match inline {
                Some(value) => (value, index + 1),
                None => {
                    let value = argv.get(index + 1).cloned().ok_or_else(|| {
                        CommandError::new(
                            ":command/missing-option-value",
                            format!("{} requires a value", option.long_name()),
                        )
                    })?;
                    (value, index + 2)
                }
            };
            if option.many {
                let mut values = match output.remove(&option.id) {
                    Some(ParsedValue::Strings(values)) => values,
                    Some(_) => unreachable!("validated option defaults"),
                    None => Vec::new(),
                };
                values.push(value);
                output.insert(option.id.clone(), ParsedValue::Strings(values));
            } else {
                if !supplied.insert(option.id.clone()) {
                    return Err(CommandError::new(
                        ":command/duplicate-option",
                        format!("{} may be supplied only once", option.long_name()),
                    ));
                }
                output.insert(option.id.clone(), ParsedValue::String(value));
            }
            Ok(next)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App<String> {
        App::create(AppConfig {
            id: "demo/app".into(),
            desc: "Demo app".into(),
        })
        .unwrap()
    }

    fn route(id: &str, path: &[&str]) -> Route<String> {
        Route {
            spec: RouteSpec {
                id: id.into(),
                path: path.iter().map(|part| (*part).into()).collect(),
                aliases: Vec::new(),
                desc: format!("{id} route"),
                options: Vec::new(),
                arguments: Vec::new(),
                passthrough: false,
            },
            handler: id.into(),
        }
    }

    #[test]
    fn parses_nested_routes_and_normalizes_options() {
        let mut app = app();
        let mut test = route("test", &["test"]);
        test.spec.options = vec![
            OptionSpec {
                id: "watch".into(),
                long: None,
                short: Some('w'),
                kind: OptionKind::Boolean,
                many: false,
                default: None,
            },
            OptionSpec {
                id: "namespace".into(),
                long: None,
                short: Some('n'),
                kind: OptionKind::String,
                many: true,
                default: None,
            },
        ];
        test.spec.arguments = vec![ArgumentSpec {
            id: "files".into(),
            required: false,
            many: true,
        }];
        app.install(test).unwrap();
        app.install(route("bundle-run", &["bundle", "run"]))
            .unwrap();

        let request = app
            .parse(vec![
                "test".into(),
                "-w".into(),
                "--namespace=demo.core".into(),
                "-n".into(),
                "demo.util".into(),
                "--".into(),
                "--not-an-option".into(),
            ])
            .unwrap();
        assert_eq!(request.route_id, "test");
        assert_eq!(
            request.options.get("watch"),
            Some(&ParsedValue::Boolean(true))
        );
        assert_eq!(
            request.options.get("namespace"),
            Some(&ParsedValue::Strings(vec![
                "demo.core".into(),
                "demo.util".into()
            ]))
        );
        assert_eq!(
            request.arguments.get("files"),
            Some(&ParsedValue::Strings(vec!["--not-an-option".into()]))
        );
        assert_eq!(
            app.parse(vec!["bundle".into(), "run".into()])
                .unwrap()
                .route_id,
            "bundle-run"
        );
    }

    #[test]
    fn collisions_lifecycle_and_stale_requests_are_deterministic() {
        let mut app = app();
        let handle = app.install(route("test", &["test"])).unwrap();
        let collision = app.install(route("again", &["test"])).unwrap_err();
        assert_eq!(collision.code, ":command/route-conflict");
        let request = app.parse(vec!["test".into()]).unwrap();
        let snapshot = app.snapshot().unwrap();
        assert!(app.uninstall(handle).unwrap());
        assert!(!app.uninstall(handle).unwrap());
        assert_eq!(
            app.handler(&request).unwrap_err().code,
            ":command/stale-request"
        );
        app.restore(snapshot).unwrap();
        assert_eq!(app.routes().unwrap().len(), 1);
        app.reset().unwrap();
        app.reset().unwrap();
        app.close();
        app.close();
        assert!(app.closed());
        assert_eq!(app.routes().unwrap_err().code, ":command/closed");
    }

    #[test]
    fn run_maps_parse_and_handler_errors_to_cli_responses() {
        let mut app = app();
        app.install(route("ok", &["ok"])).unwrap();
        assert_eq!(
            app.run(vec!["missing".into()], |_, _| unreachable!()),
            Response {
                stdout: String::new(),
                stderr: ":command/unknown-route: unknown command: missing\n".into(),
                exit: 2,
            }
        );
        assert_eq!(
            app.run(vec!["ok".into()], |_, _| {
                Ok(Response {
                    stdout: "ok".into(),
                    stderr: String::new(),
                    exit: 0,
                })
            }),
            Response {
                stdout: "ok".into(),
                stderr: String::new(),
                exit: 0,
            }
        );
        assert_eq!(
            app.run(vec!["ok".into()], |_, _| {
                Ok(Response {
                    stdout: String::new(),
                    stderr: String::new(),
                    exit: 256,
                })
            })
            .exit,
            1
        );
    }

    #[test]
    fn passthrough_routes_preserve_delegated_flags_and_restore_advances_handles() {
        let mut app = app();
        let mut delegated = route("delegated", &["delegate"]);
        delegated.spec.passthrough = true;
        delegated.spec.arguments = vec![ArgumentSpec {
            id: "argv".into(),
            required: false,
            many: true,
        }];
        let handle = app.install(delegated).unwrap();
        let snapshot = app.snapshot().unwrap();
        app.reset().unwrap();
        app.restore(snapshot).unwrap();
        let later = app.install(route("later", &["later"])).unwrap();
        assert!(later.id() > handle.id());
        assert_eq!(
            app.parse(vec![
                "delegate".into(),
                "--owned-by-handler".into(),
                "value".into()
            ])
            .unwrap()
            .arguments
            .get("argv"),
            Some(&ParsedValue::Strings(vec![
                "--owned-by-handler".into(),
                "value".into()
            ]))
        );

        let mut duplicate = route("duplicate", &["duplicate"]);
        duplicate.spec.options = vec![OptionSpec {
            id: "name".into(),
            long: None,
            short: None,
            kind: OptionKind::String,
            many: false,
            default: Some(ParsedValue::String(String::new())),
        }];
        app.install(duplicate).unwrap();
        assert_eq!(
            app.parse(vec![
                "duplicate".into(),
                "--name=".into(),
                "--name".into(),
                "again".into(),
            ])
            .unwrap_err()
            .code,
            ":command/duplicate-option"
        );
    }
}
