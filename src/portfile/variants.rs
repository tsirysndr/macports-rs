use super::*;
use molt::{MoltResult, check_args};
pub(super) fn declare(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(
        1,
        argv,
        3,
        0,
        "name ?requires variants? ?conflicts variants? ?description text? body",
    )?;
    let mut provides = Vec::new();
    let mut description = Vec::new();
    let mut variant = Variant {
        description: String::new(),
        requires: vec![],
        conflicts: vec![],
        body: argv.last().unwrap().to_string(),
    };
    let mut mode = "provides";
    for value in &argv[1..argv.len() - 1] {
        match value.as_str() {
            "provides" | "requires" | "conflicts" | "description" => mode = value.as_str(),
            _ => match mode {
                "provides" => provides.push(value.to_string()),
                "description" => description.push(value.to_string()),
                "requires" => variant.requires.push(value.to_string()),
                "conflicts" => variant.conflicts.push(value.to_string()),
                _ => unreachable!(),
            },
        }
    }
    if provides.len() != 1 {
        return Err(error("variant must provide exactly one name"));
    }
    let name = provides[0].as_str();
    if name.is_empty()
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_.".contains(&c))
    {
        return Err(error("invalid variant name"));
    }
    variant.description = description.join(" ");
    let state = interp.context::<State>(id);
    if !state.variants.contains_key(name) {
        state.order.push(name.into());
    }
    if state.selected.get(name) == Some(&true) {
        for requirement in &variant.requires {
            state.selected.insert(requirement.clone(), true);
        }
    }
    state.variants.insert(name.into(), variant);
    Ok(Value::empty())
}
pub(super) fn query(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "name")?;
    let state = interp.context::<State>(id);
    let name = argv[1].as_str();
    Ok(match argv[0].as_str() {
        "variant_exists" => Value::from(state.variants.contains_key(name)),
        "variant_isset" => Value::from(state.selected.get(name) == Some(&true)),
        "variant_set" => {
            state.selected.insert(name.into(), true);
            Value::empty()
        }
        "variant_unset" => {
            state.selected.insert(name.into(), false);
            Value::empty()
        }
        _ => unreachable!(),
    })
}
pub(super) fn apply(interp: &mut Interp, id: ContextID) -> MoltResult {
    let state = interp.context::<State>(id);
    let mut events: Vec<_> = state
        .order
        .iter()
        .rev()
        .filter(|name| state.selected.get(*name) == Some(&true))
        .map(|name| (name.clone(), false))
        .collect();
    let mut visiting = BTreeSet::new();
    let mut done = BTreeSet::new();
    let mut ordered = Vec::new();
    while let Some((name, leave)) = events.pop() {
        if leave {
            visiting.remove(&name);
            done.insert(name.clone());
            ordered.push(name);
            continue;
        }
        if done.contains(&name) {
            continue;
        }
        if !visiting.insert(name.clone()) {
            return Err(error(format!("variant requirement cycle at {name}")));
        }
        if state.selected.get(&name) == Some(&false) || state.requested.get(&name) == Some(&false) {
            return Err(error(format!(
                "required variant {name} was explicitly disabled"
            )));
        }
        let variant = state
            .variants
            .get(&name)
            .ok_or_else(|| error(format!("required variant {name} is not declared")))?;
        events.push((name.clone(), true));
        events.extend(
            variant
                .requires
                .iter()
                .rev()
                .map(|name| (name.clone(), false)),
        );
    }
    for name in &ordered {
        for conflict in &state.variants[name].conflicts {
            if done.contains(conflict) {
                return Err(error(format!("variant {name} conflicts with {conflict}")));
            }
        }
        state.selected.insert(name.clone(), true);
    }
    state.active = done;
    let scripts: Vec<_> = ordered
        .iter()
        .map(|name| state.variants[name].body.clone())
        .collect();
    for script in scripts {
        interp.eval(&script)?;
    }
    Ok(Value::empty())
}

pub(super) fn defaults(interp: &mut Interp, id: ContextID, values: &[Value]) -> MoltResult {
    for value in values {
        let mut spec = PortSpec::named("");
        spec.parse_modifier(value.as_str())
            .map_err(|e| error(e.to_string()))?;
        if spec.version.is_some() || !spec.options.is_empty() {
            return Err(error("invalid default variant"));
        }
        let state = interp.context::<State>(id);
        for (variant, enabled) in spec.variants {
            state.defaults.insert(variant.clone(), enabled);
            state.selected.entry(variant).or_insert(enabled);
        }
    }
    Ok(Value::empty())
}
