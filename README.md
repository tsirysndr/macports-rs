# port-rs

A standalone Rust implementation in progress of the MacPorts `port` command.
The compatibility target is the original CLI and existing MacPorts data, including
ordinary commands such as `port install curl +ssl`. No Tcl executable, Tcl library,
Go runtime, OpenSSL installation, or existing `port` executable is required by
this binary. Rust crates are linked at build time; normal operating-system
libraries are still used. SQLite is bundled into the executable; no database
command or separately installed SQLite library is needed.

**This is not yet a drop-in replacement. Installation is not implemented.**
It currently provides native index and installed-registry queries, a tested parallel download engine,
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
./target/release/port variants --index curl
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
| `version`, `platform` | Native, index-free, supports quiet output |
| `list [ports...]`, `list all` | Native, including upstream column formatting and no-argument default |
| `search` | Name/description defaults, explicit metadata field filters, `--depends`, exact/glob/regex, quiet and line modes |
| `info --index` | Ordered fields, `--line`, `--pretty`, metadata summaries, maintainer decoding, and dependency formatting |
| `echo` | Names/globs, regex field selectors, logical expressions, versions, variants, and options |
| `deps --index`, `rdeps --index` | Direct/recursive indexed dependencies, build/test filters, full trees, cycle handling |
| `variants --index` | Sorted variant details, defaults/requested markers, descriptions, conflicts, and requirements |
| Index data | Serialized Tcl list data is parsed in Rust; no Tcl code is evaluated |
| `install`, `upgrade`, `uninstall`, activation | Not implemented; fail explicitly before filesystem changes |
| `installed` | Read-only SQLite registry, names/globs, version filters, active/inactive and requested/unrequested selectors |
| `contents` | Registered file lists, active-image preference, version selection, quiet and size/unit output |
| `space` | Active-file sizes, hard-link deduplication, symlink sizes, unit selection, and totals |
| `provides` | Active file ownership, quiet output, normalized paths and symlinks |
| `dependents`, `rdependents` | Installed-registry reverse dependencies, verbose versions, full trees, cycle handling |
| `outdated` | Active-version, revision, epoch, and Darwin-platform comparison against the catalog |
| Other registry commands, `sync`, `selfupdate` | Not implemented |
| Port specifications | Attached/separate `@version`, `+variant`, `-variant`, and `key=value`; canonical echo rendering |
| Logical expressions | `and`, `or`, `not`, `!`, parentheses, and implicit union; preserves input order |
| `rdepends:<port>` | Indexed transitive reverse dependencies; exact, case-insensitive port names |
| Registry selectors outside `installed`/`outdated`, `depof`/`rdepof`, current directory, Portfile URLs | Not implemented |
| Action abbreviations | Unique case-insensitive prefixes, exact-name priority, upstream ambiguity checks |
| Command chaining | Quoted `;` separators, per-action option reset, stops on first error |
| Interactive mode and command files | Not implemented |

`port version` reports this Rust implementation's package version, not a
MacPorts base version. `port platform` prints `darwin <major> <architecture>`
using the same native machine detection as automatic index selection (`arm` for
Apple Silicon, `i386` for Intel). Both work without a catalog, network access,
or external executables; `-q` omits the label.

Chain actions with a quoted or escaped semicolon so your shell passes it to
`port`, for example `port -q echo curl ';' info --index --version curl`.
Global options before the first action apply to every action; options within an
action reset before the next one. Empty segments are ignored. Actions run in
order and stop on the first error; successful earlier actions are not rolled
back. The parsed index is reused until `--index-file` changes, so a chain sees a
consistent catalog snapshot for each uninterrupted use of a source. Semicolons
inside a larger argument are ordinary characters. No shell interpreter is used.

