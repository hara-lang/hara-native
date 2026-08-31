#![cfg(not(target_arch = "wasm32"))]

//! RESP listener and wire codec for the native runtime broker. Wire format,
//! dialects, and operation semantics are specified in
//! `specs/01-lang/007-resp/draft/hal-resp-protocol.md`.

use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::core::ExceptionSite;
use crate::native_cli::{
    Documentation, DocumentationValue, RuntimeBroker, RuntimeDiagnostic, RuntimeException,
};

const MAX_LINE: usize = 64 * 1024;
const MAX_BULK: usize = 64 * 1024 * 1024;
const MAX_NESTING: usize = 64;
const MAX_DIAGNOSTIC_DATA_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug)]
struct RespFailure {
    code: &'static str,
    message: String,
    diagnostic: Option<RespValue>,
}

impl RespFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            diagnostic: None,
        }
    }

    fn evaluation(diagnostic: RuntimeDiagnostic, origin: Option<SourceOrigin>) -> Self {
        let message = diagnostic.message.clone();
        Self {
            code: "EVAL_ERROR",
            message,
            diagnostic: Some(diagnostic_payload(&diagnostic, origin.as_ref())),
        }
    }
}

#[derive(Clone, Debug)]
struct SourceOrigin {
    file: String,
    line: usize,
    column: usize,
    source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RespValue {
    Simple(String),
    Error(String),
    Integer(i64),
    Bulk(Option<Vec<u8>>),
    Array(Option<Vec<RespValue>>),
}

impl RespValue {
    pub fn text(&self) -> Option<String> {
        match self {
            Self::Simple(value) | Self::Error(value) => Some(value.clone()),
            Self::Integer(value) => Some(value.to_string()),
            Self::Bulk(Some(value)) => String::from_utf8(value.clone()).ok(),
            _ => None,
        }
    }

    pub fn bulk(value: impl Into<String>) -> Self {
        Self::Bulk(Some(value.into().into_bytes()))
    }

    pub fn array(values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Array(Some(
            values
                .into_iter()
                .map(|value| Self::bulk(value.into()))
                .collect(),
        ))
    }
}

pub struct RespConnection {
    input: BufReader<TcpStream>,
    output: BufWriter<TcpStream>,
}

impl RespConnection {
    pub fn new(stream: TcpStream) -> Result<Self, String> {
        let output = stream
            .try_clone()
            .map(BufWriter::new)
            .map_err(|error| format!("RESP socket clone failed: {error}"))?;
        Ok(Self {
            input: BufReader::new(stream),
            output,
        })
    }

    pub fn read(&mut self) -> Result<Option<RespValue>, String> {
        let mut prefix = [0_u8; 1];
        match self.input.read_exact(&mut prefix) {
            Ok(()) => self.read_after_prefix(prefix[0], 0).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(error) => Err(format!("RESP read failed: {error}")),
        }
    }

    fn read_after_prefix(&mut self, prefix: u8, depth: usize) -> Result<RespValue, String> {
        if depth > MAX_NESTING {
            return Err("RESP nesting limit exceeded".into());
        }
        match prefix {
            b'+' => Ok(RespValue::Simple(self.line()?)),
            b'-' => Ok(RespValue::Error(self.line()?)),
            b':' => self
                .line()?
                .parse::<i64>()
                .map(RespValue::Integer)
                .map_err(|_| "Invalid RESP integer".into()),
            b'$' => {
                let length = self.length()?;
                if length < 0 {
                    return Ok(RespValue::Bulk(None));
                }
                let length = usize::try_from(length).map_err(|_| "Invalid RESP length")?;
                if length > MAX_BULK {
                    return Err("RESP bulk limit exceeded".into());
                }
                let mut bytes = vec![0; length];
                self.input
                    .read_exact(&mut bytes)
                    .map_err(|error| format!("RESP read failed: {error}"))?;
                self.crlf()?;
                Ok(RespValue::Bulk(Some(bytes)))
            }
            b'*' => {
                let length = self.length()?;
                if length < 0 {
                    return Ok(RespValue::Array(None));
                }
                let length = usize::try_from(length).map_err(|_| "Invalid RESP length")?;
                if length > MAX_LINE {
                    return Err("RESP array limit exceeded".into());
                }
                let mut values = Vec::with_capacity(length);
                for _ in 0..length {
                    let mut prefix = [0_u8; 1];
                    self.input
                        .read_exact(&mut prefix)
                        .map_err(|error| format!("RESP read failed: {error}"))?;
                    values.push(self.read_after_prefix(prefix[0], depth + 1)?);
                }
                Ok(RespValue::Array(Some(values)))
            }
            _ => Err("Unknown RESP type".into()),
        }
    }

