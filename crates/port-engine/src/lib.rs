//! Portfile evaluation, builds, binary archives and installation.
pub use port_core::{
    accounts, download, fields, filemap, index, info, macho, options, order, output, progress,
    query, source, spec, tcl_list, theme,
};
pub use port_registry as registry;

pub mod build;
pub mod config;
pub mod install;
pub mod portfile;
pub mod privileges;

/// Dependency metadata from the catalog, with registry-based providers.
pub mod dependencies {
    pub use port_core::dependencies::*;
    pub mod provider;
}