Actions accept unique prefixes: `port sear curl`, `port ver`, and
`port help sear`. Exact lowercase action names take priority; otherwise prefixes
are matched case-insensitively against the upstream command vocabulary, including
commands not yet implemented here. For example, `dep` is ambiguous between
`dependents` and `deps`; `activ` resolves to `activate` and reports that it is not
implemented. The Rust `download` extension also participates. Wildcard action patterns such as `sea*` also work; quote them to prevent shell
expansion. Interactive-only action abbreviations are not supported.

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
not evaluate requested variants. `variants --index` displays available indexed
variant metadata with `[+]`/`[-]` defaults and `+`/`-` requested markers. Descriptions
wrap below their labels; conflicts and requirements appear on separate lines.
Requested markers only appear where the index supplies `vinfo` for that variant,
matching upstream behavior. Version and option overrides do not change indexed
metadata. Quiet mode suppresses headings. Bare `variants` requires Portfile
evaluation and fails explicitly; variants.conf defaults are not loaded yet.

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

Glob patterns use a native string matcher: `*`, `?`, character classes, ranges,
and backslash escapes. Braces are literal, `!`/`^` inside classes do not negate
matches, and `/` has no path-separator semantics. Case-insensitive ranges fold
their endpoints before matching. The test suite includes 1,336 comparisons
recorded with macOS Tcl 8.5.9; running it does not require Tcl.

Search regexes currently use Rust regex semantics, which differ from Tcl's
advanced regular expressions. Malformed glob corner cases, Unicode matching/sorting edge cases, description
wrapping, some metadata formatting edge cases, and error details still need
full upstream parity tests. Tests currently check representative upstream-derived
behavior; they are not a complete differential compatibility suite.

## Installed-port registry

`port installed` reads `/opt/local/var/macports/registry/registry.db` without
loading a PortIndex. Override the path with `--registry-file PATH` or
`PORT_RS_REGISTRY`. Custom `macports.conf` paths are not discovered yet.
The database is opened read-only, never initialized or migrated. A missing
default registry reports no installed ports; a missing explicit path is an
error. SQLite handles committed WAL data and concurrent readers normally.

```sh
port installed
port -q installed active
port installed 'python*'
port installed active and requested
port installed '(' curl or 'python*' ')' and not inactive
port installed curl @8.10.1_0+ssl
port -v --registry-file /custom/registry.db installed requested
```

Both active (`installed`) and inactive (`imaged`) registry entries are shown.
Results sort by name, MacPorts version segments, revision, and variants. Quiet
mode hides headings, while verbose mode includes recorded requested variants,
platform, architectures, and local installation timestamps. Version filters
match a plain version or the complete version/revision/variant string.

Registry queries support names, globs, `all`, `installed`, `active`, `inactive`,
`requested`, and `unrequested`. Combine selectors with `not` (or `!`), `and`,
and `or`, in that precedence order; parentheses override precedence and adjacent
expressions form a union. Set operations preserve distinct installed images.
Complements and globs use the installed registry, including ports removed from
the catalog. Nesting is limited to 128 levels. Field/path selectors still require
catalog queries; registry mutations remain unimplemented.

`port contents curl` lists the active image's registered filesystem paths; if no
active image matches, a single inactive image is used. When several inactive
versions match, specify a full version such as `port contents curl @8.10.1_0+ssl`.
Inactive images list original image paths, while active images list active
`actual_path` records. Entry and file queries share a read transaction, avoiding
mixed results if another process activates a port concurrently.

Use `port contents --size --units KiB curl` to include current filesystem sizes.
Supported units are B, KiB, kB, MiB, MB, GiB, and GB, including upstream short
forms. Sizes follow symlinks; missing or unreadable paths display `-1`. File
contents are never opened. Quiet mode hides headings. This initial `contents`
implementation requires explicit names (with optional versions/variants); globs,
logical expressions, and registry selectors are not yet supported for this action.

`port space curl` reports the sizes of registered active files. Use `--units B`
(or the units accepted by `contents`) for a fixed unit and `--total` to print
only the sum. Multiple ports also produce a total by default; quiet mode retains
size rows. Hard links are counted once per port, symlinks contribute their own
length, and missing/unreadable files are skipped. These are logical file sizes,
not allocated disk blocks or compressed archive sizes.

