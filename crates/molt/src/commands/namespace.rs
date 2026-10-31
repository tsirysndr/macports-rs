//! Namespace evaluation and variable links used by ports-tree resources.
use crate::{check_args, molt_err, molt_ok, ContextID, Exception, Interp, MoltResult, Value};

pub fn command(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "subcommand ?arg ...?")?;
    match argv[1].as_str() {
        "current" => {
            check_args(2, argv, 2, 2, "")?;
            molt_ok!(interp.namespace())
        }
        "eval" => {
            check_args(2, argv, 4, 0, "name arg ?arg ...?")?;
            let body = if argv.len() == 4 {
                argv[3].clone()
            } else {
                Value::from(
                    argv[3..]
                        .iter()
                        .map(|value| value.as_str().trim())
                        .collect::<Vec<_>>()
                        .join(" "),
                )
            };
            interp.eval_namespace(argv[2].as_str(), &body)
        }
        "upvar" => {
            check_args(
                2,
                argv,
                5,
                0,
                "ns otherVar localVar ?otherVar localVar ...?",
            )?;
            if !(argv.len() - 3).is_multiple_of(2) {
                return molt_err!("wrong # args: should be \"namespace upvar ns otherVar localVar ?otherVar localVar ...?\"");
            }
            let namespace = interp.qualify_variable(argv[2].as_str());
            for pair in argv[3..].as_chunks::<2>().0.iter() {
                if pair.iter().any(|name| name.as_var_name().index().is_some()) {
                    return molt_err!("namespace upvar array-element aliases are not implemented");
                }
                let other = if pair[0].as_str().starts_with("::") {
                    pair[0].to_string()
                } else {
                    format!("{}::{}", namespace.trim_end_matches(':'), pair[0])
                };
                interp.upvar_named(0, &other, pair[1].as_str())?;
            }
            molt_ok!()
        }
        other => molt_err!("unsupported namespace subcommand \"{}\"", other),
    }
}
pub fn variable(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "name ?value? ?name value ...?")?;
    let mut index = 1;
    while index < argv.len() {
        let name = argv[index].as_str();
        if name.ends_with(')') && name.contains('(') {
            return molt_err!(
                "can't define \"{}\": name refers to an element in an array",
                name
            );
        }
        let target = interp.link_namespace_variable(name)?;
        if let Some(value) = argv.get(index + 1) {
            interp.set_scalar(&target, value.clone())?;
        }
        index += 2;
    }
    molt_ok!()
}
