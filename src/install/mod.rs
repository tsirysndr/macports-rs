//! Preliminary binary installation planning from indexed or evaluated metadata.
mod evaluated;
pub use evaluated::{EvaluatedPlan, plan_evaluated};
mod plan;
pub mod signature;
mod variants;
pub use plan::{Action, Item, plan};

pub mod archive;

pub mod inspect;

pub mod extract;

pub mod fetch;

pub mod target;

pub mod activation;

pub mod binary;
pub mod publication;
pub mod store;
