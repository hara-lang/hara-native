#![cfg(not(target_arch = "wasm32"))]

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread::JoinHandle;

use crate::core::{ExtensionValue, Promise, Value};

struct Record {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout_thread: Option<JoinHandle<Vec<u8>>>,
    stderr_thread: Option<JoinHandle<Vec<u8>>>,
    stdout_chunks: Receiver<Vec<u8>>,
    stderr_chunks: Receiver<Vec<u8>>,
    stdout_stream_taken: bool,
    stderr_stream_taken: bool,
    stdout: Option<Vec<u8>>,
    stderr: Option<Vec<u8>>,
    exit: Option<i32>,
}

#[derive(Default)]
struct State {
    next: u64,
    records: HashMap<u64, Record>,
}

thread_local! {
    static PROCESSES: RefCell<State> = RefCell::new(State::default());
}

pub(crate) fn is_process(value: &Value) -> bool {
    matches!(
        value,
        Value::Extension(value)
            if value.provider == "std.native.Process" && value.type_name == "Process"
    )
}

fn handle(value: &Value, operation: &str) -> Result<u64, String> {
    match value {
        Value::Extension(value)
            if value.provider == "std.native.Process" && value.type_name == "Process" =>
        {
            Ok(value.handle)
        }
        _ => Err(format!("{operation} expects a process")),
    }
}

pub(crate) fn spawn(
    argv: &[String],
    cwd: Option<&str>,
    environment: &[(String, String)],
) -> Result<Value, String> {
    let Some(program) = argv.first() else {
        return Err("os/spawn expects a non-empty argv".into());
    };
    let mut command = Command::new(program);
    command
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.envs(environment.iter().cloned());
    let mut child = command
        .spawn()
        .map_err(|error| format!("os/spawn failed: {error}"))?;
    let stdin = child.stdin.take();
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "os/spawn missing stdout".to_owned())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "os/spawn missing stderr".to_owned())?;
    let (stdout_tx, stdout_chunks) = mpsc::channel();
    let (stderr_tx, stderr_chunks) = mpsc::channel();
    let stdout_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 8192];
        while let Ok(count) = stdout.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let chunk = buffer[..count].to_vec();
            bytes.extend_from_slice(&chunk);
            let _ = stdout_tx.send(chunk);
        }
        bytes
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 8192];
        while let Ok(count) = stderr.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let chunk = buffer[..count].to_vec();
            bytes.extend_from_slice(&chunk);
            let _ = stderr_tx.send(chunk);
        }
        bytes
    });
    let handle = PROCESSES.with(|state| {
        let mut state = state.borrow_mut();
        state.next += 1;
        let handle = state.next;
        state.records.insert(
            handle,
            Record {
                child,
                stdin,
                stdout_thread: Some(stdout_thread),
                stderr_thread: Some(stderr_thread),
                stdout_chunks,
                stderr_chunks,
                stdout_stream_taken: false,
                stderr_stream_taken: false,
                stdout: None,
                stderr: None,
                exit: None,
            },
        );
        handle
    });
    Ok(Value::Extension(ExtensionValue {
        provider: "std.native.Process".into(),
        type_name: "Process".into(),
        handle,
    }))
}

fn finish(handle: u64, wait: bool) -> Result<Option<(i32, Vec<u8>, Vec<u8>)>, String> {
    PROCESSES.with(|state| {
        let mut state = state.borrow_mut();
        let process = state
            .records
            .get_mut(&handle)
            .ok_or_else(|| "os/process: unknown process".to_owned())?;
        if process.exit.is_none() {
            let status = if wait {
                Some(
                    process
                        .child
                        .wait()
                        .map_err(|error| format!("os/process-wait failed: {error}"))?,
                )
            } else {
                process
                    .child
                    .try_wait()
                    .map_err(|error| format!("os/process-wait failed: {error}"))?
            };
            if let Some(status) = status {
                process.exit = Some(status.code().unwrap_or(-1));
                process.stdout = Some(
                    process
                        .stdout_thread
                        .take()
                        .ok_or_else(|| "os/process-stdout reader unavailable".to_owned())?
                        .join()
                        .map_err(|_| "os/process-stdout reader failed".to_owned())?,
                );
                process.stderr = Some(
                    process
                        .stderr_thread
                        .take()
                        .ok_or_else(|| "os/process-stderr reader unavailable".to_owned())?
                        .join()
                        .map_err(|_| "os/process-stderr reader failed".to_owned())?,
                );
            }
        }
        Ok(process.exit.map(|exit| {
            (
                exit,
                process.stdout.clone().unwrap_or_default(),
                process.stderr.clone().unwrap_or_default(),
            )
        }))
    })
}

pub(crate) fn alive(value: &Value) -> Result<bool, String> {
    Ok(finish(handle(value, "os/process-alive?")?, false)?.is_none())
}

