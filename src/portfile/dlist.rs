//! Native dependency-item selection and evaluation used by PortSystem targets.
use super::*;
use molt::{MoltResult, check_args};
use std::collections::HashSet;

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for name in [
        "dlist_search",
        "dlist_delete",
        "dlist_has_pending",
        "dlist_count_unmet",
        "dlist_append_dependents",
        "dlist_get_next",
        "dlist_eval",
    ] {
        interp.add_context_command(name, command, id);
    }
}
fn field_list(
    interp: &mut Interp,
    id: ContextID,
    item: &Value,
    key: &str,
) -> Result<Vec<Value>, Exception> {
    Ok(ditems::field(interp, id, item, key)?.as_list()?.to_vec())
}
fn search(
    interp: &mut Interp,
    id: ContextID,
    items: &[Value],
    key: &str,
    token: &Value,
) -> Result<Vec<Value>, Exception> {
    let mut result = Vec::new();
    for item in items {
        if field_list(interp, id, item, key)?.contains(token) {
            result.push(item.clone());
        }
    }
    Ok(result)
}
fn pending(
    interp: &mut Interp,
    id: ContextID,
    items: &[Value],
    tokens: &[Value],
) -> Result<bool, Exception> {
    for token in tokens {
        if !search(interp, id, items, "provides", token)?.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}
fn unmet(interp: &Interp, status: &str, tokens: &[Value]) -> Result<usize, Exception> {
    let mut count = 0;
    for token in tokens {
        if let Ok(value) = interp.element(status, token.as_str())
            && numeric_equal(&value, 1)
        {
            continue;
        }
        count += 1;
    }
    Ok(count)
}
fn next(interp: &mut Interp, id: ContextID, items: &[Value], status: &str) -> MoltResult {
    let mut available = HashSet::new();
    for item in items {
        available.extend(
            field_list(interp, id, item, "provides")?
                .iter()
                .map(ToString::to_string),
        );
    }
    let mut best = Value::empty();
    let mut minimum = usize::MAX;
    for item in items {
        let requires = field_list(interp, id, item, "requires")?;
        if unmet(interp, status, &requires)? != 0 {
            continue;
        }
        let uses = field_list(interp, id, item, "uses")?;
        let count = unmet(interp, status, &uses)?;
        // Match native dlist_get_next: soft dependencies defer while any of
        // their providers remain pending; unavailable soft tokens are allowed.
        if count > 0 && uses.iter().any(|token| available.contains(token.as_str())) {
            continue;
        }
        if count < minimum {
            best = item.clone();
            minimum = count;
        }
    }
    Ok(best)
}
fn command(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    match argv[0].as_str() {
        "dlist_search" => {
            check_args(1, argv, 4, 4, "dlist key value")?;
            Ok(search(interp, id, &argv[1].as_list()?, argv[2].as_str(), &argv[3])?.into())
        }
        "dlist_delete" => {
            check_args(1, argv, 3, 3, "dlistVariable ditem")?;
            let mut items = interp.var(&argv[1])?.as_list()?.to_vec();
            let removed = items
                .iter()
                .position(|item| item == &argv[2])
                .map(|index| items.remove(index))
                .unwrap_or_else(Value::empty);
            interp.set_var(&argv[1], items.into())?;
            Ok(removed)
        }
        "dlist_has_pending" => {
            check_args(1, argv, 3, 3, "dlist tokens")?;
            Ok(pending(interp, id, &argv[1].as_list()?, &argv[2].as_list()?)?.into())
        }
        "dlist_count_unmet" => {
            check_args(1, argv, 4, 4, "dlist statusdict tokens")?;
            Ok((unmet(interp, argv[2].as_str(), &argv[3].as_list()?)? as i64).into())
        }
        "dlist_get_next" => {
            check_args(1, argv, 3, 3, "dlist statusdict")?;
            next(interp, id, &argv[1].as_list()?, argv[2].as_str())
        }
        "dlist_append_dependents" => {
            check_args(1, argv, 4, 4, "dlist ditem result")?;
            let items = argv[1].as_list()?;
            let mut providers: BTreeMap<String, Vec<Value>> = BTreeMap::new();
            for item in items.iter() {
                for token in field_list(interp, id, item, "provides")? {
                    providers
                        .entry(token.to_string())
                        .or_default()
                        .push(item.clone());
                }
            }
            let mut result = argv[3].as_list()?.to_vec();
            let mut seen: HashSet<String> = result.iter().map(ToString::to_string).collect();
            let mut stack = vec![argv[2].clone()];
            while let Some(item) = stack.pop() {
                if !seen.insert(item.to_string()) {
                    continue;
                }
                result.push(item.clone());
                let mut children = Vec::new();
                for token in field_list(interp, id, &item, "requires")? {
                    if let Some(matches) = providers.get(token.as_str()) {
                        children.extend(matches.iter().cloned());
                    }
                }
                stack.extend(children.into_iter().rev());
            }
            Ok(result.into())
        }
        "dlist_eval" => evaluate(interp, id, argv),
        _ => unreachable!(),
    }
}
fn call(
    interp: &mut Interp,
    command: &Value,
    arguments: impl IntoIterator<Item = Value>,
    expand: bool,
) -> MoltResult {
    let mut words = if expand {
        command.as_list()?.to_vec()
    } else {
        vec![command.clone()]
    };
    words.extend(arguments);
    interp.eval_value(&Value::from(words))
}
fn mark(interp: &mut Interp, id: ContextID, item: &Value, success: bool) -> MoltResult {
    for token in field_list(interp, id, item, "provides")? {
        interp.set_element(
            "statusdict",
            token.as_str(),
            Value::from(i64::from(success)),
        )?;
    }
    Ok(Value::empty())
}
fn evaluate(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(
        1,
        argv,
        4,
        7,
        "dlist testcond handler ?canfail? ?selector? ?reasonVariable?",
    )?;
    let mut items = argv[1].as_list()?.to_vec();
    let can_fail = argv
        .get(4)
        .map(Value::as_bool)
        .transpose()?
        .unwrap_or(false);
    let selector = argv
        .get(5)
        .cloned()
        .unwrap_or_else(|| "dlist_get_next".into());
    let reason_name = argv
        .get(6)
        .cloned()
        .unwrap_or_else(|| "dlist_eval_reason".into());
    // dlist_eval has its own Tcl procedure scope. Custom selectors receive the
    // local status array's name and can upvar it exactly as upstream selectors do.
    let mut reason = "";
    interp.push_scope();
    let result = (|| -> MoltResult {
        interp.array_set("statusdict", &[])?;
        if !argv[2].as_str().is_empty() {
            for item in items.clone() {
                if numeric_equal(&call(interp, &argv[2], [item.clone()], false)?, 1) {
                    mark(interp, id, &item, true)?;
                    if let Some(index) = items.iter().position(|value| value == &item) {
                        items.remove(index);
                    }
                }
            }
        }
        loop {
            interp.set_scalar("dlist", items.clone().into())?;
            let item = call(
                interp,
                &selector,
                [items.clone().into(), "statusdict".into()],
                false,
            )?;
            if item.as_str().is_empty() {
                if !items.is_empty() {
                    reason = "unmet_deps";
                }
                break;
            }
            let index = items
                .iter()
                .position(|value| value == &item)
                .ok_or_else(|| error("dlist selector returned an item outside the pending list"))?;
            let outcome = match call(interp, &argv[3], [item.clone()], true) {
                Ok(value) => value,
                Err(_) => {
                    reason = "handler";
                    break;
                }
            };
            let success = outcome.as_str().is_empty() || numeric_equal(&outcome, 0);
            mark(interp, id, &item, success)?;
            if !can_fail && !success {
                reason = "handler";
                break;
            }
            items.remove(index);
        }
        Ok(items.clone().into())
    })();
    interp.pop_scope();
    if !reason_name.as_str().is_empty() {
        interp.set_var(&reason_name, reason.into())?;
    }
    result
}

fn numeric_equal(value: &Value, expected: i64) -> bool {
    value.as_int().is_ok_and(|number| number == expected)
        || value
            .as_float()
            .is_ok_and(|number| number == expected as f64)
}