This action currently requires explicit names, with optional version/variant
qualifiers. It always measures the active image and warns when a specified
version differs. Inactive/missing ports and ports without registered files emit
diagnostics and do not contribute to the total. Registry queries remain
read-only and do not load a catalog or execute external tools.

`port provides /opt/local/bin/curl` looks up active file-map ownership, using
`actual_path` rather than the original image path. Multiple filenames are
accepted, and `-q` prints only owner names. Missing paths, directories, and
unowned files produce distinct messages in normal mode and no output in quiet
mode; these conditions are successful queries.

Relative paths and parent directory symlinks are normalized. A final symlink is
looked up by its own path, including dangling links and links to directories.
On macOS, native filesystem capability detection controls case-sensitive versus
case-insensitive registry matching. Ownership lookup uses bound SQL parameters
and never reads file contents or modifies registry records.

`port dependents zlib` lists registered ports that depend on `zlib`, including
active and inactive images. Normal output deduplicates port names; verbose
output includes each recorded version. `-q` prints just the dependent names.
`port rdependents zlib` walks the registry graph and suppresses shared subtrees;
`--full` repeats them and includes versions, while `-v` includes versions without
repeating subtrees. Cycles terminate; full output marks the repeated edge with
`(cyclic dependency)` rather than following it indefinitely.

These commands require explicit port names with optional version/variant
qualifiers. An active root image is preferred; otherwise an installed inactive
image is used. Registry edges refer to names rather than specific target
versions. Globs, logical expressions, and pseudo-port selectors remain
unsupported for these actions. The dependency map is read once inside the same
read transaction as the entries, and traversal is iterative for deep graphs.

`port outdated` compares active installations with the current catalog, reusing
`--registry-file` and `--index-file` overrides. It accepts the same logical expressions, names,
globs, versions, and state/requested filters as `installed`; inactive images are
excluded. If no active entries match, the catalog is not loaded or downloaded.
Removed catalog entries are skipped, and missing version fields produce warnings.

Comparisons follow MacPorts version segments, revision, and epoch rules; epoch
changes override ordering only when the version strings differ. Otherwise-equal
entries built for another Darwin platform are reported for rebuilding. `-v` also
shows newer installed versions with `>` and `!`; quiet mode suppresses notices
but retains comparison rows. C++ standard-library mismatch checks and custom
configuration discovery are not implemented yet. This command only reports
available metadata; it does not evaluate Portfiles or perform upgrades.

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

Cached and explicitly configured index queries do not initialize Tokio or
reqwest. Catalogs expose read-only access after parsing. Exact names use the
ordered catalog directly; the first case-insensitive fallback builds a shared,
thread-safe name cache. Exact spelling still wins if names differ only in case.
The cache is reused by dependency traversal and subsequent actions in a chain.

The benchmark uses 50,000 synthetic ports. One local optimized run parsed the
9.4 MiB index in **95.36 ms** and searched it in **2.87 ms** per iteration.
The first fallback lookup, including cache construction, took **3.41 ms**.
Warm mixed-case lookup took **145 ns**, versus **406 µs** for the prior full-scan
implementation on the same fixture; a missing-name lookup took **98 ns**, versus
**111 µs** previously. These are synthetic microbenchmarks, exclude startup and
disk I/O, and do not measure real-world Portfile evaluation or cache memory use.

**No speed claim against the original `port` has been established**: MacPorts is
not installed in the development environment. A real Darwin 24 ARM index has been downloaded and parsed successfully. Peak
memory measurements and differential benchmarks on the same machine remain required.

## Code organization

- `src/main.rs`, `src/cli.rs`, `src/cli/actions.rs`: CLI options, command chaining,
  action expansion, and exit handling.
- `src/commands/`: action-specific arguments and handlers for search, info,
  dependency queries, downloads, and basic index inspection.
- `src/registry/`: read-only SQLite entries and MacPorts version comparison.
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
