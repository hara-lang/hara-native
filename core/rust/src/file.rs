//! Capability-backed logical filesystem providers.
//!
//! Public paths are always absolute paths in the mounted logical filesystem.
//! Host paths are confined to provider implementations and never escape in
//! return values or error data.

use crate::core::{ExceptionInfo, Value};
use crate::task::Promise;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
use std::fs::{self, File, OpenOptions};
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
use std::io::Write;
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
use std::path::{Path, PathBuf};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileError {
    NotFound,
    AlreadyExists,
    InvalidPath(String),
    OutsideRoot,
    Denied,
    NotDirectory,
    IsDirectory,
    DirectoryNotEmpty,
    PermissionDenied,
    Unsupported,
    Io(String),
}

impl FileError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "not-found",
            Self::AlreadyExists => "already-exists",
            Self::InvalidPath(_) => "invalid-path",
            Self::OutsideRoot => "outside-root",
            Self::Denied => "denied",
            Self::NotDirectory => "not-directory",
            Self::IsDirectory => "is-directory",
            Self::DirectoryNotEmpty => "directory-not-empty",
            Self::PermissionDenied => "permission-denied",
            Self::Unsupported => "unsupported",
            Self::Io(_) => "io",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::InvalidPath(message) | Self::Io(message) => message.clone(),
            _ => format!("file/{}", self.code()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
    Symlink,
    Other,
}

