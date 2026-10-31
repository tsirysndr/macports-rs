//! A Portfile interpreter retained for later target/procedure invocation.
use super::*;

/// Owns the evaluated Portfile's procedures, globals, arrays, and native state.
/// Opening a session only evaluates metadata. It does not initialize PortSystem
/// or run targets. Shell execution and supported filesystem mutations require
/// explicit `enable_system`; unsupported commands fail explicitly. Read-only
/// file predicates inspect the host filesystem (relative to the process cwd).
pub struct Session {
    pub(super) interp: Interp,
    pub(super) id: ContextID,
    pub(super) metadata: Evaluation,
}

impl Session {
    /// Canonical selected variants from the retained interpreter's live state.
    pub fn active_variants(&mut self) -> Result<String> {
        check_supported(&mut self.interp, self.id)?;
        Ok(self
            .interp
            .context::<State>(self.id)
            .active
            .iter()
            .map(|name| format!("+{name}"))
            .collect())
    }
    pub fn open(
        source: &str,
        variables: &BTreeMap<String, String>,
        requested: &PortSpec,
        groups: PortGroups,
    ) -> Result<Self> {
        super::open(source, variables, requested, groups, None, None)
    }

    /// Evaluate with an explicit initial environment available to the Portfile
    /// and imported PortGroups. Does not inherit the host environment or enable
    /// shell execution. The caller supplies MacPorts-filtered startup values.
    pub fn open_with_environment(
        source: &str,
        variables: &BTreeMap<String, String>,
        requested: &PortSpec,
        groups: PortGroups,
        environment: &BTreeMap<String, String>,
    ) -> Result<Self> {
        super::open(
            source,
            variables,
            requested,
            groups,
            Some(environment),
            None,
        )
    }

    /// Install lazy archive-site resolution before evaluating options, variants,
    /// and PortGroups. Explicit Portfile and command-line values take precedence.
    pub fn open_with_archive_sites(
        source: &str,
        variables: &BTreeMap<String, String>,
        requested: &PortSpec,
        groups: PortGroups,
        environment: &BTreeMap<String, String>,
        archives: ArchiveSites,
    ) -> Result<Self> {
        super::open(
            source,
            variables,
            requested,
            groups,
            Some(environment),
            Some(archives),
        )
    }

    /// Snapshot taken after initial evaluation and variant application. Later
    /// procedure calls can change live state without altering this snapshot.
    pub fn metadata(&self) -> &Evaluation {
        &self.metadata
    }

    /// Discard the interpreter and keep only its initial metadata snapshot.
    pub fn into_metadata(self) -> Evaluation {
        self.metadata
    }

    /// Invoke one command at global scope with literal arguments. This is a
    /// scheduler building block, not a complete target runner: callers must
    /// arrange initialization, prerequisites, and target callbacks themselves.
    pub fn invoke(&mut self, command: &str, arguments: &[&str]) -> Result<String> {
        check_supported(&mut self.interp, self.id)?;
        let words = std::iter::once(command)
            .chain(arguments.iter().copied())
            .map(Value::from)
            .collect::<Vec<_>>();
        let result = self
            .interp
            .with_global_scope(|interp| interp.eval_value(&Value::from(words)));
        // A script catching an unsupported operation cannot turn it into a
        // successful phase. Preserve this failure for later calls as well.
        check_supported(&mut self.interp, self.id)?;
        result.map(|value| value.to_string()).map_err(failure)
    }

    /// Read a live global scalar, resolving native lazy defaults as necessary.
    pub fn scalar(&mut self, name: &str) -> Result<String> {
        check_supported(&mut self.interp, self.id)?;
        let result = self.interp.with_global_scope(|interp| interp.scalar(name));
        check_supported(&mut self.interp, self.id)?;
        result.map(|value| value.to_string()).map_err(failure)
    }
}
