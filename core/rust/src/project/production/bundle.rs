#[path = "bundle/compile.rs"]
pub(super) mod compile;
#[path = "bundle/load.rs"]
pub(super) mod load;
#[path = "bundle/model.rs"]
mod model;
#[path = "bundle/order.rs"]
mod order;
#[path = "bundle/render.rs"]
mod render;

pub(super) use model::ProductionBuild;

#[cfg(test)]
#[path = "bundle/tests.rs"]
mod tests;
