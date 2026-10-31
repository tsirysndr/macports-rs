//! Tcl file channels for Portfiles: `open`, `close`, `puts`, `gets`, `read`,
//! `eof`, `flush`, `seek`, `tell` and `fconfigure`. Reading works anywhere;
//! opening for writing requires an execution-enabled session.
use super::*;
use molt::{MoltResult, check_args};
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
};

pub(super) struct Channel {
    reader: BufReader<File>,
    eof: bool,
}

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    for (name, command) in [
        ("open", open as molt::CommandFunc),
        ("close", close),
        ("puts", puts),
        ("gets", gets),
        ("read", read),
        ("eof", eof),
        ("flush", flush),
        ("seek", seek),
        ("tell", tell),
        ("fconfigure", fconfigure),
    ] {
        interp.add_context_command(name, command, id);
    }
}

fn channel<'a>(
    interp: &'a mut Interp,
    id: ContextID,
    name: &Value,
) -> Result<&'a mut Channel, Exception> {
    interp
        .context::<State>(id)
        .channels
        .get_mut(name.as_str())
        .ok_or_else(|| error(format!("can not find channel named \"{name}\"")))
}

fn io(context: &str, err: std::io::Error) -> Exception {
    error(format!("{context}: {err}"))
}

fn open(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 4, "fileName ?access? ?permissions?")?;
    let path = argv[1].as_str();
    let access = argv
        .get(2)
        .map(Value::to_string)
        .unwrap_or_else(|| "r".into());
    let mode = match argv.get(3) {
        Some(value) => u32::try_from(value.as_int()?).map_err(|_| error("invalid permissions"))?,
        None => 0o666,
    };
    let mut options = OpenOptions::new();
    let writes = match access.as_str() {
        "r" => {
            options.read(true);
            false
        }
        "r+" => {
            options.read(true).write(true);
            true
        }
        "w" => {
            options.write(true).create(true).truncate(true);
            true
        }
        "w+" => {
            options.read(true).write(true).create(true).truncate(true);
            true
        }
        "a" => {
            options.append(true).create(true);
            true
        }
        "a+" => {
            options.read(true).append(true).create(true);
            true
        }
        flags => {
            let mut writes = false;
            for flag in Value::from(flags).as_list()?.iter() {
                match flag.as_str() {
                    "RDONLY" => {
                        options.read(true);
                    }
                    "WRONLY" => {
                        options.write(true);
                        writes = true;
                    }
                    "RDWR" => {
                        options.read(true).write(true);
                        writes = true;
                    }
                    "APPEND" => {
                        options.append(true);
                        writes = true;
                    }
                    "CREAT" => {
                        options.create(true);
                    }
                    "TRUNC" => {
                        options.truncate(true);
                    }
                    "EXCL" => {
                        options.create_new(true);
                    }
                    "NOCTTY" | "NONBLOCK" | "BINARY" => {}
                    other => return Err(error(format!("invalid access mode \"{other}\""))),
                }
            }
            writes
        }
    };
    if writes && interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, &[Value::from("open")]);
    }
    options.mode(mode);
    let file = options
        .open(path)
        .map_err(|err| io(&format!("couldn't open \"{path}\""), err))?;
    let state = interp.context::<State>(id);
    state.next_channel += 1;
    let name = format!("file{}", state.next_channel);
    state.channels.insert(
        name.clone(),
        Channel {
            reader: BufReader::new(file),
            eof: false,
        },
    );
    Ok(name.into())
}

fn close(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 3, "channelId ?direction?")?;
    let state = interp.context::<State>(id);
    if state.build_states.remove(argv[1].as_str()).is_some() {
        return Ok(Value::empty());
    }
    match state.channels.remove(argv[1].as_str()) {
        Some(mut channel) => {
            channel
                .reader
                .get_mut()
                .flush()
                .map_err(|err| io("error flushing channel", err))?;
            Ok(Value::empty())
        }
        None => Err(error(format!("can not find channel named \"{}\"", argv[1]))),
    }
}

