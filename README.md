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

# First query downloads the correct index for this Mac's CPU and Darwin version.
./target/release/port list
./target/release/port list 'curl*'
./target/release/port search --name curl
./target/release/port search --exact --line curl
./target/release/port -q info --index --version curl
./target/release/port -q echo category:net
```

Index selection uses this order:

1. `--index-file PATH`, or `PORT_RS_INDEX` when the flag is absent.
2. An existing `./PortIndex` for project-local queries.
3. A platform-specific default at
   `~/Library/Caches/port-rs/PortIndex_darwin_<major>_<arch>/PortIndex`.

The default is initialized automatically from the official MacPorts HTTPS endpoint
on the first index query. Subsequent queries use the cached file offline. The
machine's Darwin major version and CPU architecture determine the source: Apple
Silicon uses `arm`, Intel uses `i386` (including x86_64), and PowerPC uses
`powerpc`. Hardware detection also recognizes Apple Silicon under Rosetta; it
does not use subprocesses or assume the binary's compilation target is the CPU.
There is no silent fallback to a different architecture or OS version. Automatic
selection requires macOS; other systems can use an explicit or local index.

Downloads are size-limited, parsed, and checked for nonempty content before atomic
publication. Failed downloads leave no partial index, and concurrent first runs
do not overwrite each other's published files. Index authenticity currently
relies on HTTPS; detached MacPorts signatures are not verified. Explicit paths
are never initialized or overwritten automatically. `--index-file` is distinct
from MacPorts' `info --index` flag. Automatic refresh and `port sync` remain
unimplemented; removing the platform cache file triggers a fresh download.

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

Clap help headings, usage, command names, and option names share the **`#7517fc`**
accent with progress output. Help retains automatic color detection and respects
`NO_COLOR`; redirected help stays plain.

The progress renderer uses that accent, compact spinners,
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
CI runs formatting, lint, tests, and a release build exclusively on native macOS
Apple Silicon (`aarch64-apple-darwin`) and Intel (`x86_64-apple-darwin`) runners.

Cached and explicitly configured index queries do not initialize Tokio or reqwest. The benchmark reports
index parsing and in-memory search separately using 50,000 synthetic ports.
One local optimized run parsed the 9.4 MiB synthetic index in **101.78 ms**
and searched the loaded index in **1.53 ms** per iteration. These measurements
exclude startup, disk I/O, and real-world Portfile evaluation.

**No speed claim against the original `port` has been established**: MacPorts is
not installed in the development environment. A real Darwin 24 ARM index has been downloaded and parsed successfully. Peak
memory measurements and differential benchmarks on the same machine remain required.

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
