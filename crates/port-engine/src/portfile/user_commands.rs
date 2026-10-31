//! PortSystem user commands from portutil and pextlib: file helpers
//! (`delete`, `copy`, `move`, `ln`, `touch`, `reinplace`, `xinstall`), list
//! helpers, digests, and host queries.
use super::*;
use molt::{MoltResult, check_args};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for (name, command) in [
        ("ldelete", ldelete as molt::CommandFunc),
        ("lpush", lpush),
        ("lpop", lpop),
        ("lshift", lshift),
        ("lunshift", lunshift),
        ("ldindex", ldindex),
        ("delete", delete),
        ("copy", copy),
        ("move", move_files),
        ("ln", ln),
        ("touch", touch),
        ("reinplace", reinplace),
        ("xinstall", xinstall),
        ("strsed", strsed),
        ("getuid", ids),
        ("geteuid", ids),
        ("getgid", ids),
        ("getegid", ids),
        ("name_to_uid", accounts),
        ("uid_to_name", accounts),
        ("name_to_gid", accounts),
        ("gid_to_name", accounts),
        ("sysctl", sysctl),
        ("md5", digest),
        ("sha1", digest),
        ("sha256", digest),
        ("rmd160", digest),
        ("set_ui_prefix", set_ui_prefix),
        ("dirSize", dir_size),
        ("backup", backup),
        ("mkdtemp", mkdtemp),
        ("msgcat::mc", message_catalog),
    ] {
        interp.add_context_command(name, command, id);
    }
}

fn call(interp: &mut Interp, words: Vec<Value>) -> MoltResult {
    interp.eval_value(&Value::from(words))
}

fn words(head: &[&str], rest: &[Value]) -> Vec<Value> {
    head.iter()
        .map(|word| Value::from(*word))
        .chain(rest.iter().cloned())
        .collect()
}

fn require_execution(interp: &mut Interp, id: ContextID, name: &str) -> Result<(), Exception> {
    if interp.context::<State>(id).system_executor.is_none() {
        interp.context::<State>(id).unsupported.insert(name.into());
        return Err(error(format!("unsupported Portfile command {name}")));
    }
    Ok(())
}

fn io_error(context: &str, err: std::io::Error) -> Exception {
    error(format!("{context}: {err}"))
}

// ---------------------------------------------------------------------------
// List helpers (portutil ldelete and macports_util)

fn ldelete(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "list value")?;
    let mut list = argv[1].as_list()?.to_vec();
    if let Some(index) = list
        .iter()
        .position(|item| item.as_str() == argv[2].as_str())
    {
        list.remove(index);
    }
    Ok(Value::from(list))
}

fn update_list(
    interp: &mut Interp,
    name: &Value,
    change: impl FnOnce(&mut Vec<Value>) -> Value,
) -> MoltResult {
    let mut list = if interp.var_exists(name) {
        interp.var(name)?.as_list()?.to_vec()
    } else {
        Vec::new()
    };
    let result = change(&mut list);
    interp.set_var(name, Value::from(list))?;
    Ok(result)
}

fn lpush(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "varName ?value ...?")?;
    update_list(interp, &argv[1], |list| {
        list.extend_from_slice(&argv[2..]);
        Value::from(list.clone())
    })
}

fn lunshift(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "varName ?value ...?")?;
    update_list(interp, &argv[1], |list| {
        for value in argv[2..].iter().rev() {
            list.insert(0, value.clone());
        }
        Value::from(list.clone())
    })
}

fn lpop(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "varName")?;
    update_list(interp, &argv[1], |list| {
        list.pop().unwrap_or_else(Value::empty)
    })
}

fn lshift(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "varName")?;
    update_list(interp, &argv[1], |list| {
        if list.is_empty() {
            Value::empty()
        } else {
            list.remove(0)
        }
    })
}

fn ldindex(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "varName ?index ...?")?;
    let mut indices = Vec::new();
    for index in &argv[2..] {
        indices.push(index.as_int()?);
    }
    update_list(interp, &argv[1], |list| {
        let Some(&first) = indices.first() else {
            let all = Value::from(list.clone());
            list.clear();
            return all;
        };
        let length = list.len() as i64;
        let position = if first < 0 { length + first } else { first };
        if position < 0 || position >= length {
            return Value::empty();
        }
        list.remove(position as usize)
    })
}

