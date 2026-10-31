//! Tcl `exec`: pipelines and redirections, using the session's `env` array.
use super::*;
use molt::MoltResult;
use std::{
    fs,
    io::{Read, Write},
    process::{Child, Command, Stdio},
};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("exec", exec, id);
}

#[derive(Default)]
struct Plan {
    stages: Vec<Vec<String>>,
    stdin_file: Option<String>,
    stdin_text: Option<String>,
    stdout_file: Option<(String, bool)>,
    stderr_file: Option<(String, bool)>,
    stderr_to_stdout: bool,
    stdout_to_stderr: bool,
    stderr_passthrough: bool,
    pipe_stderr: Vec<bool>,
}

fn parse(args: &[String]) -> Result<Plan, Exception> {
    let mut plan = Plan::default();
    let mut current = Vec::new();
    let mut index = 0;
    let target = |index: &mut usize, inline: &str| -> Result<String, Exception> {
        if !inline.is_empty() {
            return Ok(inline.to_string());
        }
        *index += 1;
        args.get(*index)
            .cloned()
            .ok_or_else(|| error("can't specify redirection without a file name"))
    };
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "|" | "|&" => {
                if current.is_empty() {
                    return Err(error("illegal use of | or |& in command"));
                }
                plan.pipe_stderr.push(arg == "|&");
                plan.stages.push(std::mem::take(&mut current));
            }
            "&" if index == args.len() - 1 => {
                return Err(error("exec in the background is not supported"));
            }
            "2>@1" => plan.stderr_to_stdout = true,
            ">@stderr" | ">@ stderr" => plan.stdout_to_stderr = true,
            "2>@stderr" => plan.stderr_passthrough = true,
            "2>@stdout" => plan.stderr_to_stdout = true,
            _ if arg.starts_with("<<") => plan.stdin_text = Some(target(&mut index, &arg[2..])?),
            _ if arg.starts_with("<@") => {
                return Err(error("exec channel redirection is not supported"));
            }
            _ if arg.starts_with('<') => plan.stdin_file = Some(target(&mut index, &arg[1..])?),
            _ if arg.starts_with("2>>") => {
                plan.stderr_file = Some((target(&mut index, &arg[3..])?, true))
            }
            _ if arg.starts_with("2>@") => {
                return Err(error("exec channel redirection is not supported"));
            }
            _ if arg.starts_with("2>") => {
                plan.stderr_file = Some((target(&mut index, &arg[2..])?, false))
            }
            _ if arg.starts_with(">&") => {
                let file = target(&mut index, &arg[2..])?;
                plan.stdout_file = Some((file, false));
                plan.stderr_to_stdout = true;
            }
            _ if arg.starts_with(">>") => {
                plan.stdout_file = Some((target(&mut index, &arg[2..])?, true))
            }
            _ if arg.starts_with(">@") => {
                return Err(error("exec channel redirection is not supported"));
            }
            _ if arg.starts_with('>') => {
                plan.stdout_file = Some((target(&mut index, &arg[1..])?, false))
            }
            _ => current.push(arg.to_string()),
        }
        index += 1;
    }
    if current.is_empty() {
        return Err(error("illegal use of | or |& in command"));
    }
    plan.stages.push(current);
    Ok(plan)
}

fn open_output(path: &str, append: bool) -> Result<fs::File, Exception> {
    fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)
        .map_err(|err| error(format!("couldn't write file \"{path}\": {err}")))
}

