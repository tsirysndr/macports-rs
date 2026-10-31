//! Native PortSystem extract target procedures.
use super::*;
use molt::{MoltResult, check_args};

const FORMATS: [&str; 8] = [
    "use_bzip2",
    "use_lzma",
    "use_tar",
    "use_xz",
    "use_zip",
    "use_7z",
    "use_lzip",
    "use_dmg",
];

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portextract::disttagclean", clean, id);
    interp.add_context_command("portextract::get_extract_cmd", cmd, id);
    interp.add_context_command("portextract::get_extract_pre_args", pre_args, id);
    interp.add_context_command("portextract::get_extract_post_args", post_args, id);
    interp.add_context_command("portextract::extract_start", start, id);
    interp.add_context_command("portextract::extract_main", main, id);
    for name in [
        "extract.only",
        "extract.mkdir",
        "extract.rename",
        "extract.asroot",
    ] {
        commands::register(interp, id, name);
    }
}

fn call(interp: &mut Interp, words: &[&str]) -> MoltResult {
    interp.eval_value(&Value::from(
        words
            .iter()
            .map(|word| Value::from(*word))
            .collect::<Vec<_>>(),
    ))
}

fn enabled(interp: &mut Interp, name: &str) -> Result<bool, Exception> {
    Ok(interp.var_exists(&name.into()) && interp.scalar(name)?.as_bool().unwrap_or(false))
}

fn format(interp: &mut Interp) -> Result<Option<&'static str>, Exception> {
    for name in FORMATS {
        if enabled(interp, name)? {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

fn autoconf(interp: &mut Interp, name: &str) -> Result<String, Exception> {
    Ok(interp
        .scalar(&format!("portutil::autoconf::{name}"))?
        .to_string())
}

fn find(interp: &mut Interp, program: &str, hint: &str) -> MoltResult {
    let hint = autoconf(interp, hint)?;
    call(interp, &["findBinary", program, &hint])
}

fn clean(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "list")?;
    let mut names = Vec::new();
    for item in argv[1].as_list()?.iter() {
        names.push(call(interp, &["getdistname", item.as_str()])?);
    }
    Ok(Value::from(names))
}

fn cmd(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| match format(interp)? {
        Some("use_bzip2") => call(interp, &["macports::binaryInPath", "lbzip2"])
            .or_else(|_| find(interp, "bzip2", "bzip2_path")),
        Some("use_lzma") => find(interp, "lzma", "lzma_path"),
        Some("use_tar") => find(interp, "tar", "tar_command"),
        Some("use_xz") => find(interp, "xz", "xz_path"),
        Some("use_zip") => find(interp, "unzip", "unzip_path"),
        Some("use_7z") => call(interp, &["macports::binaryInPath", "7za"]),
        Some("use_lzip") => call(interp, &["macports::binaryInPath", "lzip"]),
        Some("use_dmg") => find(interp, "hdiutil", "hdiutil_path"),
        _ => find(interp, "gzip", "gzip_path"),
    })
}

fn pre_args(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        Ok(match format(interp)? {
            Some("use_tar") => "-xf",
            Some("use_zip") => "-q",
            Some("use_7z") => "x",
            Some("use_dmg") => "attach",
            _ => "-dc",
        }
        .into())
    })
}

fn escape(interp: &mut Interp, text: &str) -> Result<String, Exception> {
    Ok(call(interp, &["shellescape", text])?.to_string())
}