// ---------------------------------------------------------------------------
// File helpers

fn delete(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    call(
        interp,
        words(&["file", "delete", "-force", "--"], &argv[1..]),
    )
}

fn copy(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    call(interp, words(&["file", "copy"], &argv[1..]))
}

fn move_files(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    let mut force = false;
    let mut position = 1;
    while let Some(arg) = argv.get(position) {
        match arg.as_str() {
            "-force" => force = true,
            "--" => {
                position += 1;
                break;
            }
            option if option.starts_with('-') => {
                return Err(error(format!("move: illegal option -- {}", &option[1..])));
            }
            _ => break,
        }
        position += 1;
    }
    let args = &argv[position..];
    let mut options = vec!["file", "rename"];
    if force {
        options.push("-force");
    }
    options.push("--");
    if let [old, new] = args
        && old.as_str() != new.as_str()
        && old.as_str().eq_ignore_ascii_case(new.as_str())
    {
        // Case-only renames go through a temporary name, as upstream does.
        require_execution(interp, id, "move")?;
        let temporary = format!("{}-portrs-case-rename", old.as_str());
        call(
            interp,
            words(&options, &[old.clone(), temporary.as_str().into()]),
        )?;
        return call(interp, words(&options, &[temporary.into(), new.clone()]));
    }
    call(interp, words(&options, args))
}

fn ln(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    require_execution(interp, id, "ln")?;
    let mut args: Vec<String> = argv[1..].iter().map(Value::to_string).collect();
    let (mut force, mut no_follow, mut soft) = (false, false, false);
    while let Some(first) = args.first().cloned() {
        let Some(flags) = first.strip_prefix('-') else {
            break;
        };
        args.remove(0);
        if flags == "-" {
            break;
        }
        for flag in flags.chars() {
            match flag {
                'f' => force = true,
                'h' => no_follow = true,
                's' => soft = true,
                'v' => {}
                other => return Err(error(format!("ln: illegal option -- {other}"))),
            }
        }
    }
    let (files, target) = match args.as_slice() {
        [] => {
            return Err(error(
                "usage: ln [-f] [-h] [-s] [-v] source_file [target_file]\n       ln [-f] [-h] [-s] [-v] file ... directory",
            ));
        }
        [file] => (vec![file.clone()], "./".to_string()),
        [files @ .., target] => (files.to_vec(), target.clone()),
    };
    let target_path = Path::new(&target);
    let target_is_dir = target_path.is_dir();
    let append = target_is_dir
        && !(no_follow
            && fs::symlink_metadata(target_path).is_ok_and(|m| m.file_type().is_symlink()));
    for file in files {
        if Path::new(&file).is_dir() && !soft {
            return Err(error(format!("ln: {file}: Is a directory")));
        }
        let link = if append {
            target_path.join(Path::new(&file).file_name().unwrap_or_default())
        } else {
            PathBuf::from(&target)
        };
        if fs::symlink_metadata(&link).is_ok() {
            if force {
                fs::remove_file(&link)
                    .map_err(|err| io_error(&format!("ln: {}", link.display()), err))?;
            } else {
                return Err(error(format!("ln: {}: File exists", link.display())));
            }
        }
        let result = if soft {
            symlink(&file, &link)
        } else {
            fs::hard_link(&file, &link)
        };
        result.map_err(|err| io_error(&format!("ln: {}", link.display()), err))?;
    }
    Ok(Value::empty())
}

fn worksrc(interp: &mut Interp) -> Result<String, Exception> {
    interp.with_global_scope(|interp| Ok(interp.scalar("worksrcpath")?.to_string()))
}

