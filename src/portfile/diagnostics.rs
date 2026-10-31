//! Portfile diagnostics shared with the native progress display.
use super::*;
use molt::{MoltResult, check_args};
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("ui_error", message, id);
    interp.add_context_command("ui_warn", message, id);
}
fn message(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "message")?;
    let progress = interp
        .context::<State>(id)
        .progress
        .clone()
        .unwrap_or_else(|| crate::progress::Progress::new(false));
    if argv[0].as_str().ends_with("ui_warn") {
        progress.warning(argv[1].as_str());
    } else {
        progress.message(&format!("Error: {}", argv[1]));
    }
    // ui_error reports a diagnostic; it does not raise a Tcl exception.
    Ok(Value::empty())
}
