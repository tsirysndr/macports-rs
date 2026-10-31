//! Live fetch URL selection and mirror ordering.
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portfetch::sortsites", sort, id);
    interp.add_context_command("portfetch::get_urls", urls, id);
}
pub(super) fn sites(
    interp: &mut Interp,
    tag: &str,
    default: &str,
    empty_default: bool,
) -> MoltResult {
    let key = Value::from(format!("portfetch::urlmap({tag})"));
    if !interp.var_exists(&key) {
        let value = if empty_default && tag == default {
            Value::empty()
        } else {
            interp.eval_value(&Value::from(vec![
                "ui_error".into(),
                format!("No defined site for tag: {tag}, using {default}").into(),
            ]))?;
            interp.var(&format!("portfetch::urlmap({default})").into())?
        };
        interp.set_var(&key, value)?;
    }
    interp.var(&key)
}
fn sort(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "urls default_listvar")?;
    // Read the supplied variable in the caller's frame before entering global
    // scope, matching upstream's upvar rather than treating it as a global.
    let candidates = interp.var(&argv[1])?.as_list()?;
    interp.with_global_scope(|interp| {
        for pair in candidates.chunks(2) {
            let tag = pair[0].as_str();
            let mirrors = sites(interp, tag, argv[2].as_str(), true)?;
            if mirrors.as_list()?.len() > 1 {
                let sorted = Value::from(sort_mirrors(interp, &mirrors.as_list()?)?);
                interp.set_var(&format!("portfetch::urlmap({tag})").into(), sorted)?;
            }
        }
        Ok(Value::empty())
    })
}
fn urls(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        interp.eval_value(&Value::from(vec![
            "portfetch::checkfiles".into(),
            "::portfetch::fetch_urls".into(),
        ]))?;
        let candidates = interp.scalar("portfetch::fetch_urls")?.as_list()?;
        let mut result = Vec::new();
        for pair in candidates.chunks(2) {
            result.extend(
                sites(interp, pair[0].as_str(), "master_sites", false)?
                    .as_list()?
                    .iter()
                    .cloned(),
            );
        }
        Ok(Value::from(result))
    })
}

// Stable, fallible merge sort: comparator errors must stop the fetch rather
// than be converted to an arbitrary ordering by Rust's infallible sort API.
fn sort_mirrors(interp: &mut Interp, values: &[Value]) -> Result<Vec<Value>, Exception> {
    if values.len() <= 1 {
        return Ok(values.to_vec());
    }
    let middle = values.len() / 2;
    let left = sort_mirrors(interp, &values[..middle])?;
    let right = sort_mirrors(interp, &values[middle..])?;
    let (mut a, mut b) = (0, 0);
    let mut result = Vec::with_capacity(values.len());
    while a < left.len() && b < right.len() {
        let ordering = interp
            .eval_value(&Value::from(vec![
                "compare_pingtimes".into(),
                left[a].clone(),
                right[b].clone(),
            ]))?
            .as_int()?;
        if ordering <= 0 {
            result.push(left[a].clone());
            a += 1;
        } else {
            result.push(right[b].clone());
            b += 1;
        }
    }
    result.extend_from_slice(&left[a..]);
    result.extend_from_slice(&right[b..]);
    Ok(result)
}