fn touch(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    require_execution(interp, id, "touch")?;
    let mut directory = worksrc(interp)?;
    let mut args: Vec<String> = argv[1..].iter().map(Value::to_string).collect();
    let (mut access_only, mut no_create, mut modify_only) = (false, false, false);
    let mut reference = None;
    while let Some(first) = args.first().cloned() {
        let Some(flag) = first.strip_prefix('-') else {
            break;
        };
        args.remove(0);
        match flag {
            "a" => access_only = true,
            "c" => no_create = true,
            "m" => modify_only = true,
            "r" => {
                if args.is_empty() {
                    return Err(error("touch: option requires an argument -- r"));
                }
                reference = Some(args.remove(0));
            }
            "W" => {
                if !args.is_empty() {
                    directory = args.remove(0);
                }
            }
            "t" => return Err(error("touch -t is not supported")),
            "-" => break,
            other => return Err(error(format!("touch: illegal option -- {other}"))),
        }
    }
    if args.is_empty() {
        return Err(error(
            "usage: touch [-a] [-c] [-m] [-r file] [-t [[CC]YY]MMDDhhmm[.SS]] [-W dir] file ...",
        ));
    }
    let times = match &reference {
        Some(path) => {
            let metadata = fs::metadata(path)
                .map_err(|_| error(format!("touch: {path}: No such file or directory")))?;
            (metadata.accessed().ok(), metadata.modified().ok())
        }
        None => {
            let now = std::time::SystemTime::now();
            (Some(now), Some(now))
        }
    };
    for file in args {
        let path = Path::new(&directory).join(&file);
        if !path.exists() {
            if no_create {
                continue;
            }
            fs::File::create(&path)
                .map_err(|err| io_error(&format!("touch: {}", path.display()), err))?;
        }
        let handle = fs::File::options()
            .write(true)
            .open(&path)
            .or_else(|_| fs::File::open(&path))
            .map_err(|err| io_error(&format!("touch: {}", path.display()), err))?;
        let mut file_times = fs::FileTimes::new();
        if (access_only || !modify_only)
            && let Some(time) = times.0
        {
            file_times = file_times.set_accessed(time);
        }
        if (modify_only || !access_only)
            && let Some(time) = times.1
        {
            file_times = file_times.set_modified(time);
        }
        handle
            .set_times(file_times)
            .map_err(|err| io_error(&format!("touch: {}", path.display()), err))?;
    }
    Ok(Value::empty())
}

fn environment(interp: &mut Interp) -> Vec<(String, String)> {
    interp
        .with_global_scope(|interp| Ok::<_, Exception>(interp.array_get("env")))
        .unwrap_or_default()
        .chunks(2)
        .filter_map(|pair| match pair {
            [key, value] => Some((key.to_string(), value.to_string())),
            _ => None,
        })
        .collect()
}

