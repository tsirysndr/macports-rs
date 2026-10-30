# port-rs

A standalone Rust implementation in progress of the MacPorts `port` command.
The compatibility target is the original CLI and existing MacPorts data, including
ordinary commands such as `port install curl +ssl`. No Tcl executable, Tcl library,
Go runtime, OpenSSL installation, or existing `port` executable is required by
this binary. Rust crates are linked at build time; normal operating-system
libraries are still used.

**This is not yet a drop-in replacement. Installation is not implemented.**
It currently provides native index queries, a tested parallel download engine,
and terminal progress rendering. It does not evaluate Portfiles, mutate an
existing MacPorts registry, or forward commands to another executable.

## Build and run

```sh
cargo build --release --locked
./target/release/port --help

# Use an existing, uncompressed MacPorts PortIndex.
export PORT_RS_INDEX=/path/to/PortIndex
./target/release/port list
./target/release/port list 'curl*'
./target/release/port search --name curl
./target/release/port search --exact --line curl
./target/release/port -q info --index --version curl
./target/release/port -q echo category:net
```

The index path defaults to `./PortIndex`. `--index-file PATH` is a configuration
extension; it is deliberately distinct from MacPorts' `info --index` flag.
There is no automatic index synchronization yet.

## Current compatibility boundary

| Area | Status |
| --- | --- |
| `list [ports...]`, `list all` | Native, including upstream column formatting and no-argument default |
| `search` | Native name/description search; exact/glob/regex, case sensitivity, quiet and line modes |
| `info --index` | Selected metadata fields; default pretty formatting and full flag set are incomplete |
| `echo` | Simple names, globs, and supported indexed field selectors; compound specs are incomplete |
| `deps`, `variants` | Indexed metadata only; **not** evaluated Portfile results |
| Index data | Serialized Tcl list data is parsed in Rust; no Tcl code is evaluated |
| `install`, `upgrade`, `uninstall`, activation | Not implemented; fail explicitly before filesystem changes |
| Registry queries, `sync`, `selfupdate` | Not implemented |
| Full selector grammar, `@version`, `+variant`, command chaining, interactive mode | Not implemented |

Search regexes currently use Rust regex semantics, which differ from Tcl's
advanced regular expressions. Glob behavior, dictionary sorting, description
wrapping, field ordering, some metadata formatting, and error details still need
full upstream parity tests. Tests currently check representative upstream-derived
behavior; they are not a complete differential compatibility suite.

## Download engine and progress

Tokio and reqwest provide bounded concurrent HTTP(S) transfers with a shared
connection pool and Rustls TLS. Downloads stream to uniquely named temporary
files rather than accumulating bodies in memory. SHA-256 and optional size
checks run before atomic, no-clobber publication. Verified files are reused;
corrupt existing files are reported and preserved. Retries and alternative
mirrors are supported. A batch failure still drains other pending downloads.

The progress renderer uses the requested **`#7517fc`** accent, compact spinners,
per-file bars, byte counts, throughput, and ETA. It handles unknown content
lengths without inventing a percentage. Progress goes to stderr; redirected
output uses plain phase messages. `NO_COLOR`, `TERM=dumb`, and `-q` are respected.
Only real download and verification stages are shown. There is no simulated
installation progress.

For development, the engine is exposed through an **additional** `download`
command. This manifest is a downloader test interface, **not** a replacement
package format or an alternative implementation of `port install`:

```json
{
  "files": [
    {
      "name": "source.tar.gz",
      "urls": ["https://example.org/source.tar.gz"],
      "sha256": "REPLACE_WITH_THE_ACTUAL_64_HEX_DIGIT_SHA256",
      "size": 12345
    }
  ]
}
```

```sh
./target/release/port download --manifest downloads.json --output distfiles --jobs 8
```

`size` is optional. Filenames must be single components. Digests are mandatory.
The initial implementation retries a failed URL up to three times before moving
to the next mirror; it does not yet implement HTTP range resume or conditional
index refresh.

## Verification and performance

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo bench --locked --bench index
```

After dependency installation, tests need no network access: HTTP integration
tests bind a local loopback server. Tests cover index corruption, Unicode record
lengths, nested/escaped list values, CLI output, concurrent-transfer limits,
checksums, cache validation, retries, mirrors, truncated/oversized responses,
unsafe paths, symlinks, temporary-file cleanup, and exact RGB progress output.
CLI tests run with an empty `PATH` to detect accidental subprocess dependencies.
CI runs formatting, lint, tests, and a release build on macOS and Linux.

Read-only commands do not initialize Tokio or reqwest. The benchmark reports
index parsing and in-memory search separately using 50,000 synthetic ports.
One local optimized run parsed the 9.4 MiB synthetic index in **101.78 ms**
and searched the loaded index in **1.53 ms** per iteration. These measurements
exclude startup, disk I/O, and real-world Portfile evaluation.

**No speed claim against the original `port` has been established**: MacPorts is
not installed in the development environment. Real-index measurements, peak
memory, and differential benchmarks on the same machine remain required.

## Next compatibility gates

1. Port specifications, global/action options, configuration and source discovery,
   selectors, field formatting, and error/exit behavior matched against upstream.
2. Native evaluation of existing Portfiles and PortGroups, including Tcl language
   semantics, variants, platforms, callbacks, and MacPorts extensions. Parsing
   serialized index lists is not a substitute for this engine.
3. Dependency/version resolution using evaluated metadata; native fetch/checksum
   phases using the existing parallel downloader.
4. Build/archive handling and registry-compatible transactions, locks, activation,
   rollback, upgrades, and uninstall; connect progress events to these real phases.
5. End-to-end compatibility tests using real ports, and performance comparisons
   before claiming replacement compatibility.

Original interface references: [MacPorts guide](https://guide.macports.org/#using.port),
[port implementation](https://github.com/macports/macports-base/blob/master/src/port/port.tcl),
and [PortIndex writer](https://github.com/macports/macports-base/blob/master/src/port/portindex.tcl).