    fn length(&mut self) -> Result<i64, String> {
        self.line()?
            .parse()
            .map_err(|_| "Invalid RESP length".into())
    }

    fn line(&mut self) -> Result<String, String> {
        let mut bytes = Vec::new();
        let read = self
            .input
            .read_until(b'\n', &mut bytes)
            .map_err(|error| format!("RESP read failed: {error}"))?;
        if read < 2 || bytes[read - 2..] != *b"\r\n" {
            return Err("Invalid RESP line ending".into());
        }
        if bytes.len() > MAX_LINE {
            return Err("RESP line limit exceeded".into());
        }
        bytes.truncate(read - 2);
        String::from_utf8(bytes).map_err(|_| "RESP line is not UTF-8".into())
    }

    fn crlf(&mut self) -> Result<(), String> {
        let mut ending = [0_u8; 2];
        self.input
            .read_exact(&mut ending)
            .map_err(|error| format!("RESP read failed: {error}"))?;
        if ending != *b"\r\n" {
            return Err("Invalid RESP bulk ending".into());
        }
        Ok(())
    }

    pub fn write(&mut self, value: &RespValue) -> Result<(), String> {
        write_value(&mut self.output, value)?;
        self.output
            .flush()
            .map_err(|error| format!("RESP write failed: {error}"))
    }
}

fn write_value(output: &mut impl Write, value: &RespValue) -> Result<(), String> {
    match value {
        RespValue::Simple(value) => line_value(output, b'+', value),
        RespValue::Error(value) => line_value(output, b'-', value),
        RespValue::Integer(value) => line_value(output, b':', &value.to_string()),
        RespValue::Bulk(None) => output
            .write_all(b"$-1\r\n")
            .map_err(|error| format!("RESP write failed: {error}")),
        RespValue::Bulk(Some(bytes)) => output
            .write_all(format!("${}\r\n", bytes.len()).as_bytes())
            .and_then(|_| output.write_all(bytes))
            .and_then(|_| output.write_all(b"\r\n"))
            .map_err(|error| format!("RESP write failed: {error}")),
        RespValue::Array(None) => output
            .write_all(b"*-1\r\n")
            .map_err(|error| format!("RESP write failed: {error}")),
        RespValue::Array(Some(values)) => {
            output
                .write_all(format!("*{}\r\n", values.len()).as_bytes())
                .map_err(|error| format!("RESP write failed: {error}"))?;
            for value in values {
                write_value(output, value)?;
            }
            Ok(())
        }
    }
}

fn line_value(output: &mut impl Write, prefix: u8, value: &str) -> Result<(), String> {
    if value.contains(['\r', '\n']) {
        return Err("RESP line values cannot contain CR or LF".into());
    }
    output
        .write_all(&[prefix])
        .and_then(|_| output.write_all(value.as_bytes()))
        .and_then(|_| output.write_all(b"\r\n"))
        .map_err(|error| format!("RESP write failed: {error}"))
}

pub struct RespServer {
    host: String,
    port: u16,
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RespServer {
    pub fn start(host: &str, port: u16, broker: RuntimeBroker) -> Result<Self, String> {
        let listener = TcpListener::bind((host, port))
            .map_err(|error| format!("RESP bind {host}:{port} failed: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("RESP address failed: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("RESP listener setup failed: {error}"))?;
        let running = Arc::new(AtomicBool::new(true));
        let active = running.clone();
        let instance = format!("RUST-{}-{}", std::process::id(), address.port());
        let root = std::env::current_dir()
            .unwrap_or_default()
            .display()
            .to_string();
        let thread = std::thread::Builder::new()
            .name("hara-resp-listener".into())
            .spawn(move || {
                while active.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if stream.set_nonblocking(false).is_err() {
                                continue;
                            }
                            let broker = broker.clone();
                            let instance = instance.clone();
                            let root = root.clone();
                            let _ = std::thread::Builder::new()
                                .name("hara-resp-client".into())
                                .spawn(move || serve(stream, broker, &instance, &root));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|error| format!("RESP listener thread failed: {error}"))?;
        Ok(Self {
            host: host.into(),
            port: address.port(),
            running,
            thread: Some(thread),
        })
    }

    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for RespServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn serve(stream: TcpStream, broker: RuntimeBroker, instance: &str, root: &str) {
    let Ok(mut connection) = RespConnection::new(stream) else {
        return;
    };
    let mut protocol = 3_u8;
    let mut attached = "ROOT".to_owned();
    loop {
        let request = match connection.read() {
            Ok(Some(RespValue::Array(Some(values)))) => values,
            Ok(Some(_)) => {
                let _ = connection.write(&RespValue::Error("BAD_REQUEST expected array".into()));
                continue;
            }
            Ok(None) => return,
            Err(error) => {
                let _ = connection.write(&RespValue::Error(format!("BAD_REQUEST {error}")));
                continue;
            }
        };
        let words = request
            .iter()
            .map(RespValue::text)
            .collect::<Option<Vec<_>>>();
        let Some(words) = words else {
            let _ = connection.write(&RespValue::Error(
                "BAD_REQUEST textual arguments required".into(),
            ));
            continue;
        };
        if words.is_empty() {
            continue;
        }
        let operation = words[0].to_ascii_uppercase();
        if operation == "QUIT" {
            let _ = connection.write(&RespValue::Simple("OK".into()));
            return;
        }
        if operation == "HELLO" {
            protocol = words
                .get(1)
                .and_then(|value| value.parse().ok())
                .unwrap_or(3);
            let hello = RespValue::array([
                "SERVER",
                "HARA",
                "INSTANCE",
                instance,
                "PROTOCOL",
                &protocol.to_string(),
                "ROOT",
                root,
            ]);
            let _ = connection.write(&hello);
            continue;
        }
        if protocol >= 4 {
            let id = words.get(1).cloned().unwrap_or_else(|| "?".into());
            handle_v4(
                &mut connection,
                &broker,
                &mut attached,
                &operation,
                &id,
                &words[2..],
            );
        } else {
            handle_legacy(
                &mut connection,
                &broker,
                &mut attached,
                &operation,
                &words[1..],
            );
        }
    }
}

fn handle_v4(
    connection: &mut RespConnection,
    broker: &RuntimeBroker,
    attached: &mut String,
    operation: &str,
    id: &str,
    arguments: &[String],
) {
    let result = operation_result(broker, attached, operation, arguments);
    match result {
        Ok(value) => {
            let _ = connection.write(&RespValue::Array(Some(vec![
                RespValue::bulk("RESULT"),
                RespValue::bulk(id),
                value,
            ])));
            let _ = connection.write(&RespValue::array(["DONE", id, "OK"]));
        }
        Err(failure) => {
            let mut frame = vec![
                RespValue::bulk("ERROR"),
                RespValue::bulk(id),
                RespValue::bulk(failure.code),
                RespValue::bulk(failure.message),
            ];
            if let Some(diagnostic) = failure.diagnostic {
                frame.push(diagnostic);
            }
            let _ = connection.write(&RespValue::Array(Some(frame)));
            let _ = connection.write(&RespValue::array(["DONE", id, "ERROR"]));
        }
    }
}

fn handle_legacy(
    connection: &mut RespConnection,
    broker: &RuntimeBroker,
    attached: &mut String,
    operation: &str,
    arguments: &[String],
) {
    let result = if operation == "EVAL" && arguments.len() >= 2 {
        broker
            .eval(&arguments[0], &arguments[1])
            .map(RespValue::bulk)
            .map_err(|message| RespFailure::new("EVAL_ERROR", message))
    } else {
        operation_result(broker, attached, operation, arguments)
    };
    let response = match result {
        Ok(value) => legacy_value(value),
        Err(failure) => RespValue::Error(format!("{} {}", failure.code, failure.message)),
    };
    let _ = connection.write(&response);
}

fn operation_result(
    broker: &RuntimeBroker,
    attached: &mut String,
    operation: &str,
    arguments: &[String],
) -> Result<RespValue, RespFailure> {
    match operation {
        "EVAL" => {
            let source = arguments
                .first()
                .ok_or_else(|| RespFailure::new("BAD_REQUEST", "EVAL requires source"))?;
            broker
                .eval_diagnostic(attached, source)
                .map(RespValue::bulk)
                .map_err(|diagnostic| RespFailure::evaluation(diagnostic, eval_origin(arguments)))
        }
        "COMPLETE" => {
            let prefix = arguments.first().map_or("", String::as_str);
            broker
                .complete(attached, prefix)
                .map(RespValue::array)
                .map_err(|error| RespFailure::new("NO_SESSION", error))
        }
        "DOC" => {
            let symbol = arguments
                .first()
                .ok_or_else(|| RespFailure::new("BAD_REQUEST", "DOC requires symbol"))?;
            broker
                .documentation(attached, symbol)
                .map(documentation_value)
                .map_err(|error| {
                    if error.starts_with("No session:") {
                        RespFailure::new("NO_SESSION", error)
                    } else {
                        RespFailure::new("DOC_NOT_FOUND", error)
                    }
                })
        }
        "SESSION" => session_operation(broker, attached, arguments),
        "COMMANDS" => Ok(RespValue::bulk(
            "HELLO EVAL COMPLETE DOC SESSION COMMANDS INFO QUIT",
        )),
        "INFO" => broker
            .info(attached)
            .map(RespValue::bulk)
            .map_err(|error| RespFailure::new("NO_SESSION", error)),
        _ => Err(RespFailure::new(
            "UNKNOWN_OP",
            format!("Unknown operation: {operation}"),
        )),
    }
}

fn session_operation(
    broker: &RuntimeBroker,
    attached: &mut String,
    arguments: &[String],
) -> Result<RespValue, RespFailure> {
    let action = arguments
        .first()
        .map(|value| value.to_ascii_uppercase())
        .ok_or_else(|| RespFailure::new("BAD_REQUEST", "SESSION requires an action"))?;
    match action.as_str() {
        "NEW" => broker
            .create(
                arguments
                    .get(1)
                    .ok_or_else(|| RespFailure::new("BAD_REQUEST", "SESSION NEW requires name"))?,
            )
            .map(RespValue::bulk)
            .map_err(|error| RespFailure::new("BAD_REQUEST", error)),
        "LIST" => broker
            .list()
            .map(RespValue::array)
            .map_err(|error| RespFailure::new("INTERNAL_ERROR", error)),
        "ATTACH" => {
            let name = arguments
                .get(1)
                .ok_or_else(|| RespFailure::new("BAD_REQUEST", "SESSION ATTACH requires name"))?;
            broker
                .info(name)
                .map_err(|error| RespFailure::new("NO_SESSION", error))?;
            *attached = name.clone();
            Ok(RespValue::bulk(name))
        }
        "DETACH" => {
            *attached = "ROOT".into();
            Ok(RespValue::bulk("ROOT"))
        }
        "INFO" => broker
            .info(attached)
            .map(RespValue::bulk)
            .map_err(|error| RespFailure::new("NO_SESSION", error)),
        "CLOSE" => {
            broker
                .close(arguments.get(1).ok_or_else(|| {
                    RespFailure::new("BAD_REQUEST", "SESSION CLOSE requires name")
                })?)
                .map(RespValue::bulk)
                .map_err(|error| RespFailure::new("BAD_REQUEST", error))
        }
        _ => Err(RespFailure::new(
            "BAD_REQUEST",
            format!("Unknown SESSION action: {action}"),
        )),
    }
}

fn eval_origin(arguments: &[String]) -> Option<SourceOrigin> {
    let source = arguments.first()?.clone();
    let mut file = None;
    let mut line = None;
    let mut column = None;
    for pair in arguments[1..].chunks_exact(2) {
        match pair[0].to_ascii_uppercase().as_str() {
            "FILE" => file = Some(pair[1].clone()),
            "LINE" => line = pair[1].parse::<usize>().ok().filter(|value| *value > 0),
            "COLUMN" => column = pair[1].parse::<usize>().ok().filter(|value| *value > 0),
            _ => {}
        }
    }
    Some(SourceOrigin {
        file: file?,
        line: line?,
        column: column.unwrap_or(1),
        source,
    })
}

fn truncated_text(value: String) -> String {
    if value.len() <= MAX_DIAGNOSTIC_DATA_BYTES {
        return value;
    }
    let mut end = MAX_DIAGNOSTIC_DATA_BYTES.saturating_sub(3);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &value[..end])
}

fn optional_bulk(value: Option<String>) -> RespValue {
    value.map_or(RespValue::Bulk(None), RespValue::bulk)
}

fn optional_integer(value: Option<usize>) -> RespValue {
    value.map_or(RespValue::Bulk(None), |value| {
        RespValue::Integer(value as i64)
    })
}

fn exception_site(exception: &RuntimeException) -> Option<ExceptionSite> {
    exception.throws.last().cloned()
}

fn site_location(
    site: Option<&ExceptionSite>,
    origin: Option<&SourceOrigin>,
    use_origin: bool,
) -> (Option<String>, Option<usize>, Option<usize>) {
    let Some(site) = site else {
        return origin
            .filter(|_| use_origin)
            .map(|origin| {
                (
                    Some(origin.file.clone()),
                    Some(origin.line),
                    Some(origin.column),
                )
            })
            .unwrap_or((None, None, None));
    };
    if let Some(resource) = &site.resource {
        return (
            Some(resource.clone()),
            (site.line > 0).then_some(site.line),
            (site.column > 0).then_some(site.column),
        );
    }
    if use_origin {
        if let Some(origin) = origin {
            let line = (site.line > 0).then(|| origin.line + site.line - 1);
            let column = if site.line <= 1 {
                (site.column > 0).then(|| origin.column + site.column - 1)
            } else {
                (site.column > 0).then_some(site.column)
            };
            return (Some(origin.file.clone()), line, column);
        }
    }
    (
        None,
        (site.line > 0).then_some(site.line),
        (site.column > 0).then_some(site.column),
    )
}

fn location_payload(site: Option<&ExceptionSite>, origin: Option<&SourceOrigin>) -> RespValue {
    let use_origin = site.is_none_or(|site| site.resource.is_none());
    let (file, line, column) = site_location(site, origin, use_origin);
    RespValue::Array(Some(vec![
        RespValue::bulk("FILE"),
        optional_bulk(file),
        RespValue::bulk("LINE"),
        optional_integer(line),
        RespValue::bulk("COLUMN"),
        optional_integer(column),
    ]))
}

fn exception_payload(exception: &RuntimeException) -> RespValue {
    let class = exception.class.clone().map(truncated_text);
    let code = exception.code.clone().map(truncated_text);
    let cause = exception.cause.as_deref().map(exception_payload);
    let throws = exception
        .throws
        .iter()
        .map(|site| location_payload(Some(site), None))
        .collect::<Vec<_>>();
    RespValue::Array(Some(vec![
        RespValue::bulk("MESSAGE"),
        RespValue::bulk(truncated_text(exception.message.clone())),
        RespValue::bulk("CLASS"),
        optional_bulk(class),
        RespValue::bulk("CODE"),
        optional_bulk(code),
        RespValue::bulk("DATA"),
        RespValue::bulk(truncated_text(exception.data.clone())),
        RespValue::bulk("CAUSE"),
        cause.unwrap_or(RespValue::Bulk(None)),
        RespValue::bulk("THROWS"),
        RespValue::Array(Some(throws)),
    ]))
}

fn frame_payload(frame: &crate::core::TraceFrame, origin: Option<&SourceOrigin>) -> RespValue {
    let use_origin = frame.namespace.is_none()
        && frame
            .site
            .as_ref()
            .is_none_or(|site| site.resource.is_none());
    let (file, line, column) = site_location(frame.site.as_ref(), origin, use_origin);
    RespValue::Array(Some(vec![
        RespValue::bulk("FUNCTION"),
        RespValue::bulk(frame.name.clone()),
        RespValue::bulk("NAMESPACE"),
        optional_bulk(frame.namespace.clone()),
        RespValue::bulk("FILE"),
        optional_bulk(file),
        RespValue::bulk("LINE"),
        optional_integer(line),
        RespValue::bulk("COLUMN"),
        optional_integer(column),
    ]))
}

fn evaluation_frame_payload(origin: &SourceOrigin) -> RespValue {
    RespValue::Array(Some(vec![
        RespValue::bulk("FUNCTION"),
        RespValue::bulk("<eval>"),
        RespValue::bulk("NAMESPACE"),
        RespValue::Bulk(None),
        RespValue::bulk("FILE"),
        RespValue::bulk(origin.file.clone()),
        RespValue::bulk("LINE"),
        RespValue::Integer(origin.line as i64),
        RespValue::bulk("COLUMN"),
        RespValue::Integer(origin.column as i64),
    ]))
}

fn source_excerpt(origin: Option<&SourceOrigin>, line: Option<usize>) -> RespValue {
    let Some(origin) = origin else {
        return RespValue::Bulk(None);
    };
    let Some(line) = line else {
        return RespValue::Bulk(None);
    };
    let local_line = line.checked_sub(origin.line).map_or(0, |offset| offset + 1);
    if local_line == 0 {
        return RespValue::Bulk(None);
    }
    let lines = origin.source.lines().collect::<Vec<_>>();
    if local_line > lines.len() {
        return RespValue::Bulk(None);
    }
    let start = local_line.saturating_sub(3);
    let end = usize::min(lines.len(), local_line + 2);
    let text = lines[start..end].join("\n");
    RespValue::Array(Some(vec![
        RespValue::bulk("START-LINE"),
        RespValue::Integer((origin.line + start) as i64),
        RespValue::bulk("TEXT"),
        RespValue::bulk(truncated_text(text)),
    ]))
}

fn diagnostic_payload(diagnostic: &RuntimeDiagnostic, origin: Option<&SourceOrigin>) -> RespValue {
    let exception = diagnostic.exception.as_ref();
    let primary_site = exception.and_then(exception_site).or_else(|| {
        diagnostic
            .frames
            .iter()
            .rev()
            .find_map(|frame| frame.site.clone())
    });
    let (_, primary_line, _) = site_location(primary_site.as_ref(), origin, true);
    let mut frames = diagnostic
        .frames
        .iter()
        .rev()
        .map(|frame| frame_payload(frame, origin))
        .collect::<Vec<_>>();
    if frames.is_empty() {
        if let Some(origin) = origin {
            frames.push(evaluation_frame_payload(origin));
        }
    }
    RespValue::Array(Some(vec![
        RespValue::bulk("VERSION"),
        RespValue::Integer(1),
        RespValue::bulk("MESSAGE"),
        RespValue::bulk(truncated_text(diagnostic.message.clone())),
        RespValue::bulk("EXCEPTION"),
        exception.map_or(RespValue::Bulk(None), exception_payload),
        RespValue::bulk("PRIMARY"),
        location_payload(primary_site.as_ref(), origin),
        RespValue::bulk("EXCERPT"),
        source_excerpt(origin, primary_line),
        RespValue::bulk("FRAMES"),
        RespValue::Array(Some(frames)),
    ]))
}

fn documentation_part(value: DocumentationValue) -> RespValue {
    match value {
        DocumentationValue::Nil => RespValue::Bulk(None),
        DocumentationValue::Boolean(value) => RespValue::bulk(value.to_string()),
        DocumentationValue::Integer(value) => RespValue::Integer(value),
        DocumentationValue::String(value) => RespValue::bulk(value),
        DocumentationValue::Array(values) => {
            RespValue::Array(Some(values.into_iter().map(documentation_part).collect()))
        }
    }
}

fn documentation_value(documentation: Documentation) -> RespValue {
    RespValue::Array(Some(vec![
        RespValue::bulk("SYMBOL"),
        RespValue::bulk(documentation.symbol),
        RespValue::bulk("DOC"),
        documentation
            .doc
            .map_or(RespValue::Bulk(None), RespValue::bulk),
        RespValue::bulk("ARGLISTS"),
        documentation_part(documentation.arglists),
        RespValue::bulk("FILE"),
        documentation
            .file
            .map_or(RespValue::Bulk(None), RespValue::bulk),
        RespValue::bulk("LINE"),
        documentation
            .line
            .map_or(RespValue::Bulk(None), RespValue::Integer),
        RespValue::bulk("COLUMN"),
        documentation
            .column
            .map_or(RespValue::Bulk(None), RespValue::Integer),
    ]))
}

fn legacy_value(value: RespValue) -> RespValue {
    match value {
        RespValue::Array(Some(values)) => RespValue::bulk(
            values
                .into_iter()
                .filter_map(|value| value.text())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        value => value,
    }
}

#[cfg(test)]
mod tests {
    use super::{RespConnection, RespValue};
    use std::net::{TcpListener, TcpStream};

    fn array(value: &RespValue) -> &[RespValue] {
        match value {
            RespValue::Array(Some(values)) => values,
            value => panic!("expected RESP array, got {value:?}"),
        }
    }

    fn field<'a>(values: &'a [RespValue], name: &str) -> &'a RespValue {
        values
            .chunks_exact(2)
            .find_map(|pair| (pair[0].text().as_deref() == Some(name)).then_some(&pair[1]))
            .unwrap_or_else(|| panic!("missing {name} in {values:?}"))
    }

    #[test]
    fn resp2_values_round_trip() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let writer = std::thread::spawn(move || {
            let mut connection = RespConnection::new(TcpStream::connect(address).unwrap()).unwrap();
            connection
                .write(&RespValue::Array(Some(vec![
                    RespValue::Simple("OK".into()),
                    RespValue::Integer(42),
                    RespValue::Bulk(None),
                    RespValue::bulk("hello"),
                ])))
                .unwrap();
        });
        let (stream, _) = listener.accept().unwrap();
        let mut connection = RespConnection::new(stream).unwrap();
        assert_eq!(
            connection.read().unwrap().unwrap(),
            RespValue::Array(Some(vec![
                RespValue::Simple("OK".into()),
                RespValue::Integer(42),
                RespValue::Bulk(None),
                RespValue::bulk("hello"),
            ]))
        );
        writer.join().unwrap();
    }
    #[test]
    fn server_streams_protocol_four_and_shares_root_with_legacy_clients() {
        let broker = crate::native_cli::RuntimeBroker::start().unwrap();
        broker.eval("ROOT", "(def answer 41)").unwrap();
        let mut server = super::RespServer::start("127.0.0.1", 0, broker).unwrap();
        let endpoint = server.endpoint();

        let mut legacy = RespConnection::new(TcpStream::connect(&endpoint).unwrap()).unwrap();
        legacy
            .write(&RespValue::array(["EVAL", "ROOT", "(+ answer 1)"]))
            .unwrap();
        assert_eq!(legacy.read().unwrap().unwrap().text().unwrap(), "42");
        legacy
            .write(&RespValue::array([
                "EVAL",
                "ROOT",
                "(throw (ex :test/failed {:value 41}))",
            ]))
            .unwrap();
        assert!(matches!(
            legacy.read().unwrap().unwrap(),
            RespValue::Error(_)
        ));

        let mut modern = RespConnection::new(TcpStream::connect(&endpoint).unwrap()).unwrap();
        modern.write(&RespValue::array(["HELLO", "4"])).unwrap();
        let hello = modern.read().unwrap().unwrap();
        assert!(matches!(hello, RespValue::Array(Some(_))));
        modern
            .write(&RespValue::array(["EVAL", "REQ-1", "answer"]))
            .unwrap();
        assert_eq!(
            modern.read().unwrap().unwrap(),
            RespValue::array(["RESULT", "REQ-1", "41"])
        );
        assert_eq!(
            modern.read().unwrap().unwrap(),
            RespValue::array(["DONE", "REQ-1", "OK"])
        );
        modern
            .write(&RespValue::array(["COMPLETE", "REQ-2", "ans"]))
            .unwrap();
        assert_eq!(
            modern.read().unwrap().unwrap(),
            RespValue::Array(Some(vec![
                RespValue::bulk("RESULT"),
                RespValue::bulk("REQ-2"),
                RespValue::array(["answer"]),
            ]))
        );
        assert_eq!(
            modern.read().unwrap().unwrap(),
            RespValue::array(["DONE", "REQ-2", "OK"])
        );
        modern
            .write(&RespValue::array([
                "EVAL",
                "REQ-3",
                concat!(
                    "(defn ^{:file \"/tmp/sample.hal\" :line 12 :column 3} located ",
                    "\"A located function.\" [value] value)"
                ),
            ]))
            .unwrap();
        modern.read().unwrap().unwrap();
        modern.read().unwrap().unwrap();
        modern
            .write(&RespValue::array(["DOC", "REQ-4", "located"]))
            .unwrap();
        assert_eq!(
            modern.read().unwrap().unwrap(),
            RespValue::Array(Some(vec![
                RespValue::bulk("RESULT"),
                RespValue::bulk("REQ-4"),
                RespValue::Array(Some(vec![
                    RespValue::bulk("SYMBOL"),
                    RespValue::bulk("located"),
                    RespValue::bulk("DOC"),
                    RespValue::bulk("A located function."),
                    RespValue::bulk("ARGLISTS"),
                    RespValue::Array(Some(vec![RespValue::array(["value"])])),
                    RespValue::bulk("FILE"),
                    RespValue::bulk("/tmp/sample.hal"),
                    RespValue::bulk("LINE"),
                    RespValue::Integer(12),
                    RespValue::bulk("COLUMN"),
                    RespValue::Integer(3),
                ])),
            ]))
        );
        assert_eq!(
            modern.read().unwrap().unwrap(),
            RespValue::array(["DONE", "REQ-4", "OK"])
        );
        server.stop();
    }

    #[test]
    fn server_v4_error_carries_a_structured_evaluation_diagnostic() {
        let broker = crate::native_cli::RuntimeBroker::start().unwrap();
        let mut server = super::RespServer::start("127.0.0.1", 0, broker).unwrap();
        let endpoint = server.endpoint();
        let mut client = RespConnection::new(TcpStream::connect(&endpoint).unwrap()).unwrap();
        client.write(&RespValue::array(["HELLO", "4"])).unwrap();
        client.read().unwrap().unwrap();

        let source = "(defn boom [] (throw (ex :test/failed {:value 41})))\n(boom)";
        client
            .write(&RespValue::array([
                "EVAL",
                "REQ-ERROR",
                source,
                "FILE",
                "/tmp/request.hal",
                "LINE",
                "10",
                "COLUMN",
                "5",
            ]))
            .unwrap();
        let error = client.read().unwrap().unwrap();
        let error_values = array(&error);
        assert_eq!(error_values.len(), 5);
        assert_eq!(error_values[0].text().as_deref(), Some("ERROR"));
        assert_eq!(error_values[1].text().as_deref(), Some("REQ-ERROR"));
        assert_eq!(error_values[2].text().as_deref(), Some("EVAL_ERROR"));

        let diagnostic = array(&error_values[4]);
        assert_eq!(field(diagnostic, "VERSION"), &RespValue::Integer(1));
        let exception = array(field(diagnostic, "EXCEPTION"));
        assert_eq!(
            field(exception, "CODE").text().as_deref(),
            Some(":test/failed")
        );
        assert!(field(exception, "DATA")
            .text()
            .is_some_and(|data| data.contains(":value 41")));
        let primary = array(field(diagnostic, "PRIMARY"));
        assert_eq!(
            field(primary, "FILE").text().as_deref(),
            Some("/tmp/request.hal")
        );
        assert_eq!(field(primary, "LINE"), &RespValue::Integer(10));
        let excerpt = array(field(diagnostic, "EXCERPT"));
        assert_eq!(field(excerpt, "START-LINE"), &RespValue::Integer(10));
        assert!(field(excerpt, "TEXT")
            .text()
            .is_some_and(|text| text.contains("(boom)")));
        assert!(!array(field(diagnostic, "FRAMES")).is_empty());
        assert_eq!(
            client.read().unwrap().unwrap(),
            RespValue::array(["DONE", "REQ-ERROR", "ERROR"])
        );
        server.stop();
    }

    #[test]
    fn server_v4_validation_errors_carry_a_clickable_evaluation_location() {
        let broker = crate::native_cli::RuntimeBroker::start().unwrap();
        let mut server = super::RespServer::start("127.0.0.1", 0, broker).unwrap();
        let endpoint = server.endpoint();
        let mut client = RespConnection::new(TcpStream::connect(&endpoint).unwrap()).unwrap();
        client.write(&RespValue::array(["HELLO", "4"])).unwrap();
        client.read().unwrap().unwrap();

        client
            .write(&RespValue::array([
                "EVAL",
                "REQ-VALIDATION-ERROR",
                "(ex :unknown {})",
                "FILE",
                "/tmp/validation.hal",
                "LINE",
                "12",
                "COLUMN",
                "3",
            ]))
            .unwrap();
        let error = client.read().unwrap().unwrap();
        let error_values = array(&error);
        assert_eq!(error_values.len(), 5);
        assert_eq!(error_values[2].text().as_deref(), Some("EVAL_ERROR"));
        let diagnostic = array(&error_values[4]);
        let primary = array(field(diagnostic, "PRIMARY"));
        assert_eq!(
            field(primary, "FILE").text().as_deref(),
            Some("/tmp/validation.hal")
        );
        assert_eq!(field(primary, "LINE"), &RespValue::Integer(12));
        assert_eq!(field(primary, "COLUMN"), &RespValue::Integer(3));
        assert!(field(array(field(diagnostic, "EXCERPT")), "TEXT")
            .text()
            .is_some_and(|text| text.contains("(ex :unknown {})")));
        assert!(!array(field(diagnostic, "FRAMES")).is_empty());
        assert_eq!(
            client.read().unwrap().unwrap(),
            RespValue::array(["DONE", "REQ-VALIDATION-ERROR", "ERROR"])
        );
        server.stop();
    }
}