fn reinplace(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    require_execution(interp, id, "reinplace")?;
    let mut args: Vec<String> = argv[1..].iter().map(Value::to_string).collect();
    let (mut extended, mut suppress, mut quiet) = (false, false, false);
    let mut locale = None;
    let mut directory = worksrc(interp)?;
    while let Some(first) = args.first().cloned() {
        let Some(flag) = first.strip_prefix('-') else {
            break;
        };
        args.remove(0);
        match flag {
            "locale" => {
                if !args.is_empty() {
                    locale = Some(args.remove(0));
                }
            }
            "E" => extended = true,
            "n" => suppress = true,
            "q" => quiet = true,
            "W" => {
                if !args.is_empty() {
                    directory = args.remove(0);
                }
            }
            "-" => break,
            _ => return Err(error(format!("reinplace: unknown flag '{first}'"))),
        }
    }
    if args.len() < 2 {
        return Err(error("reinplace ?-E? ?-n? ?-q? ?-W dir? pattern file ..."));
    }
    let pattern = args.remove(0);
    let mut env = environment(interp);
    if let Some(locale) = &locale {
        env.retain(|(key, _)| key != "LC_CTYPE");
        env.push(("LC_CTYPE".into(), locale.clone()));
    }
    let progress = interp
        .context::<State>(id)
        .progress
        .clone()
        .unwrap_or_else(|| crate::progress::Progress::new(false));
    for file in args {
        let path = Path::new(&directory).join(&file);
        let input = fs::read(&path).map_err(|err| {
            error(format!(
                "reinplace: couldn't open \"{}\": {err}",
                path.display()
            ))
        })?;
        let mut command = Command::new("/usr/bin/sed");
        if extended {
            command.arg("-E");
        }
        if suppress {
            command.arg("-n");
        }
        let output = command
            .arg(&pattern)
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(fs::File::open(&path).map_err(|err| io_error("reinplace", err))?)
            .output()
            .map_err(|err| io_error("reinplace", err))?;
        if !output.status.success() {
            progress.message(&format!(
                "Error: reinplace: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
            return Err(error("reinplace sed(1) failed"));
        }
        if !quiet && output.stdout == input {
            progress.warning(&format!(
                "reinplace {pattern} didn't change anything in {}",
                path.display()
            ));
        }
        let permissions = fs::metadata(&path)
            .map_err(|err| io_error("reinplace", err))?
            .permissions();
        let mut writable = permissions.clone();
        writable.set_mode(permissions.mode() | 0o200);
        fs::set_permissions(&path, writable).map_err(|_| error("reinplace permissions failed"))?;
        fs::write(&path, &output.stdout).map_err(|_| error("reinplace copy failed"))?;
        fs::set_permissions(&path, permissions)
            .map_err(|_| error("reinplace permissions failed"))?;
    }
    Ok(Value::empty())
}

fn parse_mode(text: &str, current: u32) -> Result<u32, Exception> {
    if let Ok(mode) = u32::from_str_radix(text, 8) {
        return Ok(mode);
    }
    // Symbolic modes such as u+w,go-x.
    let mut mode = current;
    for clause in text.split(',') {
        let position = clause
            .find(['+', '-', '='])
            .ok_or_else(|| error(format!("invalid file mode \"{text}\"")))?;
        let (who, rest) = clause.split_at(position);
        let operator = rest.chars().next().unwrap();
        let mut mask = 0;
        for class in if who.is_empty() {
            "a".chars()
        } else {
            who.chars()
        } {
            mask |= match class {
                'u' => 0o4700,
                'g' => 0o2070,
                'o' => 0o1007,
                'a' => 0o7777,
                _ => return Err(error(format!("invalid file mode \"{text}\""))),
            };
        }
        let mut bits = 0;
        for permission in rest[1..].chars() {
            bits |= match permission {
                'r' => 0o444,
                'w' => 0o222,
                'x' => 0o111,
                's' => 0o6000,
                't' => 0o1000,
                _ => return Err(error(format!("invalid file mode \"{text}\""))),
            };
        }
        bits &= mask;
        match operator {
            '+' => mode |= bits,
            '-' => mode &= !bits,
            _ => mode = (mode & !mask) | bits,
        }
    }
    Ok(mode)
}

fn xinstall(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    require_execution(interp, id, "xinstall")?;
    let mut args: Vec<String> = argv[1..].iter().map(Value::to_string).collect();
    let mut directories = false;
    let mut mode: Option<String> = None;
    let mut working = None;
    let mut preserve = false;
    while let Some(first) = args.first().cloned() {
        if !first.starts_with('-') || first == "-" {
            break;
        }
        args.remove(0);
        if first == "--" {
            break;
        }
        let mut flags = first[1..].chars().peekable();
        while let Some(flag) = flags.next() {
            let value = |args: &mut Vec<String>,
                         flags: &mut std::iter::Peekable<std::str::Chars>| {
                let inline: String = flags.by_ref().collect();
                if !inline.is_empty() {
                    Some(inline)
                } else if !args.is_empty() {
                    Some(args.remove(0))
                } else {
                    None
                }
            };
            match flag {
                'd' => directories = true,
                'c' | 'C' | 's' | 'S' | 'M' | 'b' => {}
                'p' => preserve = true,
                'm' => mode = value(&mut args, &mut flags),
                'W' => working = value(&mut args, &mut flags),
                'o' | 'g' | 'B' | 'f' => {
                    value(&mut args, &mut flags);
                }
                other => return Err(error(format!("xinstall: illegal option -- {other}"))),
            }
        }
    }
    let resolve = |path: &str| match &working {
        Some(dir) if !path.starts_with('/') => Path::new(dir).join(path),
        _ => PathBuf::from(path),
    };
    // As in BSD install(1), symbolic modes apply to an empty mode.
    let apply_mode = |path: &Path, default: u32| -> Result<(), Exception> {
        let mode = match &mode {
            Some(text) => parse_mode(text, 0)?,
            None => default,
        };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .map_err(|err| io_error(&format!("xinstall: {}", path.display()), err))
    };
    if directories {
        if args.is_empty() {
            return Err(error("xinstall: no directories specified"));
        }
        for directory in &args {
            let path = PathBuf::from(directory);
            fs::create_dir_all(&path)
                .map_err(|err| io_error(&format!("xinstall: {directory}"), err))?;
            apply_mode(&path, 0o755)?;
        }
        return Ok(Value::empty());
    }
    if args.len() < 2 {
        return Err(error(
            "usage: xinstall [-bCcpSs] [-B suffix] [-f flags] [-g group] [-m mode] [-o owner] file1 file2",
        ));
    }
    let target = PathBuf::from(args.pop().unwrap());
    let into_directory = target.is_dir();
    if args.len() > 1 && !into_directory {
        return Err(error(format!(
            "xinstall: {}: Not a directory",
            target.display()
        )));
    }
    for source in &args {
        let from = resolve(source);
        let to = if into_directory {
            target.join(from.file_name().unwrap_or_default())
        } else {
            target.clone()
        };
        if fs::symlink_metadata(&to).is_ok_and(|m| !m.is_dir()) {
            fs::remove_file(&to)
                .map_err(|err| io_error(&format!("xinstall: {}", to.display()), err))?;
        }
        fs::copy(&from, &to)
            .map_err(|err| io_error(&format!("xinstall: {}", from.display()), err))?;
        apply_mode(&to, 0o755)?;
        if preserve
            && let Ok(modified) = fs::metadata(&from).and_then(|m| m.modified())
            && let Ok(handle) = fs::File::options().write(true).open(&to)
        {
            let _ = handle.set_modified(modified);
        }
    }
    Ok(Value::empty())
}

/// pextlib `strsed string s/pattern/replacement/[g]`.
fn strsed(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "string sed-expression")?;
    let text = argv[1].as_str();
    let expression = argv[2].as_str();
    let mut characters = expression.chars().peekable();
    // `[g]/a/b/` as Pextlib's strsed: a leading `g` substitutes globally and
    // the next character delimits; `s/a/b/[g]` is also accepted.
    let leading_global = match characters.peek() {
        Some('g') => {
            characters.next();
            true
        }
        Some('s') => {
            characters.next();
            false
        }
        _ => false,
    };
    let delimiter = characters
        .next()
        .ok_or_else(|| error("strsed: bad expression"))?;
    let mut parts = vec![String::new()];
    let mut escaped = false;
    for character in characters {
        if escaped {
            if character != delimiter {
                parts.last_mut().unwrap().push('\\');
            }
            parts.last_mut().unwrap().push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == delimiter {
            parts.push(String::new());
        } else {
            parts.last_mut().unwrap().push(character);
        }
    }
    if parts.len() < 3 {
        return Err(error("strsed: bad expression"));
    }
    let global = leading_global || parts[2].contains('g');
    // Basic regular expressions: \( \) groups and literal + ? | { }.
    let mut pattern = String::new();
    let mut chars = parts[0].chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('(') => pattern.push('('),
                Some(')') => pattern.push(')'),
                Some('{') => pattern.push('{'),
                Some('}') => pattern.push('}'),
                Some(other) => {
                    pattern.push('\\');
                    pattern.push(other);
                }
                None => pattern.push_str("\\\\"),
            },
            '(' | ')' | '+' | '?' | '|' | '{' | '}' => {
                pattern.push('\\');
                pattern.push(c);
            }
            other => pattern.push(other),
        }
    }
    let regex = regex::Regex::new(&pattern).map_err(|err| error(format!("strsed: {err}")))?;
    let replacement = &parts[1];
    let expand = |captures: &regex::Captures| {
        let mut out = String::new();
        let mut chars = replacement.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '&' => out.push_str(&captures[0]),
                '\\' => match chars.next() {
                    Some(d) if d.is_ascii_digit() => out.push_str(
                        captures
                            .get(d.to_digit(10).unwrap() as usize)
                            .map_or("", |m| m.as_str()),
                    ),
                    Some(other) => out.push(other),
                    None => out.push('\\'),
                },
                other => out.push(other),
            }
        }
        out
    };
    let result = if global {
        regex.replace_all(text, |c: &regex::Captures| expand(c))
    } else {
        regex.replace(text, |c: &regex::Captures| expand(c))
    };
    Ok(result.into_owned().into())
}

