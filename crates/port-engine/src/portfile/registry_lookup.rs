//! Read-only registry aliases using fresh snapshots after dependency installs.
use super::*;
use molt::{MoltResult, check_args};
use std::path::{Path, PathBuf};
pub(super) enum Source {
    Missing,
    Database(PathBuf),
}
impl Source {
    pub(super) fn from_path(path: Option<&Path>) -> Self {
        match path {
            Some(path) => Self::Database(path.to_owned()),
            None => Self::Missing,
        }
    }
}
impl Session {
    /// Bind the startup driver's selected registry. None explicitly denotes an
    /// absent registry; an unbound session cannot guess or use the host default.
    /// Each lookup opens a fresh read-only snapshot, seeing committed installs.
    pub fn set_registry_lookup(&mut self, path: Option<&Path>) {
        self.interp.context::<State>(self.id).registry_lookup = Some(match path {
            Some(path) => Source::Database(path.to_owned()),
            None => Source::Missing,
        });
    }
}
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("registry_exists", exists, id);
    interp.add_context_command("registry_exists_for_name", exists, id);
}
fn exists(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    let name_only = argv[0].as_str() == "registry_exists_for_name";
    if name_only {
        check_args(1, argv, 2, 2, "name")?;
    } else {
        check_args(1, argv, 3, 5, "name version ?revision? ?variants?")?;
    }
    let source = &interp.context::<State>(id).registry_lookup;
    let path = match source {
        None => return commands::unsupported(interp, id, argv),
        Some(Source::Missing) => return Ok(false.into()),
        Some(Source::Database(path)) => path.clone(),
    };
    // receipt_sqlite::entry_exists catches registry search errors and returns 0.
    let found = (|| -> Result<bool> {
        let Some(registry) = crate::registry::Registry::open(Some(&path))? else {
            return Ok(false);
        };
        if name_only {
            registry.contains_name(argv[1].as_str())
        } else {
            registry.contains_image(
                argv[1].as_str(),
                argv[2].as_str(),
                argv.get(3).map(Value::as_str).unwrap_or("0"),
                argv.get(4).map(Value::as_str).unwrap_or(""),
            )
        }
    })()
    .unwrap_or(false);
    Ok(found.into())
}