impl FileType {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Symlink => "symlink",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub kind: FileType,
    pub size: Option<u64>,
    pub modified_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    Create,
    Replace,
    Append,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteOptions {
    pub mode: WriteMode,
    pub parents: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            mode: WriteMode::Create,
            parents: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MkdirOptions {
    pub parents: bool,
    pub exists_ok: bool,
}

impl Default for MkdirOptions {
    fn default() -> Self {
        Self {
            parents: true,
            exists_ok: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeleteOptions {
    pub missing_ok: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CopyOptions {
    pub replace: bool,
    pub parents: bool,
    pub preserve_modified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MoveOptions {
    pub replace: bool,
    pub parents: bool,
    pub atomic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TempFileOptions {
    pub prefix: String,
    pub suffix: String,
}

impl Default for TempFileOptions {
    fn default() -> Self {
        Self {
            prefix: "tmp".into(),
            suffix: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TempDirectoryOptions {
    pub prefix: String,
}

impl Default for TempDirectoryOptions {
    fn default() -> Self {
        Self {
            prefix: "tmp".into(),
        }
    }
}

pub fn logical_normalise(path: &str) -> Result<String, FileError> {
    if path.contains('\0') {
        return Err(FileError::InvalidPath("logical path contains NUL".into()));
    }
    if path.contains('\\') {
        return Err(FileError::InvalidPath(
            "logical paths use '/' rather than host separators".into(),
        ));
    }
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err(FileError::OutsideRoot);
                }
            }
            value
                if value.len() >= 2
                    && value.as_bytes()[0].is_ascii_alphabetic()
                    && value.as_bytes()[1] == b':' =>
            {
                return Err(FileError::InvalidPath(
                    "logical paths do not accept host drive prefixes".into(),
                ));
            }
            value => segments.push(value),
        }
    }
    if segments.is_empty() {
        Ok("/".into())
    } else {
        Ok(format!("/{}", segments.join("/")))
    }
}

pub fn logical_join(base: &str, path: &str) -> Result<String, FileError> {
    let base = logical_normalise(base)?;
    let path = path.trim_start_matches('/');
    logical_normalise(&format!("{}/{}", base.trim_end_matches('/'), path))
}

pub fn logical_resolve(base: &str, path: &str) -> Result<String, FileError> {
    if path.starts_with('/') {
        logical_normalise(path)
    } else {
        logical_join(base, path)
    }
}

pub fn logical_parent(path: &str) -> Result<Option<String>, FileError> {
    let path = logical_normalise(path)?;
    if path == "/" {
        return Ok(None);
    }
    let index = path.rfind('/').unwrap_or(0);
    Ok(Some(if index == 0 {
        "/".into()
    } else {
        path[..index].into()
    }))
}

pub fn logical_name(path: &str) -> Result<String, FileError> {
    let path = logical_normalise(path)?;
    Ok(if path == "/" {
        String::new()
    } else {
        path.rsplit('/').next().unwrap_or_default().into()
    })
}

pub fn file_error_value(
    operation: &str,
    path: &str,
    target: Option<&str>,
    error: &FileError,
) -> Value {
    let path = logical_normalise(path).unwrap_or_else(|_| path.to_owned());
    let target = target.map(|value| logical_normalise(value).unwrap_or_else(|_| value.to_owned()));
    Value::ExceptionInfo(Rc::new(ExceptionInfo {
        message: format!("{operation} failed: {}", error.message()),
        data: Box::new(Value::Map(
            [
                (
                    Value::Keyword("ex/code".into()),
                    Value::Keyword(format!("file/{}", error.code()).into()),
                ),
                (
                    Value::Keyword("ex/class".into()),
                    Value::Keyword(file_error_class(error).into()),
                ),
                (
                    Value::Keyword("file/operation".into()),
                    Value::Keyword(operation.trim_start_matches("file/").into()),
                ),
                (Value::Keyword("file/path".into()), Value::String(path)),
                (
                    Value::Keyword("file/target".into()),
                    target.map(Value::String).unwrap_or(Value::Nil),
                ),
            ]
            .into_iter()
            .collect(),
        )),
        cause: None,
        provenance: Rc::new(RefCell::new(Default::default())),
    }))
}

fn file_error_class(error: &FileError) -> &'static str {
    match error {
        FileError::NotFound => "ex.class/not-found",
        FileError::AlreadyExists | FileError::DirectoryNotEmpty => "ex.class/conflict",
        FileError::InvalidPath(_) => "ex.class/argument",
        FileError::OutsideRoot | FileError::Denied | FileError::PermissionDenied => {
            "ex.class/security"
        }
        FileError::NotDirectory
        | FileError::IsDirectory
        | FileError::Unsupported
        | FileError::Io(_) => "ex.class/io",
    }
}

fn resolved(value: Value) -> Promise {
    let promise = Promise::new();
    promise.resolve(value);
    promise
}

fn rejected(operation: &str, path: &str, target: Option<&str>, error: FileError) -> Promise {
    let promise = Promise::new();
    promise.reject_value(file_error_value(operation, path, target, &error));
    promise
}

fn entries_value(entries: Vec<FileEntry>) -> Value {
    Value::Vector(
        entries
            .into_iter()
            .map(|entry| {
                Value::Map(
                    [
                        (Value::Keyword("path".into()), Value::String(entry.path)),
                        (Value::Keyword("name".into()), Value::String(entry.name)),
                        (
                            Value::Keyword("type".into()),
                            Value::Keyword(entry.kind.keyword().into()),
                        ),
                        (
                            Value::Keyword("size".into()),
                            entry
                                .size
                                .and_then(|size| i64::try_from(size).ok())
                                .map(Value::Number)
                                .unwrap_or(Value::Nil),
                        ),
                        (
                            Value::Keyword("modified-at".into()),
                            Value::Number(entry.modified_at),
                        ),
                        (
                            Value::Keyword("extensions".into()),
                            Value::Map(Default::default()),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                )
            })
            .collect(),
    )
}

fn list_value(entries: Vec<FileEntry>) -> Value {
    Value::Vector(
        entries
            .into_iter()
            .map(|entry| Value::String(entry.path))
            .collect(),
    )
}

fn walk_collect<P: FileProvider + ?Sized>(
    provider: &P,
    path: &str,
    output: &mut Vec<String>,
) -> Result<(), FileError> {
    let stat = provider.stat_entry(path)?;
    match stat.kind {
        FileType::Directory => {
            for entry in provider.entries_values(path)? {
                if entry.kind == FileType::Directory {
                    walk_collect(provider, &entry.path, output)?;
                } else {
                    output.push(entry.path);
                }
            }
        }
        FileType::File | FileType::Symlink | FileType::Other => output.push(stat.path),
    }
    Ok(())
}

pub trait FileProvider {
    fn resolve(&self, root: &str, path: &str) -> Result<String, FileError> {
        logical_resolve(root, path)
    }

    /// Resolves a mounted logical path to a host path for native operations
    /// that must hand the resource to a host-only subsystem. Providers that
    /// do not own a host filesystem deliberately leave this unsupported.
    fn host_path(&self, _path: &str) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }

    fn read_bytes(&self, path: &str) -> Result<Vec<u8>, FileError>;
    fn write_bytes(
        &self,
        path: &str,
        bytes: Vec<u8>,
        options: WriteOptions,
    ) -> Result<String, FileError>;
    fn exists_value(&self, path: &str) -> Result<bool, FileError>;
    fn stat_entry(&self, path: &str) -> Result<FileEntry, FileError>;
    fn entries_values(&self, path: &str) -> Result<Vec<FileEntry>, FileError>;
    fn mkdir_path(&self, path: &str, options: MkdirOptions) -> Result<String, FileError>;
    fn delete_path(&self, path: &str, options: DeleteOptions) -> Result<String, FileError>;
    fn copy_path(
        &self,
        source: &str,
        target: &str,
        options: CopyOptions,
    ) -> Result<String, FileError>;
    fn move_path(
        &self,
        source: &str,
        target: &str,
        options: MoveOptions,
    ) -> Result<String, FileError>;
    fn temp_file_path(&self, parent: &str, options: TempFileOptions) -> Result<String, FileError>;
    fn temp_directory_path(
        &self,
        parent: &str,
        options: TempDirectoryOptions,
    ) -> Result<String, FileError>;

    fn read(&self, path: &str) -> Result<Promise, FileError> {
        let logical = logical_normalise(path)?;
        Ok(match self.read_bytes(&logical) {
            Ok(bytes) => resolved(Value::Bytes(bytes)),
            Err(error) => rejected("file/read", &logical, None, error),
        })
    }

    fn write(&self, path: &str, bytes: Vec<u8>) -> Result<Promise, FileError> {
        self.write_with_options(path, bytes, WriteOptions::default())
    }

    fn write_with_options(
        &self,
        path: &str,
        bytes: Vec<u8>,
        options: WriteOptions,
    ) -> Result<Promise, FileError> {
        let logical = logical_normalise(path)?;
        Ok(match self.write_bytes(&logical, bytes, options) {
            Ok(path) => resolved(Value::String(path)),
            Err(error) => rejected("file/write", &logical, None, error),
        })
    }

    fn exists(&self, path: &str) -> Result<Promise, FileError> {
        let logical = logical_normalise(path)?;
        Ok(match self.exists_value(&logical) {
            Ok(value) => resolved(Value::Bool(value)),
            Err(FileError::NotFound) => resolved(Value::Bool(false)),
            Err(error) => rejected("file/exists?", &logical, None, error),
        })
    }

    fn stat(&self, path: &str) -> Result<Promise, FileError> {
        let logical = logical_normalise(path)?;
        Ok(match self.stat_entry(&logical) {
            Ok(entry) => resolved(entries_value(vec![entry]).into_single_entry()),
            Err(error) => rejected("file/stat", &logical, None, error),
        })
    }

    fn entries(&self, path: &str) -> Result<Promise, FileError> {
        let logical = logical_normalise(path)?;
        Ok(match self.entries_values(&logical) {
            Ok(entries) => resolved(entries_value(entries)),
            Err(error) => rejected("file/entries", &logical, None, error),
        })
    }

    fn list(&self, path: &str) -> Result<Promise, FileError> {
        let logical = logical_normalise(path)?;
        Ok(match self.entries_values(&logical) {
            Ok(entries) => resolved(list_value(entries)),
            Err(error) => rejected("file/list", &logical, None, error),
        })
    }

    fn walk(&self, path: &str) -> Result<Promise, FileError> {
        let logical = logical_normalise(path)?;
        let mut values = Vec::new();
        Ok(match walk_collect(self, &logical, &mut values) {
            Ok(()) => {
                values.sort();
                resolved(Value::Vector(
                    values.into_iter().map(Value::String).collect(),
                ))
            }
            Err(error) => rejected("file/walk", &logical, None, error),
        })
    }

    fn mkdir(&self, path: &str) -> Result<Promise, FileError> {
        self.mkdir_with_options(path, MkdirOptions::default())
    }

    fn mkdir_with_options(&self, path: &str, options: MkdirOptions) -> Result<Promise, FileError> {
        let logical = logical_normalise(path)?;
        Ok(match self.mkdir_path(&logical, options) {
            Ok(path) => resolved(Value::String(path)),
            Err(error) => rejected("file/mkdir", &logical, None, error),
        })
    }

    fn delete(&self, path: &str) -> Result<Promise, FileError> {
        self.delete_with_options(path, DeleteOptions::default())
    }

    fn delete_with_options(
        &self,
        path: &str,
        options: DeleteOptions,
    ) -> Result<Promise, FileError> {
        let logical = logical_normalise(path)?;
        Ok(match self.delete_path(&logical, options) {
            Ok(path) => resolved(Value::String(path)),
            Err(error) => rejected("file/delete", &logical, None, error),
        })
    }

    fn copy(&self, source: &str, target: &str, options: CopyOptions) -> Result<Promise, FileError> {
        let source = logical_normalise(source)?;
        let target = logical_normalise(target)?;
        Ok(match self.copy_path(&source, &target, options) {
            Ok(path) => resolved(Value::String(path)),
            Err(error) => rejected("file/copy", &source, Some(&target), error),
        })
    }

    fn move_entry(
        &self,
        source: &str,
        target: &str,
        options: MoveOptions,
    ) -> Result<Promise, FileError> {
        let source = logical_normalise(source)?;
        let target = logical_normalise(target)?;
        Ok(match self.move_path(&source, &target, options) {
            Ok(path) => resolved(Value::String(path)),
            Err(error) => rejected("file/move", &source, Some(&target), error),
        })
    }

    fn temp_file(&self, parent: &str, options: TempFileOptions) -> Result<Promise, FileError> {
        let parent = logical_normalise(parent)?;
        Ok(match self.temp_file_path(&parent, options) {
            Ok(path) => resolved(Value::String(path)),
            Err(error) => rejected("file/temp-file", &parent, None, error),
        })
    }

    fn temp_directory(
        &self,
        parent: &str,
        options: TempDirectoryOptions,
    ) -> Result<Promise, FileError> {
        let parent = logical_normalise(parent)?;
        Ok(match self.temp_directory_path(&parent, options) {
            Ok(path) => resolved(Value::String(path)),
            Err(error) => rejected("file/temp-directory", &parent, None, error),
        })
    }
}

trait SingleEntryValue {
    fn into_single_entry(self) -> Value;
}

impl SingleEntryValue for Value {
    fn into_single_entry(self) -> Value {
        match self {
            Value::Vector(values) => values.iter().next().cloned().unwrap_or(Value::Nil),
            value => value,
        }
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
fn io_error(error: std::io::Error) -> FileError {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::NotFound => FileError::NotFound,
        ErrorKind::AlreadyExists => FileError::AlreadyExists,
        ErrorKind::PermissionDenied => FileError::PermissionDenied,
        ErrorKind::NotADirectory => FileError::NotDirectory,
        ErrorKind::IsADirectory => FileError::IsDirectory,
        ErrorKind::DirectoryNotEmpty => FileError::DirectoryNotEmpty,
        ErrorKind::Unsupported => FileError::Unsupported,
        _ => FileError::Io(error.to_string()),
    }
}

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
fn modified_millis(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

fn validate_temp_name(prefix: &str, suffix: Option<&str>) -> Result<(), FileError> {
    for value in std::iter::once(prefix).chain(suffix) {
        if value.contains('/') || value.contains('\\') || value.contains('\0') {
            return Err(FileError::InvalidPath(
                "temporary entry prefix and suffix must be single logical path fragments".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
#[derive(Debug, Clone)]
pub struct NativeFileProvider {
    root: PathBuf,
}

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
impl NativeFileProvider {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        let root = if root.is_absolute() {
            root.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(root)
        };
        Self { root }
    }

    fn scoped(&self, logical: &str) -> Result<PathBuf, FileError> {
        let logical = logical_normalise(logical)?;
        let logical_path = Path::new(&logical);
        let relative = logical_path
            .strip_prefix(&self.root)
            .unwrap_or_else(|_| Path::new(logical.trim_start_matches('/')));
        let candidate = self.root.join(relative);
        let mut current = self.root.clone();
        let relative = candidate
            .strip_prefix(&self.root)
            .map_err(|_| FileError::OutsideRoot)?;
        let components: Vec<_> = relative.components().collect();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(FileError::OutsideRoot)
                }
                Ok(metadata) if !metadata.is_dir() => return Err(FileError::NotDirectory),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(io_error(error)),
            }
        }
        Ok(candidate)
    }

    fn entry(&self, logical: &str, host: &Path) -> Result<FileEntry, FileError> {
        let metadata = fs::symlink_metadata(host).map_err(io_error)?;
        let kind = if metadata.file_type().is_symlink() {
            FileType::Symlink
        } else if metadata.is_file() {
            FileType::File
        } else if metadata.is_dir() {
            FileType::Directory
        } else {
            FileType::Other
        };
        Ok(FileEntry {
            path: logical_normalise(logical)?,
            name: logical_name(logical)?,
            kind,
            size: (kind == FileType::File).then_some(metadata.len()),
            modified_at: modified_millis(&metadata),
        })
    }

    fn ensure_parent(&self, path: &Path, parents: bool) -> Result<(), FileError> {
        let parent = path
            .parent()
            .ok_or_else(|| FileError::InvalidPath("path has no parent".into()))?;
        if parents {
            fs::create_dir_all(parent).map_err(io_error)
        } else {
            let metadata = fs::symlink_metadata(parent).map_err(io_error)?;
            if metadata.is_dir() {
                Ok(())
            } else {
                Err(FileError::NotDirectory)
            }
        }
    }
}

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
impl FileProvider for NativeFileProvider {
    fn host_path(&self, path: &str) -> Result<String, FileError> {
        self.scoped(path)
            .map(|path| path.to_string_lossy().into_owned())
    }

    fn read_bytes(&self, path: &str) -> Result<Vec<u8>, FileError> {
        let host = self.scoped(path)?;
        let metadata = fs::symlink_metadata(&host).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            return Err(FileError::Unsupported);
        }
        if metadata.is_dir() {
            return Err(FileError::IsDirectory);
        }
        if !metadata.is_file() {
            return Err(FileError::Unsupported);
        }
        fs::read(host).map_err(io_error)
    }

    fn write_bytes(
        &self,
        path: &str,
        bytes: Vec<u8>,
        options: WriteOptions,
    ) -> Result<String, FileError> {
        let logical = logical_normalise(path)?;
        let host = self.scoped(&logical)?;
        self.ensure_parent(&host, options.parents)?;
        if let Ok(metadata) = fs::symlink_metadata(&host) {
            if metadata.file_type().is_symlink() {
                return Err(FileError::Unsupported);
            }
            if metadata.is_dir() {
                return Err(FileError::IsDirectory);
            }
        }
        let mut builder = OpenOptions::new();
        builder.write(true);
        match options.mode {
            WriteMode::Create => {
                builder.create_new(true);
            }
            WriteMode::Replace => {
                builder.create(true).truncate(true);
            }
            WriteMode::Append => {
                builder.create(true).append(true);
            }
        }
        let mut file = builder.open(host).map_err(io_error)?;
        file.write_all(&bytes).map_err(io_error)?;
        Ok(logical)
    }

    fn exists_value(&self, path: &str) -> Result<bool, FileError> {
        let host = self.scoped(path)?;
        match fs::symlink_metadata(host) {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(io_error(error)),
        }
    }

    fn stat_entry(&self, path: &str) -> Result<FileEntry, FileError> {
        let logical = logical_normalise(path)?;
        let host = self.scoped(&logical)?;
        self.entry(&logical, &host)
    }

    fn entries_values(&self, path: &str) -> Result<Vec<FileEntry>, FileError> {
        let logical = logical_normalise(path)?;
        let host = self.scoped(&logical)?;
        let metadata = fs::symlink_metadata(&host).map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(FileError::NotDirectory);
        }
        let mut entries = Vec::new();
        for value in fs::read_dir(host).map_err(io_error)? {
            let value = value.map_err(io_error)?;
            let name = value.file_name().into_string().map_err(|_| {
                FileError::InvalidPath("filesystem entry is not valid UTF-8".into())
            })?;
            let child = logical_resolve(&logical, &name)?;
            entries.push(self.entry(&child, &value.path())?);
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(entries)
    }

    fn mkdir_path(&self, path: &str, options: MkdirOptions) -> Result<String, FileError> {
        let logical = logical_normalise(path)?;
        let host = self.scoped(&logical)?;
        if let Ok(metadata) = fs::symlink_metadata(&host) {
            if metadata.is_dir() && options.exists_ok {
                return Ok(logical);
            }
            return Err(FileError::AlreadyExists);
        }
        if options.parents {
            fs::create_dir_all(host).map_err(io_error)?;
        } else {
            self.ensure_parent(&host, false)?;
            fs::create_dir(host).map_err(io_error)?;
        }
        Ok(logical)
    }

    fn delete_path(&self, path: &str, options: DeleteOptions) -> Result<String, FileError> {
        let logical = logical_normalise(path)?;
        if logical == "/" {
            return Err(FileError::Denied);
        }
        let host = self.scoped(&logical)?;
        let metadata = match fs::symlink_metadata(&host) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && options.missing_ok => {
                return Ok(logical)
            }
            Err(error) => return Err(io_error(error)),
        };
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir(host).map_err(io_error)?;
        } else {
            fs::remove_file(host).map_err(io_error)?;
        }
        Ok(logical)
    }

    fn copy_path(
        &self,
        source: &str,
        target: &str,
        options: CopyOptions,
    ) -> Result<String, FileError> {
        let source = logical_normalise(source)?;
        let target = logical_normalise(target)?;
        if source == target {
            return Err(FileError::AlreadyExists);
        }
        let source_host = self.scoped(&source)?;
        let target_host = self.scoped(&target)?;
        let metadata = fs::symlink_metadata(&source_host).map_err(io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(if metadata.is_dir() {
                FileError::IsDirectory
            } else {
                FileError::Unsupported
            });
        }
        self.ensure_parent(&target_host, options.parents)?;
        if let Ok(target_metadata) = fs::symlink_metadata(&target_host) {
            if !options.replace {
                return Err(FileError::AlreadyExists);
            }
            if target_metadata.is_dir() && !target_metadata.file_type().is_symlink() {
                return Err(FileError::IsDirectory);
            }
            // Replace the directory entry itself. Opening a target symlink with
            // truncate would otherwise mutate a file outside the mounted root.
            fs::remove_file(&target_host).map_err(io_error)?;
        }
        let mut copy_options = fs::OpenOptions::new();
        copy_options.write(true).create_new(true);
        let mut input = File::open(&source_host).map_err(io_error)?;
        let mut output = copy_options.open(&target_host).map_err(io_error)?;
        std::io::copy(&mut input, &mut output).map_err(io_error)?;
        if options.preserve_modified {
            let modified = metadata.modified().map_err(io_error)?;
            output
                .set_times(fs::FileTimes::new().set_modified(modified))
                .map_err(io_error)?;
        }
        Ok(target)
    }

    fn move_path(
        &self,
        source: &str,
        target: &str,
        options: MoveOptions,
    ) -> Result<String, FileError> {
        let source = logical_normalise(source)?;
        let target = logical_normalise(target)?;
        if source == "/" || target == "/" {
            return Err(FileError::Denied);
        }
        if source == target {
            self.stat_entry(&source)?;
            return Ok(target);
        }
        if target.starts_with(&format!("{source}/")) {
            return Err(FileError::InvalidPath(
                "cannot move a directory beneath itself".into(),
            ));
        }
        let source_host = self.scoped(&source)?;
        let target_host = self.scoped(&target)?;
        let source_metadata = fs::symlink_metadata(&source_host).map_err(io_error)?;
        if source_metadata.file_type().is_symlink() {
            return Err(FileError::Unsupported);
        }
        self.ensure_parent(&target_host, options.parents)?;
        if let Ok(target_metadata) = fs::symlink_metadata(&target_host) {
            if !options.replace {
                return Err(FileError::AlreadyExists);
            }
            if target_metadata.is_dir() && !target_metadata.file_type().is_symlink() {
                fs::remove_dir(&target_host).map_err(io_error)?;
            } else {
                fs::remove_file(&target_host).map_err(io_error)?;
            }
        }
        fs::rename(source_host, target_host).map_err(|error| {
            if options.atomic {
                FileError::Unsupported
            } else {
                io_error(error)
            }
        })?;
        Ok(target)
    }

    fn temp_file_path(&self, parent: &str, options: TempFileOptions) -> Result<String, FileError> {
        validate_temp_name(&options.prefix, Some(&options.suffix))?;
        let parent = logical_normalise(parent)?;
        let parent_host = self.scoped(&parent)?;
        let metadata = fs::symlink_metadata(&parent_host).map_err(io_error)?;
        if !metadata.is_dir() {
            return Err(FileError::NotDirectory);
        }
        for _ in 0..1024 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = format!("{}-{:016x}{}", options.prefix, sequence, options.suffix);
            let logical = logical_resolve(&parent, &name)?;
            let host = self.scoped(&logical)?;
            match OpenOptions::new().write(true).create_new(true).open(host) {
                Ok(_) => return Ok(logical),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(io_error(error)),
            }
        }
        Err(FileError::Io(
            "unable to allocate unique temporary file".into(),
        ))
    }

    fn temp_directory_path(
        &self,
        parent: &str,
        options: TempDirectoryOptions,
    ) -> Result<String, FileError> {
        validate_temp_name(&options.prefix, None)?;
        let parent = logical_normalise(parent)?;
        let parent_host = self.scoped(&parent)?;
        let metadata = fs::symlink_metadata(&parent_host).map_err(io_error)?;
        if !metadata.is_dir() {
            return Err(FileError::NotDirectory);
        }
        for _ in 0..1024 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = format!("{}-{:016x}", options.prefix, sequence);
            let logical = logical_resolve(&parent, &name)?;
            let host = self.scoped(&logical)?;
            match fs::create_dir(host) {
                Ok(()) => return Ok(logical),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(io_error(error)),
            }
        }
        Err(FileError::Io(
            "unable to allocate unique temporary directory".into(),
        ))
    }
}

#[derive(Debug, Clone)]
enum MemoryNode {
    Directory { modified_at: i64 },
    File { bytes: Vec<u8>, modified_at: i64 },
}

#[derive(Debug, Clone)]
pub struct MemoryFileProvider {
    nodes: Rc<RefCell<HashMap<String, MemoryNode>>>,
}

impl MemoryFileProvider {
    pub fn new(_root: impl Into<String>) -> Self {
        Self {
            nodes: Rc::new(RefCell::new(HashMap::from([(
                "/".into(),
                MemoryNode::Directory {
                    modified_at: now_millis(),
                },
            )]))),
        }
    }

    pub fn insert(&self, path: &str, bytes: Vec<u8>) -> Result<(), FileError> {
        self.write_bytes(
            path,
            bytes,
            WriteOptions {
                mode: WriteMode::Replace,
                parents: true,
            },
        )?;
        Ok(())
    }

    fn ensure_parent(&self, path: &str, parents: bool) -> Result<(), FileError> {
        let parent = logical_parent(path)?.ok_or(FileError::Denied)?;
        if parents {
            let mut current = String::from("/");
            for segment in parent
                .trim_start_matches('/')
                .split('/')
                .filter(|value| !value.is_empty())
            {
                current = logical_resolve(&current, segment)?;
                let mut nodes = self.nodes.borrow_mut();
                match nodes.get(&current) {
                    Some(MemoryNode::Directory { .. }) => {}
                    Some(_) => return Err(FileError::NotDirectory),
                    None => {
                        nodes.insert(
                            current.clone(),
                            MemoryNode::Directory {
                                modified_at: now_millis(),
                            },
                        );
                    }
                }
            }
            Ok(())
        } else {
            match self.nodes.borrow().get(&parent) {
                Some(MemoryNode::Directory { .. }) => Ok(()),
                Some(_) => Err(FileError::NotDirectory),
                None => Err(FileError::NotFound),
            }
        }
    }

    fn entry_for(&self, path: &str, node: &MemoryNode) -> Result<FileEntry, FileError> {
        let (kind, size, modified_at) = match node {
            MemoryNode::Directory { modified_at } => (FileType::Directory, None, *modified_at),
            MemoryNode::File { bytes, modified_at } => {
                (FileType::File, Some(bytes.len() as u64), *modified_at)
            }
        };
        Ok(FileEntry {
            path: logical_normalise(path)?,
            name: logical_name(path)?,
            kind,
            size,
            modified_at,
        })
    }
}

impl FileProvider for MemoryFileProvider {
    fn read_bytes(&self, path: &str) -> Result<Vec<u8>, FileError> {
        let path = logical_normalise(path)?;
        match self.nodes.borrow().get(&path) {
            Some(MemoryNode::File { bytes, .. }) => Ok(bytes.clone()),
            Some(MemoryNode::Directory { .. }) => Err(FileError::IsDirectory),
            None => Err(FileError::NotFound),
        }
    }

    fn write_bytes(
        &self,
        path: &str,
        mut bytes: Vec<u8>,
        options: WriteOptions,
    ) -> Result<String, FileError> {
        let path = logical_normalise(path)?;
        if path == "/" {
            return Err(FileError::IsDirectory);
        }
        self.ensure_parent(&path, options.parents)?;
        let mut nodes = self.nodes.borrow_mut();
        match (nodes.get(&path), options.mode) {
            (Some(MemoryNode::Directory { .. }), _) => return Err(FileError::IsDirectory),
            (Some(_), WriteMode::Create) => return Err(FileError::AlreadyExists),
            (
                Some(MemoryNode::File {
                    bytes: existing, ..
                }),
                WriteMode::Append,
            ) => {
                let mut output = existing.clone();
                output.extend(bytes);
                bytes = output;
            }
            _ => {}
        }
        nodes.insert(
            path.clone(),
            MemoryNode::File {
                bytes,
                modified_at: now_millis(),
            },
        );
        Ok(path)
    }

    fn exists_value(&self, path: &str) -> Result<bool, FileError> {
        Ok(self.nodes.borrow().contains_key(&logical_normalise(path)?))
    }

    fn stat_entry(&self, path: &str) -> Result<FileEntry, FileError> {
        let path = logical_normalise(path)?;
        let nodes = self.nodes.borrow();
        let node = nodes.get(&path).ok_or(FileError::NotFound)?;
        self.entry_for(&path, node)
    }

    fn entries_values(&self, path: &str) -> Result<Vec<FileEntry>, FileError> {
        let path = logical_normalise(path)?;
        match self.nodes.borrow().get(&path) {
            Some(MemoryNode::Directory { .. }) => {}
            Some(_) => return Err(FileError::NotDirectory),
            None => return Err(FileError::NotFound),
        }
        let mut output = BTreeMap::new();
        let prefix = if path == "/" {
            "/".into()
        } else {
            format!("{path}/")
        };
        let nodes = self.nodes.borrow();
        for (candidate, node) in nodes.iter() {
            if candidate == &path || !candidate.starts_with(&prefix) {
                continue;
            }
            let remainder = &candidate[prefix.len()..];
            if remainder.is_empty() || remainder.contains('/') {
                continue;
            }
            output.insert(candidate.clone(), self.entry_for(candidate, node)?);
        }
        Ok(output.into_values().collect())
    }

    fn mkdir_path(&self, path: &str, options: MkdirOptions) -> Result<String, FileError> {
        let path = logical_normalise(path)?;
        if let Some(node) = self.nodes.borrow().get(&path) {
            return if matches!(node, MemoryNode::Directory { .. }) && options.exists_ok {
                Ok(path)
            } else {
                Err(FileError::AlreadyExists)
            };
        }
        self.ensure_parent(&path, options.parents)?;
        self.nodes.borrow_mut().insert(
            path.clone(),
            MemoryNode::Directory {
                modified_at: now_millis(),
            },
        );
        Ok(path)
    }

    fn delete_path(&self, path: &str, options: DeleteOptions) -> Result<String, FileError> {
        let path = logical_normalise(path)?;
        if path == "/" {
            return Err(FileError::Denied);
        }
        let mut nodes = self.nodes.borrow_mut();
        let Some(node) = nodes.get(&path) else {
            return if options.missing_ok {
                Ok(path)
            } else {
                Err(FileError::NotFound)
            };
        };
        if matches!(node, MemoryNode::Directory { .. }) {
            let prefix = format!("{path}/");
            if nodes.keys().any(|candidate| candidate.starts_with(&prefix)) {
                return Err(FileError::DirectoryNotEmpty);
            }
        }
        nodes.remove(&path);
        Ok(path)
    }

    fn copy_path(
        &self,
        source: &str,
        target: &str,
        options: CopyOptions,
    ) -> Result<String, FileError> {
        let source = logical_normalise(source)?;
        let target = logical_normalise(target)?;
        if source == target {
            return Err(FileError::AlreadyExists);
        }
        let (bytes, modified_at) = match self.nodes.borrow().get(&source) {
            Some(MemoryNode::File { bytes, modified_at }) => (bytes.clone(), *modified_at),
            Some(MemoryNode::Directory { .. }) => return Err(FileError::IsDirectory),
            None => return Err(FileError::NotFound),
        };
        let result = self.write_bytes(
            &target,
            bytes,
            WriteOptions {
                mode: if options.replace {
                    WriteMode::Replace
                } else {
                    WriteMode::Create
                },
                parents: options.parents,
            },
        )?;
        if options.preserve_modified {
            if let Some(MemoryNode::File {
                modified_at: target_modified,
                ..
            }) = self.nodes.borrow_mut().get_mut(&target)
            {
                *target_modified = modified_at;
            }
        }
        Ok(result)
    }

    fn move_path(
        &self,
        source: &str,
        target: &str,
        options: MoveOptions,
    ) -> Result<String, FileError> {
        if options.atomic { /* memory moves are atomic within one provider */ }
        let source = logical_normalise(source)?;
        let target = logical_normalise(target)?;
        if source == "/" || target == "/" {
            return Err(FileError::Denied);
        }
        if source == target {
            self.stat_entry(&source)?;
            return Ok(target);
        }
        if target.starts_with(&format!("{source}/")) {
            return Err(FileError::InvalidPath(
                "cannot move a directory beneath itself".into(),
            ));
        }
        self.ensure_parent(&target, options.parents)?;
        let mut nodes = self.nodes.borrow_mut();
        if !nodes.contains_key(&source) {
            return Err(FileError::NotFound);
        }
        if nodes.contains_key(&target) && !options.replace {
            return Err(FileError::AlreadyExists);
        }
        if options.replace {
            let target_prefix = format!("{target}/");
            if matches!(nodes.get(&target), Some(MemoryNode::Directory { .. }))
                && nodes
                    .keys()
                    .any(|candidate| candidate.starts_with(&target_prefix))
            {
                return Err(FileError::DirectoryNotEmpty);
            }
            nodes.remove(&target);
        }
        let prefix = format!("{source}/");
        let moving: Vec<(String, MemoryNode)> = nodes
            .iter()
            .filter(|(path, _)| *path == &source || path.starts_with(&prefix))
            .map(|(path, node)| (path.clone(), node.clone()))
            .collect();
        for (path, _) in &moving {
            nodes.remove(path);
        }
        for (path, node) in moving {
            let suffix = path.strip_prefix(&source).unwrap_or_default();
            nodes.insert(format!("{target}{suffix}"), node);
        }
        Ok(target)
    }

    fn temp_file_path(&self, parent: &str, options: TempFileOptions) -> Result<String, FileError> {
        validate_temp_name(&options.prefix, Some(&options.suffix))?;
        for _ in 0..1024 {
            let name = format!(
                "{}-{:016x}{}",
                options.prefix,
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed),
                options.suffix
            );
            let path = logical_resolve(parent, &name)?;
            match self.write_bytes(&path, Vec::new(), WriteOptions::default()) {
                Ok(value) => return Ok(value),
                Err(FileError::AlreadyExists) => {}
                Err(error) => return Err(error),
            }
        }
        Err(FileError::Io(
            "unable to allocate unique temporary file".into(),
        ))
    }

    fn temp_directory_path(
        &self,
        parent: &str,
        options: TempDirectoryOptions,
    ) -> Result<String, FileError> {
        validate_temp_name(&options.prefix, None)?;
        for _ in 0..1024 {
            let name = format!(
                "{}-{:016x}",
                options.prefix,
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            );
            let path = logical_resolve(parent, &name)?;
            match self.mkdir_path(
                &path,
                MkdirOptions {
                    parents: false,
                    exists_ok: false,
                },
            ) {
                Ok(value) => return Ok(value),
                Err(FileError::AlreadyExists) => {}
                Err(error) => return Err(error),
            }
        }
        Err(FileError::Io(
            "unable to allocate unique temporary directory".into(),
        ))
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedFileProvider;

impl FileProvider for UnsupportedFileProvider {
    fn read_bytes(&self, _path: &str) -> Result<Vec<u8>, FileError> {
        Err(FileError::Unsupported)
    }
    fn write_bytes(
        &self,
        _path: &str,
        _bytes: Vec<u8>,
        _options: WriteOptions,
    ) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }
    fn exists_value(&self, _path: &str) -> Result<bool, FileError> {
        Err(FileError::Unsupported)
    }
    fn stat_entry(&self, _path: &str) -> Result<FileEntry, FileError> {
        Err(FileError::Unsupported)
    }
    fn entries_values(&self, _path: &str) -> Result<Vec<FileEntry>, FileError> {
        Err(FileError::Unsupported)
    }
    fn mkdir_path(&self, _path: &str, _options: MkdirOptions) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }
    fn delete_path(&self, _path: &str, _options: DeleteOptions) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }
    fn copy_path(
        &self,
        _source: &str,
        _target: &str,
        _options: CopyOptions,
    ) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }
    fn move_path(
        &self,
        _source: &str,
        _target: &str,
        _options: MoveOptions,
    ) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }
    fn temp_file_path(
        &self,
        _parent: &str,
        _options: TempFileOptions,
    ) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }
    fn temp_directory_path(
        &self,
        _parent: &str,
        _options: TempDirectoryOptions,
    ) -> Result<String, FileError> {
        Err(FileError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{PromiseRejection, PromiseState};

    fn rejection_data(promise: Promise) -> Value {
        match promise.wait_state() {
            PromiseState::Rejected(PromiseRejection::Value(Value::ExceptionInfo(info))) => {
                (*info.data).clone()
            }
            state => panic!("expected structured filesystem rejection, got {state:?}"),
        }
    }

    #[test]
    fn logical_paths_are_absolute_and_cannot_escape() {
        assert_eq!(
            logical_normalise("src//./main.hal").unwrap(),
            "/src/main.hal"
        );
        assert_eq!(
            logical_join("/src", "/test/main.hal").unwrap(),
            "/src/test/main.hal"
        );
        assert_eq!(
            logical_resolve("/src", "/test/main.hal").unwrap(),
            "/test/main.hal"
        );
        assert_eq!(
            logical_resolve("/src/lib", "../main.hal").unwrap(),
            "/src/main.hal"
        );
        assert_eq!(
            logical_parent("/src/main.hal").unwrap(),
            Some("/src".into())
        );
        assert_eq!(
            logical_normalise("../escape").unwrap_err(),
            FileError::OutsideRoot
        );
        assert!(matches!(
            logical_normalise(r"src\main.hal"),
            Err(FileError::InvalidPath(_))
        ));
        assert!(matches!(
            logical_normalise("C:/host/path"),
            Err(FileError::InvalidPath(_))
        ));
    }

    #[test]
    fn memory_provider_honours_safe_defaults_and_sorted_entries() {
        let files = MemoryFileProvider::new("ignored");
        files.mkdir_path("/src", MkdirOptions::default()).unwrap();
        files
            .write_bytes("/src/b", vec![2], WriteOptions::default())
            .unwrap();
        files
            .write_bytes("/src/a", vec![1], WriteOptions::default())
            .unwrap();
        assert_eq!(
            files
                .write_bytes("/src/a", vec![3], WriteOptions::default())
                .unwrap_err(),
            FileError::AlreadyExists
        );
        files
            .write_bytes(
                "/src/a",
                vec![3],
                WriteOptions {
                    mode: WriteMode::Append,
                    parents: false,
                },
            )
            .unwrap();
        assert_eq!(files.read_bytes("/src/a").unwrap(), vec![1, 3]);
        let entries = files.entries_values("/src").unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["/src/a", "/src/b"]
        );
        assert!(entries.iter().all(|entry| entry.name.len() == 1));
    }

    #[test]
    fn provider_failures_are_structured_promise_rejections() {
        let files = MemoryFileProvider::new("ignored");
        let data = rejection_data(files.read("/missing").unwrap());
        let Value::Map(data) = data else {
            panic!("filesystem rejection data was not a map");
        };
        assert_eq!(
            data.get(&Value::Keyword("ex/code".into())),
            Some(&Value::Keyword("file/not-found".into()))
        );
        assert_eq!(
            data.get(&Value::Keyword("file/operation".into())),
            Some(&Value::Keyword("read".into()))
        );
        assert_eq!(
            data.get(&Value::Keyword("file/path".into())),
            Some(&Value::String("/missing".into()))
        );
        assert_eq!(
            data.get(&Value::Keyword("file/target".into())),
            Some(&Value::Nil)
        );
    }

    #[test]
    fn memory_move_rejects_descendants_and_non_empty_replacement() {
        let files = MemoryFileProvider::new("ignored");
        files
            .write_bytes(
                "/source/child",
                vec![1],
                WriteOptions {
                    mode: WriteMode::Create,
                    parents: true,
                },
            )
            .unwrap();
        assert!(matches!(
            files.move_path("/source", "/source/child/nested", MoveOptions::default()),
            Err(FileError::InvalidPath(_))
        ));

        files
            .write_bytes(
                "/target/existing",
                vec![2],
                WriteOptions {
                    mode: WriteMode::Create,
                    parents: true,
                },
            )
            .unwrap();
        assert_eq!(
            files
                .move_path(
                    "/source",
                    "/target",
                    MoveOptions {
                        replace: true,
                        ..MoveOptions::default()
                    },
                )
                .unwrap_err(),
            FileError::DirectoryNotEmpty
        );
        assert_eq!(
            files
                .move_path("/source", "/source", MoveOptions::default())
                .unwrap(),
            "/source"
        );
    }

    #[test]
    fn memory_copy_preserves_modified_time_only_when_requested() {
        let files = MemoryFileProvider::new("ignored");
        files
            .write_bytes("/source", vec![1, 2], WriteOptions::default())
            .unwrap();
        if let Some(MemoryNode::File { modified_at, .. }) =
            files.nodes.borrow_mut().get_mut("/source")
        {
            *modified_at = 1234;
        }
        files
            .copy_path(
                "/source",
                "/target",
                CopyOptions {
                    preserve_modified: true,
                    ..CopyOptions::default()
                },
            )
            .unwrap();
        assert_eq!(files.stat_entry("/target").unwrap().modified_at, 1234);
        assert_eq!(
            files
                .copy_path(
                    "/source",
                    "/source",
                    CopyOptions {
                        replace: true,
                        ..CopyOptions::default()
                    },
                )
                .unwrap_err(),
            FileError::AlreadyExists
        );
    }

    #[test]
    fn temporary_names_must_remain_beneath_the_explicit_parent() {
        let files = MemoryFileProvider::new("ignored");
        files.mkdir_path("/tmp", MkdirOptions::default()).unwrap();
        assert!(matches!(
            files.temp_file_path(
                "/tmp",
                TempFileOptions {
                    prefix: "../escape".into(),
                    suffix: String::new(),
                },
            ),
            Err(FileError::InvalidPath(_))
        ));
        let first = files
            .temp_file_path("/tmp", TempFileOptions::default())
            .unwrap();
        let second = files
            .temp_file_path("/tmp", TempFileOptions::default())
            .unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with("/tmp/tmp-"));
        assert!(second.starts_with("/tmp/tmp-"));
    }

