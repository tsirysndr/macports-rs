//! Native PortSystem path and quoting helpers without external tools.
use super::*;
use molt::{MoltResult, check_args};
use regex::{Captures, Regex};
use std::sync::LazyLock;

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("quotemeta", quotemeta, id);
    interp.add_context_command("shellescape", shellescape, id);
    interp.add_context_command("macports::shellescape", shellescape, id);
    interp.add_context_command("realpath", realpath, id);
    interp.add_context_command("portutil::get_oldbuildpath", old_build_path, id);
    interp.add_context_command("vercmp", vercmp, id);
}

/// `vercmp a b` returns -1, 0 or 1; `vercmp a op b` returns a boolean.
fn vercmp(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 4, "versionA ?operator? versionB")?;
    let (a, b) = (argv[1].as_str(), argv[argv.len() - 1].as_str());
    let order = crate::registry::compare_versions(a, b);
    if argv.len() == 3 {
        return Ok((order as i64).into());
    }
    use std::cmp::Ordering::*;
    let result = match argv[2].as_str() {
        "==" => order == Equal,
        "!=" => order != Equal,
        "<" => order == Less,
        "<=" => order != Greater,
        ">" => order == Greater,
        ">=" => order != Less,
        other => return Err(error(format!("unknown vercmp operator \"{other}\""))),
    };
    Ok(result.into())
}

fn quotemeta(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "str")?;
    // Portlib quotes every non-word character, including spaces and punctuation
    // that is not special to a particular regex engine. Tcl's regex word class
    // is Unicode alphanumeric characters plus underscore, not Rust regex's \w.
    static NON_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\p{L}\p{Nd}_]").unwrap());
    Ok(NON_WORD
        .replace_all(argv[1].as_str(), |capture: &Captures<'_>| {
            format!("\\{}", &capture[0])
        })
        .into_owned()
        .into())
}

fn shellescape(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "arg")?;
    let mut escaped = String::new();
    for character in argv[1].as_str().chars() {
        if !character.is_ascii_alphanumeric() && !".:@%/+=_-".contains(character) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    Ok(escaped.into())
}

fn realpath(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "path")?;
    let path = std::fs::canonicalize(argv[1].as_str()).map_err(|err| {
        // Pextlib returns strerror(errno) with the default NONE error code.
        let message = err
            .raw_os_error()
            .map(|errno| {
                // Rust wraps the native message with a numeric suffix; Pextlib
                // exposes only the native message.
                let suffix = format!(" (os error {errno})");
                err.to_string().trim_end_matches(&suffix).to_owned()
            })
            .unwrap_or_else(|| err.to_string());
        Exception::molt_err(format!("realpath failed: {message}").into())
    })?;
    path.into_os_string()
        .into_string()
        .map(Value::from)
        .map_err(|_| error("realpath result is not valid UTF-8"))
}

fn old_build_path(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        let build = interp.scalar("portbuildpath")?;
        let port = interp.scalar("portpath")?;
        let parent =
            interp.eval_value(&Value::from(vec!["file".into(), "dirname".into(), build]))?;
        let mapped = port.as_str().replace("://", ".").replace('/', "_");
        interp.eval_value(&Value::from(vec![
            "file".into(),
            "join".into(),
            parent,
            mapped.into(),
        ]))
    })
}
