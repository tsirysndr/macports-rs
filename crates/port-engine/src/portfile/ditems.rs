//! Opaque MacPorts dependency items; operations manipulate data, not phases.
use super::*;
use molt::{MoltDict, MoltResult, check_args};

#[derive(Default)]
pub(super) struct Items {
    next: u64,
    values: BTreeMap<String, MoltDict>,
    unique: BTreeMap<(String, String), Vec<Value>>,
    targets: Vec<String>,
}
impl Items {
    pub(super) fn snapshot(&self) -> BTreeMap<String, BTreeMap<String, String>> {
        self.values
            .iter()
            .map(|(handle, fields)| {
                (
                    handle.clone(),
                    fields
                        .iter()
                        .map(|(key, value)| (key.to_string(), value.to_string()))
                        .collect(),
                )
            })
            .collect()
    }
    fn create(&mut self) -> Result<String, Exception> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| error("dependency item IDs exhausted"))?;
        let handle = format!("ditem_{}", self.next);
        self.values.insert(handle.clone(), MoltDict::new());
        Ok(handle)
    }
    fn get(&self, handle: &str) -> Result<&MoltDict, Exception> {
        self.values
            .get(handle)
            .ok_or_else(|| error(format!("unknown dependency item {handle}")))
    }
}

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for name in [
        "ditem_create",
        "ditem_delete",
        "ditem_key",
        "ditem_append",
        "ditem_append_unique",
        "ditem_contains",
        "ditem_as_string",
    ] {
        interp.add_context_command(name, command, id);
    }
    interp.add_context_command("target_new", target_new, id);
    interp.add_context_command("makeuserproc", makeuserproc, id);
    for name in [
        "requires", "uses", "deplist", "prerun", "postrun", "runtype", "state", "init", "runpkg",
    ] {
        interp.add_context_command(&format!("target_{name}"), target_property, id);
    }
}

pub(super) fn new_target(
    interp: &mut Interp,
    id: ContextID,
    name: &str,
    procedure: &str,
) -> MoltResult {
    let items = &mut interp.context::<State>(id).ditems;
    let handle = items.create()?;
    let fields = items.values.get_mut(&handle).unwrap();
    fields.insert("name".into(), name.into());
    fields.insert("procedure".into(), procedure.into());
    items.targets.push(handle.clone());
    let targets = list(items.targets.clone());
    interp.with_global_scope(|interp| interp.set_scalar("portutil::targets", targets))?;
    Ok(handle.into())
}
pub(super) fn field(interp: &mut Interp, id: ContextID, handle: &Value, key: &str) -> MoltResult {
    Ok(interp
        .context::<State>(id)
        .ditems
        .get(handle.as_str())?
        .get(&Value::from(key))
        .cloned()
        .unwrap_or_else(Value::empty))
}
pub(super) fn userproc(interp: &mut Interp, name: &str, body: &str) -> MoltResult {
    let body = format!("global {{*}}[info globals]\n{body}");
    interp.eval_value(&Value::from(vec![
        "proc".into(),
        format!("::{}", name.trim_start_matches("::")).into(),
        Value::empty(),
        body.into(),
    ]))
}
fn makeuserproc(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "name body")?;
    userproc(interp, argv[1].as_str(), argv[2].as_str())
}
pub(super) fn set(interp: &mut Interp, id: ContextID, handle: &Value, key: &str, value: Value) {
    interp
        .context::<State>(id)
        .ditems
        .values
        .entry(handle.to_string())
        .or_default()
        .insert(key.into(), value);
}
fn target_new(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "name procedure")?;
    new_target(interp, id, argv[1].as_str(), argv[2].as_str())
}
fn target_property(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    let key = argv[0].as_str().strip_prefix("target_").unwrap();
    if key == "runpkg" {
        check_args(1, argv, 3, 3, "ditem package")?;
        set(interp, id, &argv[1], key, argv[2].clone());
        Ok(argv[2].clone())
    } else {
        check_args(1, argv, 2, 0, "ditem ?value ...?")?;
        let mut args = vec![Value::from("ditem_append"), argv[1].clone(), key.into()];
        args.extend_from_slice(&argv[2..]);
        command(interp, id, &args)
    }
}
fn command(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    let name = argv[0].as_str();
    if name == "ditem_create" {
        check_args(1, argv, 1, 1, "")?;
        return Ok(interp.context::<State>(id).ditems.create()?.into());
    }
    let (min, max, usage) = match name {
        "ditem_delete" | "ditem_as_string" => (2, 2, "ditem"),
        "ditem_key" => (2, 0, "ditem ?key ?value??"),
        _ => (3, 0, "ditem key ?value ...?"),
    };
    check_args(1, argv, min, max, usage)?;
    let handle = argv[1].as_str();
    let items = &mut interp.context::<State>(id).ditems;
    match name {
        "ditem_delete" => {
            items.get(handle)?;
            items.values.remove(handle);
            items.unique.retain(|(item, _), _| item != handle);
            Ok(Value::empty())
        }
        "ditem_as_string" => Ok(Value::from(items.get(handle)?.clone())),
        "ditem_key" => {
            if argv.len() >= 4 {
                items
                    .values
                    .entry(handle.into())
                    .or_default()
                    .insert(argv[2].clone(), argv[3].clone());
                Ok(argv[3].clone())
            } else if argv.len() == 3 {
                Ok(items
                    .get(handle)?
                    .get(&argv[2])
                    .cloned()
                    .unwrap_or_else(Value::empty))
            } else {
                Ok(items
                    .values
                    .get(handle)
                    .cloned()
                    .map(Value::from)
                    .unwrap_or_else(Value::empty))
            }
        }
        "ditem_contains" => {
            let value = items.get(handle)?.get(&argv[2]);
            let found = if let Some(needle) = argv.get(3) {
                value
                    .map(|value| value.as_list().map(|list| list.contains(needle)))
                    .transpose()?
                    .unwrap_or(false)
            } else {
                value.is_some()
            };
            Ok(found.into())
        }
        "ditem_append" => {
            let fields = items.values.entry(handle.into()).or_default();
            let mut values = fields
                .get(&argv[2])
                .map(|v| v.as_list().map(|v| v.to_vec()))
                .transpose()?
                .unwrap_or_default();
            values.extend_from_slice(&argv[3..]);
            let value = Value::from(values);
            fields.insert(argv[2].clone(), value.clone());
            Ok(value)
        }
        "ditem_append_unique" => {
            let cache_key = (handle.to_owned(), argv[2].to_string());
            let mut values = if let Some(values) = items.unique.get(&cache_key) {
                values.clone()
            } else {
                items
                    .get(handle)?
                    .get(&argv[2])
                    .map(|v| v.as_list().map(|v| v.to_vec()))
                    .transpose()?
                    .unwrap_or_default()
            };
            // The native API keeps a per-key cache, including when other ditem
            // operations subsequently replace the value.
            let mut seen = BTreeSet::new();
            values.extend_from_slice(&argv[3..]);
            values.retain(|value| seen.insert(value.to_string()));
            if values.is_empty() && !items.unique.contains_key(&cache_key) {
                return Err(error("no values for unique dependency item key"));
            }
            items.unique.insert(cache_key, values.clone());
            let value = Value::from(values);
            items
                .values
                .entry(handle.into())
                .or_default()
                .insert(argv[2].clone(), value.clone());
            Ok(value)
        }
        _ => unreachable!(),
    }
}
