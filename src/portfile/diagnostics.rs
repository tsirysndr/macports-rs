//! Portfile diagnostics shared with the native progress display.
use super::*;
use molt::{MoltResult, check_args};
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for name in [
        "ui_error",
        "ui_warn",
        "ui_msg",
        "ui_notice",
        "ui_info",
        "ui_debug",
    ] {
        interp.add_context_command(name, message, id);
    }
}
fn message(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    // Upstream accepts `?-nonewline? ?channel? message`; only the text matters here.
    check_args(1, argv, 1, 4, "?-nonewline? ?channel? message")?;
    let text = argv
        .last()
        .filter(|_| argv.len() > 1)
        .map(Value::as_str)
        .unwrap_or("");
    let progress = interp
        .context::<State>(id)
        .progress
        .clone()
        .unwrap_or_else(|| crate::progress::Progress::new(false));
    match argv[0].as_str() {
        "ui_warn" => progress.warning(text),
        "ui_error" => progress.error(text),
        "ui_msg" | "ui_notice" => progress.message(text),
        command => progress.log(command.trim_start_matches("ui_"), text),
    }
    // ui_error reports a diagnostic; it does not raise a Tcl exception.
    Ok(Value::empty())
}
