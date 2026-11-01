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

## Install

On macOS (Apple Silicon or Intel), install the latest release with:

```sh
curl -fsSL https://raw.githubusercontent.com/tsirysndr/macports-rs/main/install.sh | sh
```

This installs `port` into `/usr/local/bin` (using sudo when needed) after
checking the download against its published SHA-256. Set `PORT_RS_VERSION` to
install a specific tag, or `PORT_RS_INSTALL_DIR` to choose another directory.
If MacPorts is also installed, whichever `port` comes first in `PATH` runs.
`port selfupdate` later replaces the binary with newer releases. The installer
also links `portindex`, which the same binary provides.

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
its Portfiles; the downloaded index alone only supports indexed queries. A tree
without an index can be indexed with `portindex` (or by listing it as a
`file://` source in `sources.conf` and running `port sync`).

## Binary archives

`port install` prefers signed binary archives from the MacPorts package
servers. A port whose archive is available needs only its runtime
dependencies, so build-only dependencies are skipped. All archives an install
needs are downloaded concurrently before installation starts, up to four
transfers at a time. Set `fetch_threads` in `macports.conf` to change the
limit (1 to 128):

```
fetch_threads 8
```

Ports without an archive, or whose download fails, are built from source.
`-s` always builds from source and `-b` requires binaries.

## Benchmarks

Read-only queries against the same ports tree (42,470 ports) and registry as
MacPorts 2.12.7, on an Apple M4 (10 cores, macOS 15.6). Best of 20 runs per
command, both binaries interleaved:

| Command                   | MacPorts | port-rs | Speedup |
| ------------------------- | -------: | ------: | ------: |
| `port version`            |    58 ms |    5 ms |   11.8× |
| `port info curl`          |   203 ms |  178 ms |    1.1× |
| `port search --name curl` |   136 ms |  118 ms |    1.2× |
| `port list curl`          |   160 ms |   97 ms |    1.7× |
| `port deps curl`          |   186 ms |  177 ms |    1.1× |
| `port variants curl`      |   183 ms |  170 ms |    1.1× |
| `port rdeps curl`         |   724 ms |  555 ms |    1.3× |
| `port echo all`           |  1622 ms |  118 ms |   13.7× |
| `port installed`          |    64 ms |    6 ms |   10.4× |
| `port outdated`           |    78 ms |   92 ms |    0.9× |

Commands that only read the registry start in a few milliseconds. Queries
that read the index parse all of it (about 90 ms), where MacPorts seeks to one
record through `PortIndex.quick`, so single-port lookups such as `outdated`
gain least. Commands that evaluate Portfiles (`info`, `deps`, `variants`)
spend most of their time in the Tcl interpreter. Reproduce with:

```sh
cargo build --release --locked
python3 scripts/compare.py target/release/port
```

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