fn post_args(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        Ok(match format(interp)? {
            Some("use_tar" | "use_7z") => String::new(),
            Some("use_zip") => {
                let directory = interp.scalar("extract.dir")?;
                format!("-d {}", escape(interp, directory.as_str())?)
            }
            Some("use_dmg") => {
                let mount = interp
                    .context::<State>(id)
                    .dmg_mount
                    .clone()
                    .unwrap_or_else(|| "/tmp/mports.XXXXXXXX".into());
                let mount = escape(interp, &mount)?;
                let distname = interp.scalar("distname")?;
                let command = interp.scalar("extract.cmd")?;
                let directory = interp.scalar("extract.dir")?;
                let target = escape(interp, &format!("{directory}/{distname}"))?;
                let find = find(interp, "find", "find_path")?;
                let cpio = find_cpio(interp)?;
                let rmdir = find_rmdir(interp)?;
                format!(
                    "-private -readonly -nobrowse -mountpoint {mount} && cd {mount} && {find} . -depth -perm -+r -print0 | {cpio} -0 -p -d -m -u {target}; status=$?; cd / && {command} detach {mount} && {rmdir} {mount}; exit $status"
                )
            }
            _ => format!("| {} -xf -", autoconf(interp, "tar_command")?),
        }
        .into())
    })
}

fn find_cpio(interp: &mut Interp) -> MoltResult {
    find(interp, "cpio", "cpio_path")
}

fn find_rmdir(interp: &mut Interp) -> MoltResult {
    find(interp, "rmdir", "rmdir_path")
}

fn start(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    phase_start::notice(interp, id, argv, "Extracting")?;
    interp.with_global_scope(|interp| {
        call(interp, &["handle_add_users"])?;
        if enabled(interp, "extract.mkdir")? {
            let path = interp.scalar("worksrcpath")?;
            call(interp, &["file", "mkdir", path.as_str()])?;
            interp.set_scalar("extract.dir", path)?;
        }
        if enabled(interp, "use_dmg")? {
            let directory = tempfile::Builder::new()
                .prefix("mports.")
                .tempdir_in("/tmp")
                .map_err(|err| error(format!("mkdtemp failed: {err}")))?
                .keep();
            interp.context::<State>(id).dmg_mount = Some(directory.to_string_lossy().into_owned());
        }
        Ok(Value::empty())
    })
}

fn main(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        if !interp.var_exists(&"distfiles".into()) && !interp.var_exists(&"extract.only".into()) {
            return Ok(0.into());
        }
        let files = interp.scalar("filespath")?.to_string();
        let dmg = enabled(interp, "use_dmg")?;
        for distfile in interp.scalar("extract.only")?.as_list()?.iter() {
            let local = format!("{files}/{distfile}");
            let path = if filesystem::predicate("exists", &local) {
                local
            } else {
                format!("{}/{distfile}", interp.scalar("distpath")?)
            };
            interp.set_scalar("extract.args", format!("'{path}'").into())?;
            if dmg {
                call(interp, &["elevateToRoot", "extract dmg"])?;
            }
            let result = call(interp, &["command_exec", "extract"]);
            if dmg {
                call(interp, &["dropPrivileges"])?;
            }
            result?;
            let directory = interp.scalar("extract.dir")?;
            call(interp, &["chownAsRoot", directory.as_str()])?;
        }
        let worksrc = interp.scalar("worksrcpath")?;
        if enabled(interp, "extract.rename")? && !filesystem::predicate("exists", worksrc.as_str())
        {
            let work = interp.scalar("workpath")?.to_string();
            let distname = interp.scalar("distname")?.to_string();
            let mut directories = Vec::new();
            for entry in std::fs::read_dir(&work)
                .map_err(|err| filesystem::path_error(&work, "cannot read", err))?
                .flatten()
            {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with('.') && entry.path().is_dir() {
                    directories.push(entry.path());
                }
            }
            match directories.as_slice() {
                [only] => {
                    let target = std::path::Path::new(&work).join(&distname);
                    if *only != target {
                        std::fs::rename(only, &target).map_err(|err| {
                            filesystem::path_error(&target.to_string_lossy(), "cannot rename", err)
                        })?;
                    }
                }
                [] => {
                    return Err(error(format!(
                        "extract.rename: no directories exist in {work}"
                    )));
                }
                many => {
                    let names = many
                        .iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect::<Vec<_>>();
                    return Err(error(format!(
                        "extract.rename: multiple directories exist in {work}: {}",
                        names.join(" ")
                    )));
                }
            }
        }
        Ok(0.into())
    })
}
