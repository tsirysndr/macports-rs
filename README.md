# port-rs

[![Rust](https://github.com/tsirysndr/macports-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/tsirysndr/macports-rs/actions/workflows/ci.yml)

A standalone Rust implementation of the MacPorts `port` CLI, built with clap,
Tokio, and reqwest. No Tcl runtime or existing MacPorts executable required;
SQLite is bundled.

Every `port` action is implemented, from queries to source and binary
installs, upgrades, snapshots, sync, lint and installer packages, with the
[documented differences](docs/compatibility.md#actions-and-global-options).
Test it against a separate prefix (see `PORTSRC` in the guide) before using it
on an existing MacPorts installation.

## Quick start

Use macOS (Apple Silicon or Intel) with the stable Rust toolchain and Xcode
Command Line Tools.

```sh
cargo build --release --locked
./target/release/port --help
./target/release/port search --name curl
./target/release/port info --index curl
./target/release/port -y install curl
```

The first index query downloads and caches the index for your Mac's CPU and
Darwin version. Use `--index-file PATH` or `PORT_RS_INDEX` for a local index.
For local Portfile evaluation, select a ports tree containing a `PortIndex` and
its Portfiles; the downloaded index alone only supports indexed queries.

## Development

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --release
cargo bench --locked --bench index
```

Tests run offline after dependencies are installed; HTTP tests use local servers.
CI targets macOS `aarch64-apple-darwin` and `x86_64-apple-darwin` only.

The project is a Cargo workspace:

| Crate | Purpose |
| --- | --- |
| `crates/port-core/` | Catalog parsing, selectors, port specifications, dependency graphs, progress and terminal output |
| `crates/port-registry/` | SQLite registry queries, transactions and snapshots, through sqlx with sea-query statements |
| `crates/port-engine/` | Portfile evaluation (`portfile/`), builds (`build/`), signed archives and installation (`install/`), configuration (`config/`) |
| `crates/port-rs/` | The `port` binary: CLI parsing, command handlers (`src/commands/`), integration tests and benchmarks |
| `crates/molt/` | Vendored Tcl interpreter and [local patches](crates/molt/PORT-RS-PATCHES.md) |

## Contributing

Pick a gap from the [compatibility notes](docs/compatibility.md) and include a
regression test for the behavior you add or fix. Keep MacPorts command semantics
and existing data formats compatible; unsupported behavior should fail explicitly.
Use focused modules and conventional commits, and run the checks above before
opening a pull request.

The main remaining work is complete Portfile/PortGroup support, source-phase
scheduling, and end-to-end install, activation, upgrade, and uninstall. Performance
comparisons with the original `port` are still needed.

## Documentation

- [Command support and limitations](docs/compatibility.md#current-compatibility-boundary)
- [Registry behavior](docs/compatibility.md#installed-port-registry)
- [Installation and native execution](docs/compatibility.md#installation-planning)
- [Downloads and terminal UI](docs/compatibility.md#download-engine-and-progress)
- [Performance notes](docs/compatibility.md#verification-and-performance)
- [MacPorts interface reference](https://guide.macports.org/#using.port)

## License

[MIT](LICENSE) — Copyright (c) 2026 Tsiry Sandratraina <tsiry.sndr@rocksky.app>.
