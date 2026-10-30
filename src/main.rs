mod cli;

use port_rs::commands;
use std::io;

fn run() -> anyhow::Result<()> {
    let mut session = commands::Session::default();
    for args in cli::invocations(std::env::args_os()) {
        let cli = cli::Cli::parse_invocation(args)?;
        session.run(
            cli.command,
            cli.index_file.as_deref(),
            cli.registry_file.as_deref(),
            cli.quiet,
            cli.verbose,
        )?;
    }
    Ok(())
}
fn main() {
    // SAFETY: this is the first operation in main, before CLI dispatch creates
    // runtimes or starts concurrent downloads/build work.
    unsafe {
        port_rs::source::capture_creation_mask_before_threads();
    }
    if let Err(error) = run() {
        if error
            .downcast_ref::<io::Error>()
            .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
        {
            return;
        }
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}