pub(crate) fn write(value: &Value, bytes: &[u8]) -> Result<usize, String> {
    let handle = handle(value, "os/process-write")?;
    PROCESSES.with(|state| {
        let mut state = state.borrow_mut();
        let stdin = state
            .records
            .get_mut(&handle)
            .and_then(|process| process.stdin.as_mut())
            .ok_or_else(|| "os/process-write: input is closed".to_owned())?;
        stdin
            .write_all(bytes)
            .and_then(|()| stdin.flush())
            .map_err(|error| format!("os/process-write failed: {error}"))?;
        Ok(bytes.len())
    })
}

pub(crate) fn close_input(value: &Value) -> Result<(), String> {
    let handle = handle(value, "os/process-close-input")?;
    PROCESSES.with(|state| {
        let mut state = state.borrow_mut();
        let process = state
            .records
            .get_mut(&handle)
            .ok_or_else(|| "os/process-close-input: unknown process".to_owned())?;
        process.stdin.take();
        Ok(())
    })
}

fn result(handle: u64, kind: &'static str, wait: bool) -> Result<Option<Value>, String> {
    Ok(
        finish(handle, wait)?.map(|(exit, stdout, stderr)| match kind {
            "stdout" => Value::Bytes(stdout),
            "stderr" => Value::Bytes(stderr),
            _ => Value::Number(exit as i64),
        }),
    )
}

pub(crate) fn promise(value: &Value, kind: &'static str) -> Result<Promise, String> {
    let handle = handle(value, &format!("os/process-{kind}"))?;
    let promise = Promise::new();
    let weak = promise.downgrade();
    promise.set_poller(Rc::new(move || {
        if let Some(promise) = weak.upgrade() {
            match result(handle, kind, false) {
                Ok(Some(value)) => {
                    promise.resolve(value);
                }
                Ok(None) => {}
                Err(error) => {
                    promise.reject(error);
                }
            }
        }
    }));
    let weak = promise.downgrade();
    promise.set_waiter(Rc::new(move || {
        if let Some(promise) = weak.upgrade() {
            match result(handle, kind, true) {
                Ok(Some(value)) => {
                    promise.resolve(value);
                }
                Ok(None) => {}
                Err(error) => {
                    promise.reject(error);
                }
            }
        }
    }));
    Ok(promise)
}

pub(crate) fn take_stream(value: &Value, kind: &'static str) -> Result<u64, String> {
    let handle = handle(value, &format!("os/process-{kind}-stream"))?;
    PROCESSES.with(|state| {
        let mut state = state.borrow_mut();
        let process = state
            .records
            .get_mut(&handle)
            .ok_or_else(|| "os/process-stream: unknown process".to_owned())?;
        let taken = if kind == "stdout" {
            &mut process.stdout_stream_taken
        } else {
            &mut process.stderr_stream_taken
        };
        if *taken {
            return Err(format!("os/process-{kind}-stream already taken"));
        }
        *taken = true;
        Ok(handle)
    })
}

fn stream_result(handle: u64, kind: &'static str, wait: bool) -> Result<Option<Value>, String> {
    PROCESSES.with(|state| {
        let state = state.borrow();
        let process = state
            .records
            .get(&handle)
            .ok_or_else(|| "os/process-stream: unknown process".to_owned())?;
        let receiver = if kind == "stdout" {
            &process.stdout_chunks
        } else {
            &process.stderr_chunks
        };
        if wait {
            return Ok(receiver.recv().ok().map(Value::Bytes));
        }
        match receiver.try_recv() {
            Ok(bytes) => Ok(Some(Value::Bytes(bytes))),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Ok(Some(Value::Nil)),
        }
    })
}

pub(crate) fn stream_promise(handle: u64, kind: &'static str) -> Promise {
    let promise = Promise::new();
    let weak = promise.downgrade();
    promise.set_poller(Rc::new(move || {
        if let Some(promise) = weak.upgrade() {
            match stream_result(handle, kind, false) {
                Ok(Some(value)) => {
                    promise.resolve(value);
                }
                Ok(None) => {}
                Err(error) => {
                    promise.reject(error);
                }
            };
        }
    }));
    let weak = promise.downgrade();
    promise.set_waiter(Rc::new(move || {
        if let Some(promise) = weak.upgrade() {
            match stream_result(handle, kind, true) {
                Ok(Some(value)) => {
                    promise.resolve(value);
                }
                Ok(None) => {
                    promise.resolve(Value::Nil);
                }
                Err(error) => {
                    promise.reject(error);
                }
            };
        }
    }));
    promise
}

pub(crate) fn kill(value: &Value) -> Result<(), String> {
    let handle = handle(value, "os/process-kill")?;
    PROCESSES.with(|state| {
        let mut state = state.borrow_mut();
        let process = state
            .records
            .get_mut(&handle)
            .ok_or_else(|| "os/process-kill: unknown process".to_owned())?;
        if process.exit.is_none() {
            let _ = process.child.kill();
        }
        Ok(())
    })
}
