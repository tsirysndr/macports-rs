use super::*;
use molt::{MoltResult, check_args};
impl Session {
    /// Share host preferences and measurements across installation sessions.
    pub fn set_mirror_timings(&mut self, timings: crate::install::mirror_times::MirrorTimings) {
        self.interp.context::<State>(self.id).mirror_times = timings;
    }
}
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("compare_pingtimes", compare, id);
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
