//! Pure PortSystem helpers, independent of the filesystem and external tools.
use super::*;
use molt::{MoltResult, check_args};
use regex::{Captures, Regex};
use std::sync::LazyLock;

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("quotemeta", quotemeta, id);
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