    #[cfg(all(unix, not(target_arch = "wasm32")))]
    #[test]
    fn native_provider_accepts_a_host_absolute_path_within_its_mount() {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "hara-native-file-mounted-path-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("project.edn"), b"{}").unwrap();
        let root = root.canonicalize().unwrap();
        let files = NativeFileProvider::new(&root);
        let mounted = root.join("project.edn").to_string_lossy().into_owned();
        let generated = root.join("generated.hal").to_string_lossy().into_owned();

        assert_eq!(files.read_bytes(&mounted).unwrap(), b"{}");
        assert_eq!(files.stat_entry(&mounted).unwrap().path, mounted);
        assert_eq!(
            files
                .write_bytes(
                    &generated,
                    b"(ns generated)\n".to_vec(),
                    WriteOptions::default()
                )
                .unwrap(),
            generated
        );
        assert_eq!(
            fs::read(root.join("generated.hal")).unwrap(),
            b"(ns generated)\n"
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(unix, not(target_arch = "wasm32")))]
    #[test]
    fn native_copy_replaces_a_symlink_entry_without_following_it() {
        use std::os::unix::fs::symlink;

        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "hara-native-file-test-{}-{sequence}",
            std::process::id()
        ));
        let outside = std::env::temp_dir().join(format!(
            "hara-native-file-outside-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("source"), b"inside").unwrap();
        fs::write(&outside, b"outside").unwrap();
        symlink(&outside, root.join("target")).unwrap();

        let files = NativeFileProvider::new(&root);
        files
            .copy_path(
                "/source",
                "/target",
                CopyOptions {
                    replace: true,
                    ..CopyOptions::default()
                },
            )
            .unwrap();

        assert_eq!(fs::read(root.join("target")).unwrap(), b"inside");
        assert_eq!(fs::read(&outside).unwrap(), b"outside");
        assert!(!fs::symlink_metadata(root.join("target"))
            .unwrap()
            .file_type()
            .is_symlink());

        fs::remove_dir_all(root).unwrap();
        fs::remove_file(outside).unwrap();
    }
}
