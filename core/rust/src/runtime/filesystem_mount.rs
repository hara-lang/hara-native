use super::filesystem_runtime::FilesystemRuntimeAdapter;
use super::{SessionKernel, SessionMountId};
use crate::filesystem::FilesystemHandle;
use std::rc::Rc;

impl SessionKernel {
    /// Creates a Kernel-owned mount from an already-opened provider-neutral
    /// filesystem capability. Existing attach/detach/close accounting remains
    /// authoritative; the adapter supplies only the runtime dispatch seam.
    pub fn create_provider_filesystem(&mut self, handle: FilesystemHandle) -> SessionMountId {
        let descriptor = handle.descriptor();
        let kind = stable_provider_kind(descriptor.kind());
        let display = descriptor.display().to_owned();
        self.create_filesystem(
            Rc::new(FilesystemRuntimeAdapter::new(handle)),
            kind,
            &display,
        )
    }
}

fn stable_provider_kind(kind: &str) -> &'static str {
    match kind {
        "native" => "native",
        "memory" => "memory",
        "indexeddb" => "indexeddb",
        "sftp" => "sftp",
        "github" => "github",
        "google-drive" => "google-drive",
        "s3" => "s3",
        "webdav" => "webdav",
        _ => "provider",
    }
}
