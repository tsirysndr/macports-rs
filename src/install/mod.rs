//! Preliminary binary installation planning from indexed metadata, without Portfile evaluation.
mod plan;
pub mod signature;
mod variants;
pub use plan::{Action, Item, plan};

pub mod archive;

pub mod inspect;
