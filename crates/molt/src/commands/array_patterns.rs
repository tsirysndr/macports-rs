//! Pattern selection for Tcl array queries and removals.
use crate::{
    check_args, glob::Glob, molt_err, molt_ok, ContextID, Exception, Interp, MoltResult, Value,
};

pub(super) fn names(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 3, 5, "arrayName ?mode? ?pattern?")?;
    let exact = if argv.len() == 5 {
        let mode = argv[3].as_str();
        if mode.len() > 1 && "-exact".starts_with(mode) {
            true
        } else if mode.len() > 1 && "-glob".starts_with(mode) {
            false
        } else if mode.len() > 1 && "-regexp".starts_with(mode) {
            return molt_err!("array names -regexp is not implemented");
        } else {
            return molt_err!("bad option \"{}\": must be -exact, -glob, or -regexp", mode);
        }
    } else {
        false
    };
    let mut names = interp.array_names(argv[2].as_str());
    if argv.len() >= 4 {
        let pattern = argv.last().unwrap().as_str();
        if exact {
            names.retain(|name| name.as_str() == pattern);
        } else {
            let glob = Glob::new(pattern, true);
            names.retain(|name| glob.matches(name.as_str()));
        }
    }
    molt_ok!(Value::from(names))
}

pub(super) fn get(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 3, 4, "arrayName ?pattern?")?;
    let entries = interp.array_get(argv[2].as_str());
    if argv.len() == 3 {
        return molt_ok!(Value::from(entries));
    }
    let glob = Glob::new(argv[3].as_str(), true);
    let selected: Vec<Value> = entries
        .as_chunks::<2>()
        .0
        .iter()
        .filter(|pair| glob.matches(pair[0].as_str()))
        .flat_map(|pair| pair.iter().cloned())
        .collect();
    molt_ok!(Value::from(selected))
}

pub(super) fn unset(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(2, argv, 3, 4, "arrayName ?pattern?")?;
    let name = argv[2].as_str();
    if argv.len() == 3 {
        interp.array_unset(name);
    } else {
        let glob = Glob::new(argv[3].as_str(), true);
        for key in interp.array_names(name) {
            if glob.matches(key.as_str()) {
                interp.unset_element(name, key.as_str());
            }
        }
    }
    molt_ok!()
}
