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
./target/release/port search --categories net
./target/release/port search --depends zlib
./target/release/port info --index --line --name --version --categories curl zlib
./target/release/port -q info --index --version curl
./target/release/port deps --index curl
./target/release/port rdeps --index --no-build --no-test curl
./target/release/port echo rdepends:zlib and category:net
./target/release/port -q echo category:net
./target/release/port -q echo '(' category:net or zlib ')' and not 'name:ish$'
./target/release/port echo curl@8.0_1 +ssl -universal build.jobs=8
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
| `search` | Name/description defaults, explicit metadata field filters, `--depends`, exact/glob/regex, quiet and line modes |
| `info --index` | Ordered fields, `--line`, `--pretty`, metadata summaries, maintainer decoding, and dependency formatting |
| `echo` | Names/globs, regex field selectors, logical expressions, versions, variants, and options |
| `deps --index`, `rdeps --index` | Direct/recursive indexed dependencies, build/test filters, full trees, cycle handling |
| `variants` | Indexed variant names only; **not** evaluated Portfile results |
| Index data | Serialized Tcl list data is parsed in Rust; no Tcl code is evaluated |
| `install`, `upgrade`, `uninstall`, activation | Not implemented; fail explicitly before filesystem changes |
| Registry queries, `sync`, `selfupdate` | Not implemented |
| Port specifications | Attached/separate `@version`, `+variant`, `-variant`, and `key=value`; canonical echo rendering |
| Logical expressions | `and`, `or`, `not`, `!`, parentheses, and implicit union; preserves input order |
| `rdepends:<port>` | Indexed transitive reverse dependencies; exact, case-insensitive port names |
| Registry selectors, `depof`/`rdepof`, current directory, Portfile URLs | Not implemented |
| Command chaining | Quoted `;` separators, per-action option reset, stops on first error |
| Interactive mode and command files | Not implemented |

Chain actions with a quoted or escaped semicolon so your shell passes it to
`port`, for example `port -q echo curl ';' info --index --version curl`.
Global options before the first action apply to every action; options within an
action reset before the next one. Empty segments are ignored. Actions run in
order and stop on the first error; successful earlier actions are not rolled
back. The parsed index is reused until `--index-file` changes, so a chain sees a
consistent catalog snapshot for each uninterrupted use of a source. Semicolons
inside a larger argument are ordinary characters. No shell interpreter is used.

Place command flags before the first port expression. Field selectors use
case-insensitive regular expressions, including singular/plural aliases such as
`category:`/`categories:` and `variant:`/`variants:`. For example,
`name:^curl$` matches an exact name, and `depends:zlib$` searches the indexed
dependency fields. An unmatched glob or field selector produces an empty set.
Adjacent expressions form a union; `not` binds tighter than `and`, and `and`
binds tighter than `or`. Quote globs and shell parentheses.

`search` defaults to matching names or brief descriptions. Supplying field flags
replaces that default; multiple fields are ORed without duplicate results.
Examples include `--categories`, `--maintainer`, `--version`, `--long_description`,
and individual `--depends_lib`/`--depends_build` fields. `--depends` includes all
seven dependency phases.

`info` emits explicit fields in command-line order. `--line` removes labels and
uses tabs between fields and commas within lists, preserving empty columns.
Normal multi-port output uses `--` separators; line output uses one row per port.
`--pretty` formats selected fields for reading; no field flags produce a metadata
summary. Maintainer handles and obfuscated addresses are decoded. Dependencies
show port names by default; `-v` retains full dependency specifications. Index
queries still cannot determine how requested variants change dependencies.

Specs can be attached (`curl@8.0_1+ssl`) or separated
(`curl @8.0_1 +ssl -universal build.jobs=8`). Negative variants following a
version can use a slash (`@8.0/-ssl`) to distinguish them from version suffixes
such as `@8.0-beta`. Echo sorts enabled variants before disabled variants,
retains distinct explicit versions, and preserves the first duplicate spec.
`list` still shows the available indexed version and warns when a version is
requested. `info --index` explicitly reads available indexed metadata; it does
not evaluate requested variants. `variants` with overrides is rejected until
native Portfile evaluation exists.

Dependency inspection requires explicit `--index` while the native Portfile
engine is unavailable. `deps --index` groups dependencies by phase; `rdeps --index`
walks their complete indexed graph. `--no-build` removes fetch, extract, patch,
and build edges; `--no-test` removes test edges at every level. Library/runtime
edges remain. `--full` repeats shared subtrees and annotates cycles; the default
prints each dependency once. `-v` retains `port:`, `bin:`, `lib:`, and `path:`
specifications, while `-q` suppresses headings. Both dependency commands validate
all selected roots before printing; recursive traversal also validates reachable
ports and uses an explicit stack for deep graphs.
As in `info --index`, version/variant overrides do not change indexed dependency
metadata. Indexed inspection extracts the final colon-separated component as
the fallback name, including unusual indexed forms such as
`port:bin/cmake:cmake`; verbose output preserves the original specification.
Empty fallback names and malformed serialized lists produce contextual errors.
These commands list fallback ports; they do not check whether a
`bin:`, `lib:`, or `path:` requirement is already satisfied on the machine.

`rdepends:zlib` selects all indexed ports that transitively depend on `zlib`,
across all seven dependency phases. It matches an exact port name rather than a
regex and composes with logical expressions and modifiers. Reverse edges are
built once per expression, and cycles terminate without duplicates. A cyclic
path can include the named port itself. Unlike `dependentof`/`rdependentof`, this
selector does not inspect the installed-port registry.

Catalog expansions and search results use dictionary ordering: numeric runs
compare by value (`python9` before `python10`), with case and leading zeros
breaking ties. Explicit port arguments and unions retain their input order.
Non-ASCII case mapping uses Rust Unicode tables, which can differ from the
Unicode version used by a particular Tcl release.

Search regexes currently use Rust regex semantics, which differ from Tcl's
advanced regular expressions. Glob behavior, Unicode sorting edge cases, description
wrapping, some metadata formatting edge cases, and error details still need
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

## Code organization

- `src/main.rs`, `src/cli.rs`: CLI options, command chaining, and exit handling.
- `src/commands/`: action-specific arguments and handlers for search, info,
  dependency queries, downloads, and basic index inspection.
- `src/dependencies/`: dependency specifications, phase filters, graph traversal,
  and direct/tree output.
- `src/index.rs`, `src/source.rs`, `src/tcl_list.rs`: catalog parsing and discovery.
- `src/query.rs`, `src/spec.rs`, `src/order.rs`: selectors, expressions, port
  specifications, and dictionary ordering.
- `src/info.rs`, `src/fields.rs`, `src/output.rs`: metadata and text formatting.
- `src/download.rs`, `src/progress.rs`, `src/theme.rs`: transfers and terminal UI.

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

## License

[MIT](LICENSE) — Copyright (c) 2026 Tsiry Sandratraina <tsiry.sndr@rocksky.app>.
