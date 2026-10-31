//! Native generation and staging of launchd property lists.
use super::*;
mod defaults;
mod plist;
pub(super) use defaults::initialize;
mod wrapper;
use anyhow::{Context, bail};
use molt::MoltResult;
use std::{
    fs, io,
    os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
    path::{Path, PathBuf},
};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portstartupitem::startupitem_create", create, id);
    interp.add_context_command("eval_targets", eval_targets, id);
    defaults::install(interp, id);
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
        "startupitem.executable",
        "startupitem.init",
        "startupitem.start",
        "startupitem.stop",
        "startupitem.restart",
        "startupitem.user",
        "startupitem.group",
        "startupitem.logfile",
        "startupitem.logfile.stderr",
        "startupitem.netchange",
        "startupitem.logevents",
        "startupitem.daemondo.verbosity",
        "startupitem.pidfile",
        "startupitem.debug",
        "startupitem.autostart",
        "startupitem.requires",
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
enum Source {
    Custom(PathBuf),
    Generated(plist::Generated),
}
struct Item {
    source: Source,
    unique: String,
    plist: String,
    location: String,
    name: String,
    autostart: bool,
    install: bool,
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
fn plan(interp: &mut Interp) -> Result<Vec<Item>> {
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
        if let Some(item) = plan_item(interp, &attrs)? {
            items.push(item);
        }
    }
    Ok(items)
}
fn plan_item(interp: &mut Interp, attrs: &Attributes) -> Result<Option<Item>> {
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
    let source = if custom.is_empty() {
        let generated = plist::generate(interp, attrs, &name, &unique, &location)?;
        if let Some((filename, _)) = &generated.wrapper {
            ensure!(
                filename != &plist,
                "startup wrapper and plist filenames must differ"
            );
        }
        Source::Generated(generated)
    } else {
        let source = PathBuf::from(custom);
        ensure!(
            fs::symlink_metadata(&source)
                .with_context(|| format!("cannot read custom plist {}", source.display()))?
                .is_file(),
            "custom plist must be a regular file"
        );
        Source::Custom(source)
    };
    let flag = |interp: &mut Interp, key: &str, default: &str| -> Result<bool> {
        Value::from(attr_text(interp, attrs, key, default)?)
            .as_bool()
            .map_err(failure)
    };
    let autostart = flag(interp, "autostart", "no")?;
    // Only load/unload need the install location; staging-only sessions may
    // lack the configuration it defaults from, so use upstream's default then.
    let install = flag(interp, "install", "yes").unwrap_or(true);
    Ok(Some(Item {
        source,
        unique,
        plist,
        location,
        name,
        autostart,
        install,
    }))
}
/// `eval_targets` for the launchd targets activation runs (`load`, `unload`).
/// Other targets are run by the host's phase driver, not from Portfiles.
fn eval_targets(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    molt::check_args(1, argv, 2, 2, "target")?;
    let target = argv[1].as_str();
    if !matches!(target, "load" | "unload") {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        let items = plan(interp).map_err(|e| error(e.to_string()))?;
        let autostart_only = interp
            .scalar("portstartupitem::autostart_only")
            .ok()
            .and_then(|value| value.as_bool().ok())
            .unwrap_or(false);
        let prefix = interp.scalar("prefix")?.to_string();
        let subport = interp.scalar("subport")?.to_string();
        let progress = interp
            .context::<State>(id)
            .progress
            .clone()
            .unwrap_or_else(|| crate::progress::Progress::new(false));
        // SAFETY: getuid only reads the process credentials.
        let root = unsafe { libc::getuid() } == 0;
        for item in items {
            if target == "load" && autostart_only && !item.autostart {
                continue;
            }
            let path = if item.install {
                format!("/Library/{}/{}", item.location, item.plist)
            } else {
                format!("{prefix}/etc/{}/{}", item.location, item.plist)
            };
            if target == "load" && !Path::new(&path).exists() {
                return Err(error(format!("Launchd plist {path} was not found")));
            }
            if item.location == "LaunchDaemons" && !root {
                progress.warning(&format!(
                    "Skipping {target} of startupitem '{}' for {subport}, root privileges required",
                    item.name
                ));
                continue;
            }
            let verb = if target == "load" {
                "Loading"
            } else {
                "Unloading"
            };
            progress.message(&format!(
                "--->  {verb} startupitem '{}' for {subport}",
                item.name
            ));
            interp.eval_value(&Value::from(vec![
                Value::from("system"),
                Value::from(format!("/bin/launchctl {target} -w {path}")),
            ]))?;
        }
        Ok(0.into())
    })
}

pub(super) fn preflight(interp: &mut Interp, _id: ContextID) -> MoltResult {
    plan(interp)
        .map(|_| Value::empty())
        .map_err(|e| error(e.to_string()))
}
fn create(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        let result = (|| -> Result<()> {
            let items = plan(interp)?;
            if items.is_empty() {
                return Ok(());
            }
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
                    Ok(metadata)
                        if matches!(item.source, Source::Generated(_)) && metadata.is_file() => {}
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
                match &item.source {
                    Source::Custom(source) => copy(source, &destination)?,
                    Source::Generated(generated) => {
                        if let Some((filename, script)) = &generated.wrapper {
                            write_generated(&directory.join(filename), script, 0o755 & !mask)?;
                        }
                        write_generated(&destination, &generated.xml, 0o644 & !mask)?;
                        if generated.keep_run {
                            let directory = format!("{}{}/var/run", root.display(), prefix);
                            let value = interp.scalar("destroot.keepdirs").map_err(failure)?;
                            let mut keeps = value.as_list().map_err(failure)?.to_vec();
                            if !keeps.iter().any(|value| value.as_str() == directory) {
                                keeps.push(directory.into());
                                interp
                                    .set_scalar("destroot.keepdirs", Value::from(keeps))
                                    .map_err(failure)?;
                            }
                        }
                        for warning in &generated.warnings {
                            interp
                                .context::<State>(id)
                                .progress
                                .clone()
                                .unwrap_or_else(|| crate::progress::Progress::new(false))
                                .message(&format!("Error: {warning}"));
                        }
                    }
                }
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

fn write_generated(destination: &Path, xml: &str, mode: u32) -> Result<()> {
    use std::io::Write;
    match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            ensure!(metadata.is_file(), "generated plist must be a regular file");
            let mut output = fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(destination)?;
            ensure!(
                output.metadata()?.is_file(),
                "generated plist must be a regular file"
            );
            // Match open-with-truncation semantics: retain ownership, mode,
            // and hard links, and respect write permission on existing files.
            output.set_len(0)?;
            output.write_all(xml.as_bytes())?;
            return Ok(());
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => (),
        Err(err) => return Err(err.into()),
    }
    let mut temporary = tempfile::NamedTempFile::new_in(destination.parent().unwrap())?;
    temporary.write_all(xml.as_bytes())?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(mode))?;
    temporary.persist_noclobber(destination)?;
    Ok(())
}
