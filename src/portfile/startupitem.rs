//! Native staging of port-supplied launchd property lists.
use super::*;
use anyhow::{Context, bail};
use molt::MoltResult;
use std::{
    fs, io,
    os::unix::fs::{OpenOptionsExt, symlink},
    path::{Path, PathBuf},
};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portstartupitem::startupitem_create", create, id);
    for key in [
        "startupitems",
        "startupitem.create",
        "startupitem.custom_file",
        "startupitem.type",
        "startupitem.name",
        "startupitem.uniquename",
        "startupitem.plist",
        "startupitem.location",
        "startupitem.install",
    ] {
        commands::register(interp, id, key);
    }
}
fn optional(interp: &mut Interp, key: &str) -> Result<Option<Value>> {
    if interp.var_exists(&key.into()) {
        interp.scalar(key).map(Some).map_err(failure)
    } else {
        Ok(None)
    }
}
fn text(interp: &mut Interp, key: &str, default: &str) -> Result<String> {
    Ok(optional(interp, key)?.map_or_else(|| default.into(), |v| v.to_string()))
}
fn component(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0']),
        "invalid startup item path component {name:?}"
    );
    Ok(())
}
struct Custom {
    source: PathBuf,
    unique: String,
    plist: String,
    location: String,
}
type Attributes = BTreeMap<String, Value>;
fn attribute(interp: &mut Interp, attrs: &Attributes, key: &str) -> Result<Option<Value>> {
    match attrs.get(key) {
        Some(value) => Ok(Some(value.clone())),
        None => optional(interp, &format!("startupitem.{key}")),
    }
}
fn attr_text(interp: &mut Interp, attrs: &Attributes, key: &str, default: &str) -> Result<String> {
    Ok(attribute(interp, attrs, key)?.map_or_else(|| default.into(), |value| value.to_string()))
}
fn plan(interp: &mut Interp, id: ContextID) -> Result<Vec<Custom>> {
    let raw = text(interp, "startupitems", "")?;
    let groups = if raw.is_empty() {
        vec![Attributes::new()]
    } else {
        let words = Value::from(raw).as_list().map_err(failure)?.to_vec();
        let mut groups: Vec<(String, Attributes)> = Vec::new();
        let mut current = None;
        for pair in words.chunks(2) {
            let key = pair[0].as_str();
            let value = pair.get(1).cloned().unwrap_or_else(Value::empty);
            if key == "name" {
                let name = value.to_string();
                let index = groups
                    .iter()
                    .position(|(old, _)| old == &name)
                    .unwrap_or_else(|| {
                        groups.push((name.clone(), Attributes::new()));
                        groups.len() - 1
                    });
                current = Some(index);
                let unique = format!("org.macports.{name}");
                groups[index]
                    .1
                    .insert("uniquename".into(), unique.clone().into());
                groups[index]
                    .1
                    .insert("plist".into(), format!("{unique}.plist").into());
            }
            let index = current.context("startupitems must begin with a name")?;
            groups[index].1.insert(key.into(), value);
        }
        groups.into_iter().map(|(_, attrs)| attrs).collect()
    };
    let mut items = Vec::new();
    for attrs in groups {
        if let Some(item) = plan_item(interp, id, &attrs)? {
            items.push(item);
        }
    }
    Ok(items)
}
fn plan_item(interp: &mut Interp, id: ContextID, attrs: &Attributes) -> Result<Option<Custom>> {
    let enabled = attribute(interp, attrs, "create")?
        .map(|v| v.as_bool().map_err(failure))
        .transpose()?
        .unwrap_or(false);
    let custom = attr_text(interp, attrs, "custom_file", "")?;
    let kind = attribute(interp, attrs, "type")?;
    if kind.as_ref().is_some_and(|v| v.as_str() == "none") || (!enabled && custom.is_empty()) {
        return Ok(None);
    }
    // Upstream's default type is none unless startupitem.create is enabled.
    if kind.is_none() {
        let global_enabled = optional(interp, "startupitem.create")?
            .map(|value| value.as_bool().map_err(failure))
            .transpose()?
            .unwrap_or(false);
        if !global_enabled {
            return Ok(None);
        }
    }
    if custom.is_empty() {
        return unsupported(interp, id);
    }
    let kind = match kind {
        Some(kind) => kind.to_string(),
        None => {
            let configured = interp
                .element("system_options", "startupitem_type")
                .map_err(failure)?
                .to_string();
            if configured.is_empty() || configured == "default" {
                if interp.scalar("os.platform").map_err(failure)?.as_str() == "darwin" {
                    "launchd".into()
                } else {
                    "none".into()
                }
            } else {
                configured
            }
        }
    };
    if kind == "none" {
        return Ok(None);
    }
    ensure!(kind == "launchd", "unsupported startup item type {kind}");
    let subport = interp.scalar("subport").map_err(failure)?.to_string();
    let name = attr_text(interp, attrs, "name", &subport)?;
    let unique = attr_text(interp, attrs, "uniquename", &format!("org.macports.{name}"))?;
    let plist = attr_text(interp, attrs, "plist", &format!("{unique}.plist"))?;
    let location = attr_text(interp, attrs, "location", "LaunchDaemons")?;
    component(&unique)?;
    component(&plist)?;
    ensure!(
        matches!(location.as_str(), "LaunchAgents" | "LaunchDaemons"),
        "unsupported launchd location {location}"
    );
    let source = PathBuf::from(custom);
    ensure!(
        fs::symlink_metadata(&source)
            .with_context(|| format!("cannot read custom plist {}", source.display()))?
            .is_file(),
        "custom plist must be a regular file"
    );
    Ok(Some(Custom {
        source,
        unique,
        plist,
        location,
    }))
}
fn unsupported<T>(interp: &mut Interp, id: ContextID) -> Result<T> {
    interp
        .context::<State>(id)
        .unsupported
        .insert("startup item generation".into());
    bail!("native startup item generation is not implemented")
}
pub(super) fn preflight(interp: &mut Interp, id: ContextID) -> MoltResult {
    plan(interp, id)
        .map(|_| Value::empty())
        .map_err(|e| error(e.to_string()))
}
fn create(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        let result = (|| -> Result<()> {
            let items = plan(interp, id)?;
            if items.is_empty() {
                return Ok(());
            }
            ensure!(
                unsafe { libc::getuid() != 0 && libc::geteuid() != 0 },
                "privileged startup item staging requires the privilege worker"
            );
            let root = PathBuf::from(interp.scalar("destroot").map_err(failure)?.to_string());
            ensure!(
                fs::symlink_metadata(&root)?.is_dir(),
                "destroot must be a physical directory"
            );
            let prefix = interp.scalar("prefix").map_err(failure)?.to_string();
            for item in items {
                let relative = format!("{prefix}/etc/{}/{}", item.location, item.unique);
                let directory = destroot_mtree::stage_path(&root, &relative)?;
                let destination = directory.join(&item.plist);
                // file copy without -force must not overwrite existing entries.
                match fs::symlink_metadata(&destination) {
                    Ok(_) => bail!(
                        "custom plist destination already exists: {}",
                        destination.display()
                    ),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e.into()),
                }
                let mask = interp
                    .context::<State>(id)
                    .creation_mask
                    .context("creation mask is not initialized")?;
                crate::build::directories::create_all(&directory, Some(mask))?;
                copy(&item.source, &destination)?;
                let link = directory.parent().unwrap().join(&item.plist);
                let target = format!("{relative}/{}", item.plist);
                // Non-root MacPorts places the link in prefix/etc, even when the
                // install option is true. Loading services is a separate command.
                replace_link(&link, Path::new(&target))?;
            }
            Ok(())
        })();
        result
            .map(|()| Value::empty())
            .map_err(|e| error(e.to_string()))
    })
}
fn copy(source: &Path, destination: &Path) -> Result<()> {
    let mut source = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(source)?;
    let metadata = source.metadata()?;
    ensure!(metadata.is_file(), "custom plist must be a regular file");
    let mut output = tempfile::NamedTempFile::new_in(destination.parent().unwrap())?;
    io::copy(&mut source, &mut output)?;
    output.as_file().set_permissions(metadata.permissions())?;
    output.as_file().set_times(
        fs::FileTimes::new()
            .set_accessed(metadata.accessed()?)
            .set_modified(metadata.modified()?),
    )?;
    output.persist_noclobber(destination)?;
    Ok(())
}
fn replace_link(link: &Path, target: &Path) -> Result<()> {
    let temp = tempfile::tempdir_in(link.parent().unwrap())?;
    let replacement = temp.path().join("link");
    symlink(target, &replacement)?;
    fs::rename(&replacement, link)?;
    Ok(())
}
