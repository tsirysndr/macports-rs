//! Preliminary binary installation planning from indexed metadata, without Portfile evaluation.
mod plan;
mod variants;
pub use plan::{Action, Item, plan};
