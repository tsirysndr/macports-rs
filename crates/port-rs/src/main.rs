mod cli;
mod shell;

use port_rs::commands;
use std::{
    ffi::OsString,
    io::{self, BufRead, IsTerminal, Write},
    path::PathBuf,
};

fn report(error: &anyhow::Error) {
    if error
        .downcast_ref::<io::Error>()
        .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
    {
        return;
    }
    eprintln!("Error: {error:#}");
}

/// Run one command line (possibly several `;`-separated actions); false on error.
fn run_line(session: &mut commands::Session, args: Vec<OsString>) -> bool {
    let process_all = port_rs::options::get().process_all;
    let mut ok = true;
    for args in cli::invocations(args) {
        let result = cli::Cli::parse_invocation(args).and_then(|cli| {
            commands::set_archive_mode(match (cli.source_only, cli.binary_only) {
                (true, _) => commands::ArchiveMode::SourceOnly,
                (_, true) => commands::ArchiveMode::BinaryOnly,
                _ => commands::ArchiveMode::Any,
            });
            session.run(
                cli.command,
                cli.index_file.as_deref(),
                cli.registry_file.as_deref(),
                cli.quiet,
                cli.verbose,
            )
        });
        if let Err(error) = result {
            report(&error);
            ok = false;
            if !process_all {
                break;
            }
        }
    }
    ok
}

/// The `--index-file` named on the command line or in the environment.
fn index_file(args: &[OsString]) -> Option<PathBuf> {
    args.windows(2)
        .find(|pair| pair[0] == "--index-file")
        .map(|pair| PathBuf::from(&pair[1]))
        .or_else(|| std::env::var_os("PORT_RS_INDEX").map(PathBuf::from))
}

/// Upstream's shell and `-F` files: one command line per line, `#` comments,
/// `cd` to a port or directory, and `quit`/`exit`.
fn command_file(
    session: &mut commands::Session,
    base: &[OsString],
    input: &mut dyn BufRead,
    interactive: bool,
) -> bool {
    let quiet = base.iter().any(|arg| {
        let arg = arg.to_string_lossy();
        arg == "--quiet" || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains('q'))
    });
    let noisy = interactive && !quiet;
    if noisy {
        println!("port-rs {}", env!("CARGO_PKG_VERSION"));
        println!("Entering shell mode... (\"help\" for help, \"quit\" to quit)");
    }
    let process_all = port_rs::options::get().process_all;
    let mut ok = true;
    let mut editor = interactive
        .then(|| shell::LineEditor::new(index_file(base)))
        .flatten();
    loop {
        let prompt = if noisy {
            let directory = std::env::current_dir().unwrap_or_default();
            let parts: Vec<_> = directory.iter().collect();
            let short: PathBuf = parts[parts.len().saturating_sub(2)..].iter().collect();
            format!("[{}] > ", short.display())
        } else {
            String::new()
        };
        let line = if let Some(editor) = &mut editor {
            match editor.read(&prompt) {
                Some(line) => line,
                None => {
                    if noisy {
                        println!();
                    }
                    break;
                }
            }
        } else {
            if noisy {
                print!("{prompt}");
                let _ = io::stdout().flush();
            }
            let mut line = String::new();
            match input.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    if noisy {
                        println!();
                    }
                    break;
                }
                Ok(_) => line,
            }
        };
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words = match port_rs::tcl_list::parse(line) {
            Ok(words) => words,
            Err(error) => {
                report(&error);
                ok = false;
                continue;
            }
        };
        match words.first().map(String::as_str) {
            Some("quit" | "exit") => break,
            Some("cd") => {
                let target = words.get(1).cloned().unwrap_or_default();
                let result = commands::port_directory(&target, index_file(base).as_deref())
                    .and_then(|directory| Ok(std::env::set_current_dir(directory)?));
                if let Err(error) = result {
                    report(&error);
                    ok = false;
                }
            }
            _ => {
                let args: Vec<OsString> = base
                    .iter()
                    .cloned()
                    .chain(words.into_iter().map(OsString::from))
                    .collect();
                if !run_line(session, args) {
                    ok = false;
                }
            }
        }
        if !ok && !interactive && !process_all {
            break;
        }
    }
    if noisy {
        println!("Goodbye");
    }
    ok
}

fn run() -> anyhow::Result<bool> {
    let mut args: Vec<OsString> = std::env::args_os().collect();
    let leading = cli::take_leading(&mut args)?;
    port_rs::options::set(leading.options.clone());
    if let Some(directory) = &leading.directory {
        let target = commands::port_directory(directory, index_file(&args).as_deref())?;
        std::env::set_current_dir(&target).map_err(|err| {
            anyhow::anyhow!(
                "port -D could not change directory to {}: {err}",
                target.display()
            )
        })?;
    }
    let mut session = commands::Session::default();
    let base_end = 1 + cli::leading_count(&args[1..]);
    let base: Vec<OsString> = args[..base_end].to_vec();
    let asks_clap = base[1..]
        .iter()
        .any(|arg| matches!(arg.to_str(), Some("--help" | "-h" | "--version" | "-V")));
    let has_action = base_end < args.len() || asks_clap;
    let mut ok = true;
    if has_action {
        ok = run_line(&mut session, args.clone());
    }
    let mut files = leading.command_files.clone();
    if !has_action && files.is_empty() {
        files.push("-".into());
    }
    if ok || leading.options.process_all {
        for file in files {
            let file_ok = if file == "-" {
                let interactive = io::stdin().is_terminal();
                command_file(&mut session, &base, &mut io::stdin().lock(), interactive)
            } else {
                let input = std::fs::File::open(&file)
                    .map_err(|err| anyhow::anyhow!("Failed to open command file; {err}"))?;
                command_file(&mut session, &base, &mut io::BufReader::new(input), false)
            };
            ok &= file_ok;
            if !ok && !leading.options.process_all {
                break;
            }
        }
    }
    Ok(ok)
}

fn main() {
    // SAFETY: this is the first operation in main, before CLI dispatch creates
    // runtimes or starts concurrent downloads/build work.
    unsafe {
        port_rs::source::capture_creation_mask_before_threads();
    }
    // Invoked as `portindex` (a link to this binary), act as that tool.
    let mut args = std::env::args();
    let program = args.next().unwrap_or_default();
    if std::path::Path::new(&program)
        .file_name()
        .is_some_and(|name| name == "portindex")
    {
        let args: Vec<String> = args.collect();
        match port_rs::commands::portindex::main(&args) {
            Ok(code) => std::process::exit(code),
            Err(error) => {
                report(&error);
                std::process::exit(1);
            }
        }
    }
    match run() {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(error) => {
            report(&error);
            std::process::exit(1);
        }
    }
}
