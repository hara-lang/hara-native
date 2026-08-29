#[path = "graph/finalize.rs"]
mod finalize;
#[path = "graph/index.rs"]
mod index;
#[path = "graph/model.rs"]
mod model;
#[path = "graph/reachability.rs"]
mod reachability;

pub(crate) use super::unit::NativeRootInventory;
pub use super::unit::{Effect, UnitAnalysis, UnitKind};
pub use finalize::finish_analysis;
pub use model::{Analysis, AnalysisOutput, BuildOutput, ModuleAnalysis, RetentionReason};