// ---------------------------------------------------------------------------
// Host queries

fn ids(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    // SAFETY: these libc calls take no arguments and only read process credentials.
    let id = unsafe {
        match argv[0].as_str() {
            "getuid" => libc::getuid(),
            "geteuid" => libc::geteuid(),
            "getgid" => libc::getgid(),
            _ => libc::getegid(),
        }
    };
    Ok(i64::from(id).into())
}

fn accounts(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "name")?;
    let query = argv[1].as_str();
    let group = argv[0].as_str().contains("gid");
    let result = match argv[0].as_str() {
        "name_to_uid" | "name_to_gid" => crate::accounts::by_name(query, group)
            .ok()
            .flatten()
            .map(|account| i64::from(account.id))
            .unwrap_or(-1)
            .to_string(),
        _ => query
            .parse::<u32>()
            .ok()
            .and_then(|id| crate::accounts::by_id(id, group).ok().flatten())
            .map(|account| account.name)
            .unwrap_or_default(),
    };
    Ok(result.into())
}

fn sysctl(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "name")?;
    let output = Command::new("/usr/sbin/sysctl")
        .args(["-n", argv[1].as_str()])
        .output()
        .map_err(|err| io_error("sysctl", err))?;
    if !output.status.success() {
        return Err(error(format!("sysctl failed for {}", argv[1])));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string()
        .into())
}

