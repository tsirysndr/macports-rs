//! MacPorts platform blocks (portutil.tcl's platform command).
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn command(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 0, "os ?release? ?arch? body")?;
    for variable in ["os.platform", "os.subplatform", "os.arch", "os.major"] {
        global(interp, variable);
    }
    let os = argv[1].as_str();
    let platform = interp.scalar("os.platform")?;
    // Match Tcl's short-circuit evaluation: absent properties are errors only
    // when required to decide this particular platform clause.
    let matches_os = os == platform.as_str() || os == interp.scalar("os.subplatform")?.as_str();
    if argv.len() == 3 && (matches_os || os == interp.scalar("os.arch")?.as_str()) {
        return interp.eval_value(argv.last().unwrap());
    }
    if !matches_os {
        return Ok(Value::empty());
    }
    let mut release = None;
    let mut architecture = None;
    for argument in &argv[2..argv.len() - 1] {
        let text = argument.as_str();
        if !text.is_empty() && text.bytes().all(|c| c.is_ascii_digit()) {
            release = Some(argument);
        } else {
            // Upstream's arch regexp captures the leading ASCII alphanumeric
            // sequence. os.arch uses families such as arm and i386.
            architecture = Some(
                text.chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect::<String>(),
            );
        }
    }
    if let Some(release) = release
        && interp.scalar("os.major")?.as_int()? != release.as_int()?
    {
        return Ok(Value::empty());
    }
    if let Some(architecture) = architecture
        && interp.scalar("os.arch")?.as_str() != architecture
    {
        return Ok(Value::empty());
    }
    interp.eval_value(argv.last().unwrap())
}
