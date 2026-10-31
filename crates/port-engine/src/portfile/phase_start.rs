//! Native startup callbacks for the build and test runtime packages.
use super::*;
use crate::progress::Progress;
use molt::MoltResult;

impl Session {
    /// Share the installer's presentation manager, including quiet mode and
    /// concurrent download bars, with native phase startup callbacks.
    pub fn set_progress(&mut self, progress: Progress) {
        self.interp.context::<State>(self.id).progress = Some(progress);
    }
    pub fn progress(&mut self) -> Option<Progress> {
        self.interp.context::<State>(self.id).progress.clone()
    }
}
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portbuild::build_start", build, id);
    interp.add_context_command("porttest::test_start", test, id);
}
pub(super) fn notice(interp: &mut Interp, id: ContextID, argv: &[Value], verb: &str) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        let subport = interp.scalar("subport")?;
        let prefix = interp.scalar("UI_PREFIX")?;
        let progress = interp
            .context::<State>(id)
            .progress
            .clone()
            .unwrap_or_else(|| Progress::new(false));
        progress.phase_with_prefix(prefix.as_str(), &format!("{verb} {subport}"));
        Ok(Value::empty())
    })
}
fn build(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    notice(interp, id, argv, "Building")?;
    warn_compiler_fallback(interp, id)
}
pub(super) fn warn_compiler_fallback(interp: &mut Interp, id: ContextID) -> MoltResult {
    interp.with_global_scope(|interp| {
        if interp.scalar("portconfigure::no_default_compiler_allowed")?.as_bool()? {
            let state = interp.context::<State>(id);
            if state.warned.insert("no_default_compiler_allowed".into()) {
                state.progress.clone().unwrap_or_else(|| Progress::new(false)).warning(
                    "All compilers are either blacklisted or unavailable; defaulting to first fallback option",
                );
            }
        }
        Ok(Value::empty())
    })
}
fn test(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    notice(interp, id, argv, "Testing")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compiler_fallback_warning_is_recorded_once_per_session() {
        let mut s = Session::open(
            "name app; version 1",
            &BTreeMap::new(),
            &PortSpec::named("app"),
            PortGroups::default(),
        )
        .unwrap();
        s.enable_system(&BTreeMap::new(), |_| panic!("startup needs no shell"))
            .unwrap();
        s.set_progress(Progress::new(true));
        s.invoke("portbuild::build_start", &[]).unwrap();
        assert!(s.interp.context::<State>(s.id).warned.is_empty());
        s.invoke("set", &["portconfigure::no_default_compiler_allowed", "1"])
            .unwrap();
        for _ in 0..2 {
            s.invoke("portbuild::build_start", &["ignored", "args"])
                .unwrap();
        }
        s.invoke("configure.compiler", &["clang"]).unwrap();
        s.invoke("portconfigure::configure_start", &[]).unwrap();
        assert_eq!(s.interp.context::<State>(s.id).warned.len(), 1);
        assert!(
            s.interp
                .context::<State>(s.id)
                .warned
                .contains("no_default_compiler_allowed")
        );
    }
}
