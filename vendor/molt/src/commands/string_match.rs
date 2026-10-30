use crate::{
    check_args, glob::Glob, molt_err, molt_ok, ContextID, Exception, Interp, MoltResult, Value,
};

pub(super) fn command(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 4, 5, "?-nocase? pattern string")?;
    let nocase = argv.len() == 5;
    if nocase {
        let option = argv[2].as_str();
        if option.len() <= 1 || !"-nocase".starts_with(option) {
            return molt_err!("bad option \"{}\": must be -nocase", option);
        }
    }
    let pattern = argv[argv.len() - 2].as_str();
    let value = argv[argv.len() - 1].as_str();
    molt_ok!(Glob::new(pattern, !nocase).matches(value))
}
