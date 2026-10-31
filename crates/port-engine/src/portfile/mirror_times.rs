use super::*;
use molt::{MoltResult, check_args};
impl Session {
    pub fn set_mirror_worker(&mut self, worker: crate::install::mirror_worker::MirrorWorker) {
        self.set_mirror_timings(worker.timings());
        self.interp.context::<State>(self.id).mirror_worker = Some(worker);
    }
    /// Share host preferences and measurements across installation sessions.
    pub fn set_mirror_timings(&mut self, timings: crate::install::mirror_times::MirrorTimings) {
        let state = self.interp.context::<State>(self.id);
        state.mirror_worker = None;
        state.mirror_times = timings;
    }
}
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("compare_pingtimes", compare, id);
    interp.add_context_command("async_ping_start", start, id);
    interp.add_context_command("wait_for_pingtime", wait, id);
}
fn compare(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "a b")?;
    interp
        .context::<State>(id)
        .mirror_times
        .compare(
            argv[1].as_str(),
            argv[2].as_str(),
            std::time::SystemTime::now(),
        )
        .map(Value::from)
        .map_err(|err| error(err.to_string()))
}

fn start(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "urls")?;
    let state = interp.context::<State>(id);
    let Some(worker) = state
        .mirror_worker
        .clone()
        .filter(|_| state.system_executor.is_some())
    else {
        return commands::unsupported(interp, id, argv);
    };
    worker
        .start(argv[1].as_list()?.iter().map(ToString::to_string).collect())
        .map(|_| Value::empty())
        .map_err(|err| error(err.to_string()))
}
fn wait(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "url")?;
    let state = interp.context::<State>(id);
    let Some(worker) = state
        .mirror_worker
        .clone()
        .filter(|_| state.system_executor.is_some())
    else {
        return commands::unsupported(interp, id, argv);
    };
    worker
        .wait(argv[1].as_str())
        .map(|_| Value::empty())
        .map_err(|err| error(err.to_string()))
}
