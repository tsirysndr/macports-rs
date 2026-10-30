//! Bind host-owned locked state files to PortSystem state helper commands.
use super::*;
use crate::build::state::StateFile;
use molt::{MoltResult, check_args};

impl Session {
    /// Prepare real-build state after work-directory/privilege setup, then
    /// transfer its lock into this session. The returned compatibility reports
    /// the pre-cleanup state (including unknown versions for host warnings).
    pub fn open_build_state(
        &mut self,
        work: &std::path::Path,
        subport: &str,
        portfile: &std::path::Path,
        ignore_different: bool,
    ) -> Result<(String, crate::build::state::Compatibility)> {
        check_supported(&mut self.interp, self.id)?;
        let state = self.interp.context::<State>(self.id);
        ensure!(
            state.system_executor.is_some(),
            "build state requires an execution-enabled session"
        );
        let prepared = crate::build::work_state::prepare(
            work,
            subport,
            portfile,
            state.creation_mask,
            ignore_different,
        )?;
        let handle = self.attach_build_state(prepared.state)?;
        Ok((handle, prepared.compatibility))
    }
    /// Transfer a locked state file to an execution-enabled session. The host
    /// still performs open_statefile policy, including stale-work cleanup.
    /// The returned opaque handle is accepted by state helpers and `close`.
    pub fn attach_build_state(&mut self, file: StateFile) -> Result<String> {
        check_supported(&mut self.interp, self.id)?;
        let state = self.interp.context::<State>(self.id);
        ensure!(
            state.system_executor.is_some(),
            "build state requires an execution-enabled session"
        );
        attach(state, file)
    }
    /// Close a transferred handle and release its lock, without Tcl evaluation.
    pub fn close_build_state(&mut self, handle: &str) -> Result<()> {
        self.interp
            .context::<State>(self.id)
            .build_states
            .remove(handle)
            .map(drop)
            .ok_or_else(|| anyhow::anyhow!("unknown build-state handle \"{handle}\""))
    }
}
pub(super) fn attach(state: &mut State, file: StateFile) -> Result<String> {
    state.next_build_state = state
        .next_build_state
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("build state handle limit reached"))?;
    let handle = format!("port_rs_state{}", state.next_build_state);
    state.build_states.insert(handle.clone(), file);
    Ok(handle)
}
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("open_statefile", super::state_open::open, id);
    interp.add_context_command("check_statefile", check, id);
    interp.add_context_command("write_statefile", write, id);
    interp.add_context_command("get_statefile_value", get, id);
    interp.add_context_command("check_statefile_variants", variants, id);
    interp.add_context_command("close", close, id);
}
fn file<'a>(
    interp: &'a mut Interp,
    id: ContextID,
    handle: &Value,
) -> Result<&'a mut StateFile, Exception> {
    interp
        .context::<State>(id)
        .build_states
        .get_mut(handle.as_str())
        .ok_or_else(|| {
            Exception::molt_err(format!("unknown build-state handle \"{handle}\"").into())
        })
}
fn io_error(err: anyhow::Error) -> Exception {
    Exception::molt_err(err.to_string().into())
}
fn check(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 4, 4, "class id fd")?;
    Ok(file(interp, id, &argv[3])?
        .contains(argv[1].as_str(), argv[2].as_str())
        .map_err(io_error)?
        .into())
}
fn write(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 4, 4, "class id fd")?;
    if file(interp, id, &argv[3])?
        .record(argv[1].as_str(), argv[2].as_str())
        .map_err(io_error)?
    {
        Ok(Value::empty())
    } else {
        Ok(0.into())
    }
}
fn get(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 4, 4, "class fd result")?;
    let pattern = regex::Regex::new(&format!("{}: (.*)", argv[1]))
        .map_err(|err| Exception::molt_err(format!("invalid state class pattern: {err}").into()))?;
    for line in file(interp, id, &argv[2])?.lines().map_err(io_error)? {
        if let Some(captures) = pattern.captures(&line) {
            interp.set_var(
                &argv[3],
                captures
                    .get(1)
                    .map(|value| value.as_str())
                    .unwrap_or("")
                    .into(),
            )?;
            return Ok(1.into());
        }
    }
    Ok(0.into())
}
fn variants(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 4, 4, "variations oldvariations fd")?;
    let (found, old) = file(interp, id, &argv[3])?.variants().map_err(io_error)?;
    let value = Value::from(
        old.iter()
            .flat_map(|(key, value)| [Value::from(key.as_str()), Value::from(value.as_str())])
            .collect::<Vec<_>>(),
    );
    interp.set_var(&argv[2], value)?;
    if !found {
        return Ok(0.into());
    }
    let requested = interp.var(&argv[1])?;
    let requested = requested.as_dict()?;
    let requested: BTreeMap<_, _> = requested
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    Ok((old != requested).into())
}
fn close(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "channelId")?;
    if interp
        .context::<State>(id)
        .build_states
        .remove(argv[1].as_str())
        .is_none()
    {
        return Err(Exception::molt_err(
            format!("can not find channel named \"{}\"", argv[1]).into(),
        ));
    }
    Ok(Value::empty())
}