fn digest(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "file path")?;
    if argv[1].as_str() != "file" {
        return Err(error(format!("{} supports only \"file\"", argv[0])));
    }
    let kind: crate::install::source::checksum::Kind = argv[0]
        .as_str()
        .parse()
        .map_err(|err: anyhow::Error| error(err.to_string()))?;
    let path = Path::new(argv[2].as_str());
    let values = crate::install::source::checksum::calculate(path, &[kind])
        .map_err(|err| error(format!("{err:#}")))?;
    Ok(values.into_iter().next().unwrap_or_default().into())
}

fn set_ui_prefix(interp: &mut Interp, _: ContextID, _: &[Value]) -> MoltResult {
    interp.with_global_scope(|interp| {
        if !interp.var_exists(&"UI_PREFIX".into()) {
            interp.set_scalar("UI_PREFIX", "---> ".into())?;
        }
        Ok(Value::empty())
    })
}

fn size_of(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_dir() {
        fs::read_dir(path)
            .map(|entries| entries.flatten().map(|entry| size_of(&entry.path())).sum())
            .unwrap_or(0)
    } else {
        metadata.len()
    }
}

fn dir_size(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "dir")?;
    Ok((size_of(Path::new(argv[1].as_str())) as i64).into())
}

fn backup(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "file")?;
    require_execution(interp, id, "backup")?;
    let file = argv[1].as_str();
    let mut target = format!("{file}.mpsaved");
    let mut counter = 1;
    while Path::new(&target).exists() {
        target = format!("{file}.mpsaved{counter}");
        counter += 1;
    }
    fs::rename(file, &target).map_err(|err| io_error(&format!("backup {file}"), err))?;
    Ok(target.into())
}

fn mkdtemp(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "template")?;
    require_execution(interp, id, "mkdtemp")?;
    let template = argv[1].as_str();
    let prefix = template.trim_end_matches('X');
    let path = Path::new(prefix);
    let directory = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let created = tempfile::Builder::new()
        .prefix(&name)
        .tempdir_in(directory)
        .map_err(|err| io_error("mkdtemp", err))?
        .keep();
    Ok(created.to_string_lossy().into_owned().into())
}

/// `msgcat::mc src ?arg ...?` with no translations: format the source string.
fn message_catalog(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 0, "src ?arg ...?")?;
    if argv.len() == 2 {
        return Ok(argv[1].clone());
    }
    call(interp, words(&["format"], &argv[1..]))
}