fn exec(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    let mut ignore_stderr = false;
    let mut keep_newline = false;
    let mut start = 1;
    while let Some(arg) = argv.get(start) {
        match arg.as_str() {
            "-ignorestderr" => ignore_stderr = true,
            "-keepnewline" => keep_newline = true,
            "--" => {
                start += 1;
                break;
            }
            _ => break,
        }
        start += 1;
    }
    let args: Vec<String> = argv[start..].iter().map(Value::to_string).collect();
    if args.is_empty() {
        return Err(error(
            "wrong # args: should be \"exec ?-option ...? arg ?arg ...?\"",
        ));
    }
    let plan = parse(&args)?;
    let environment: Vec<(String, String)> = interp
        .with_global_scope(|interp| Ok::<_, Exception>(interp.array_get("env")))?
        .chunks(2)
        .filter_map(|pair| match pair {
            [key, value] => Some((key.to_string(), value.to_string())),
            _ => None,
        })
        .collect();
    let stderr_capture = tempfile::tempfile().map_err(|err| error(format!("exec: {err}")))?;
    let mut children: Vec<Child> = Vec::new();
    let mut previous: Option<std::process::ChildStdout> = None;
    let last = plan.stages.len() - 1;
    for (position, stage) in plan.stages.iter().enumerate() {
        let mut command = Command::new(&stage[0]);
        command
            .args(&stage[1..])
            .env_clear()
            .envs(environment.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        if let Some(output) = previous.take() {
            command.stdin(Stdio::from(output));
        } else if let Some(path) = &plan.stdin_file {
            command.stdin(
                fs::File::open(path)
                    .map_err(|err| error(format!("couldn't read file \"{path}\": {err}")))?,
            );
        } else if plan.stdin_text.is_some() {
            command.stdin(Stdio::piped());
        } else {
            command.stdin(Stdio::null());
        }
        let stderr = if position < last && plan.pipe_stderr[position] {
            None
        } else if let Some((path, append)) = &plan.stderr_file {
            Some(Stdio::from(open_output(path, *append)?))
        } else if plan.stderr_passthrough {
            Some(Stdio::inherit())
        } else if plan.stderr_to_stdout && position == last {
            None
        } else {
            Some(Stdio::from(
                stderr_capture
                    .try_clone()
                    .map_err(|err| error(format!("exec: {err}")))?,
            ))
        };
        if position < last {
            command.stdout(Stdio::piped());
        } else if let Some((path, append)) = &plan.stdout_file {
            command.stdout(open_output(path, *append)?);
        } else if plan.stdout_to_stderr {
            command.stdout(Stdio::inherit());
        } else {
            command.stdout(Stdio::piped());
        }
        match stderr {
            Some(stderr) => {
                command.stderr(stderr);
            }
            None => {
                // Merge stderr into the stage's stdout pipe or file.
                command.stderr(Stdio::piped());
            }
        }
        let mut child = command
            .spawn()
            .map_err(|err| error(format!("couldn't execute \"{}\": {err}", stage[0])))?;
        if position == 0
            && let (Some(text), Some(mut input)) = (&plan.stdin_text, child.stdin.take())
        {
            let text = text.clone();
            std::thread::spawn(move || {
                let _ = input.write_all(text.as_bytes());
            });
        }
        if position < last {
            previous = child.stdout.take();
        }
        children.push(child);
    }
    let mut last_child = children.pop().unwrap();
    let mut output = Vec::new();
    let stderr_reader = last_child.stderr.take().map(|mut stderr| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
            bytes
        })
    });
    if let Some(mut stdout) = last_child.stdout.take() {
        let _ = stdout.read_to_end(&mut output);
    }
    let merged = stderr_reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    let mut failed = false;
    for mut child in children {
        if let Some(mut stderr) = child.stderr.take() {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
            output.extend(bytes);
        }
        failed |= !child.wait().map(|status| status.success()).unwrap_or(false);
    }
    failed |= !last_child
        .wait()
        .map(|status| status.success())
        .unwrap_or(false);
    output.extend(merged);
    let mut captured = Vec::new();
    let mut capture = stderr_capture;
    use std::io::Seek;
    let _ = capture.seek(std::io::SeekFrom::Start(0));
    let _ = capture.read_to_end(&mut captured);
    let mut text = String::from_utf8_lossy(&output).into_owned();
    if !keep_newline && text.ends_with('\n') {
        text.pop();
    }
    let stderr_text = String::from_utf8_lossy(&captured).into_owned();
    if failed {
        let mut message = text.clone();
        if !stderr_text.is_empty() {
            if !message.is_empty() {
                message.push('\n');
            }
            message.push_str(stderr_text.trim_end_matches('\n'));
        }
        if message.is_empty() {
            message = "child process exited abnormally".into();
        }
        return Err(error(message));
    }
    if !ignore_stderr && !stderr_text.is_empty() {
        let mut message = text;
        if !message.is_empty() {
            message.push('\n');
        }
        message.push_str(stderr_text.trim_end_matches('\n'));
        return Err(error(message));
    }
    Ok(text.into())
}
