#[path = "task/promise.rs"]
pub mod promise;

// Production reachability is project-owned source reached through the task
// facade until the native project model is split into its directory module.
#[cfg(all(
    feature = "bytecode-vm",
    not(any(target_arch = "wasm32", feature = "raw-wasm"))
))]
#[path = "project/production.rs"]
pub mod production;

pub use promise::{LocalPromiseProvider, Promise, PromiseProvider, PromiseRejection, PromiseState};
