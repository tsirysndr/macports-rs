//! Preliminary binary installation planning from indexed or evaluated metadata.
mod evaluated;
pub use evaluated::{
    EvaluatedPlan, SessionPlan, plan_evaluated, plan_evaluated_with_providers, plan_sessions,
    plan_source_sessions,
};
mod plan;
pub mod signature;
mod variants;
pub use plan::{Action, Item, plan, plan_with_providers};

pub mod archive;

pub mod inspect;

pub mod extract;

pub mod fetch;

pub mod target;

pub mod activation;

pub mod binary;
pub mod publication;
pub mod registration;
pub mod store;

pub mod environment;
pub mod proxy;

pub mod pubkeys;

pub mod sites;

pub mod mirror_times;

pub mod mirror_probe;

pub mod mirror_worker;

pub mod source;