fn puts(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 4, "?-nonewline? ?channelId? string")?;
    let mut args = &argv[1..];
    let newline = if args.first().is_some_and(|arg| arg.as_str() == "-nonewline") {
        args = &args[1..];
        false
    } else {
        true
    };
    let (name, text) = match args {
        [text] => ("stdout".to_string(), text.to_string()),
        [name, text] => (name.to_string(), text.to_string()),
        _ => {
            return Err(error(
                "wrong # args: should be \"puts ?-nonewline? ?channelId? string\"",
            ));
        }
    };
    let text = if newline { format!("{text}\n") } else { text };
    match name.as_str() {
        "stdout" => {
            print!("{text}");
            let _ = std::io::stdout().flush();
        }
        "stderr" => eprint!("{text}"),
        _ => channel(interp, id, &Value::from(name.as_str()))?
            .reader
            .get_mut()
            .write_all(text.as_bytes())
            .map_err(|err| io("error writing", err))?,
    }
    Ok(Value::empty())
}

fn gets(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 3, "channelId ?varName?")?;
    let channel = channel(interp, id, &argv[1])?;
    let mut line = String::new();
    let count = channel
        .reader
        .read_line(&mut line)
        .map_err(|err| io("error reading", err))?;
    let ended = count == 0;
    if ended {
        channel.eof = true;
    }
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    } else if !ended {
        channel.eof = true;
    }
    match argv.get(2) {
        Some(name) => {
            let length = if ended {
                -1
            } else {
                line.chars().count() as i64
            };
            interp.set_var(name, line.into())?;
            Ok(length.into())
        }
        None => Ok(line.into()),
    }
}

fn read(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 3, "?-nonewline? channelId ?numChars?")?;
    let (strip, name, limit) = if argv[1].as_str() == "-nonewline" {
        (
            true,
            argv.get(2)
                .cloned()
                .ok_or_else(|| error("missing channel"))?,
            None,
        )
    } else {
        (
            false,
            argv[1].clone(),
            argv.get(2).map(|v| v.as_int()).transpose()?,
        )
    };
    let channel = channel(interp, id, &name)?;
    let mut bytes = Vec::new();
    match limit {
        Some(limit) => {
            let mut buffer = vec![0; usize::try_from(limit).unwrap_or(0)];
            let count = channel
                .reader
                .read(&mut buffer)
                .map_err(|err| io("error reading", err))?;
            buffer.truncate(count);
            bytes = buffer;
            if count == 0 {
                channel.eof = true;
            }
        }
        None => {
            channel
                .reader
                .read_to_end(&mut bytes)
                .map_err(|err| io("error reading", err))?;
            channel.eof = true;
        }
    }
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if strip && text.ends_with('\n') {
        text.pop();
    }
    Ok(text.into())
}

fn eof(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "channelId")?;
    Ok(channel(interp, id, &argv[1])?.eof.into())
}

fn flush(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "channelId")?;
    match argv[1].as_str() {
        "stdout" => {
            let _ = std::io::stdout().flush();
        }
        "stderr" => {}
        _ => channel(interp, id, &argv[1])?
            .reader
            .get_mut()
            .flush()
            .map_err(|err| io("error flushing", err))?,
    }
    Ok(Value::empty())
}

fn seek(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 4, "channelId offset ?origin?")?;
    let offset = argv[2].as_int()?;
    let position = match argv.get(3).map(Value::as_str).unwrap_or("start") {
        "start" => SeekFrom::Start(u64::try_from(offset).map_err(|_| error("invalid offset"))?),
        "current" => SeekFrom::Current(offset),
        "end" => SeekFrom::End(offset),
        other => return Err(error(format!("bad origin \"{other}\""))),
    };
    let channel = channel(interp, id, &argv[1])?;
    channel
        .reader
        .seek(position)
        .map_err(|err| io("error during seek", err))?;
    channel.eof = false;
    Ok(Value::empty())
}

fn tell(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "channelId")?;
    let position = channel(interp, id, &argv[1])?
        .reader
        .stream_position()
        .map_err(|err| io("error during tell", err))?;
    Ok((position as i64).into())
}

/// Encoding and translation options are accepted; channels are byte streams.
fn fconfigure(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(
        1,
        argv,
        2,
        0,
        "channelId ?optionName? ?value? ?optionName value ...?",
    )?;
    Ok(Value::empty())
}
