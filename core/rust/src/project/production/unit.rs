#[path = "unit/analysis.rs"]
mod analysis;
#[path = "unit/analyze.rs"]
mod analyze;
#[path = "unit/dynamic.rs"]
mod dynamic;
#[path = "unit/kind.rs"]
mod kind;
#[path = "unit/location.rs"]
mod location;
#[path = "unit/program.rs"]
mod program;
#[path = "unit/provides.rs"]
mod provides;

pub(crate) use analysis::NativeRootInventory;
pub use analysis::UnitAnalysis;
pub use analyze::{analyze_unit, execute_compile_time_unit, expand_top_level};
pub use kind::{Effect, UnitKind};
pub use location::source_location;
pub use provides::raw_provided_vars;
pub(crate) use provides::unit_kind as classify_unit_kind;
use provides::{qualify, without_metadata};
