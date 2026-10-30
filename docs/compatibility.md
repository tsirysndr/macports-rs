# Compatibility and implementation notes

[Back to the project README](../README.md)

A standalone Rust implementation in progress of the MacPorts `port` command.
The compatibility target is the original CLI and existing MacPorts data, including
ordinary commands such as `port install curl +ssl`. No Tcl executable, Tcl library,
Go runtime, OpenSSL installation, or existing `port` executable is required by
this binary. Rust crates are linked at build time; normal operating-system
libraries are still used. SQLite is bundled into the executable; no database
command or separately installed SQLite library is needed.

**This is not yet a drop-in replacement. Installation is not implemented.**
It currently provides native index and installed-registry queries, a tested parallel download engine,
and terminal progress rendering. It evaluates a subset of local Portfiles for
variant and dependency inspection. It does not mutate an
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
| `deps`, `rdeps` | Evaluate supported local Portfiles for variant-dependent direct/recursive dependencies |
| `variants --index` | Sorted variant details, defaults/requested markers, descriptions, conflicts, and requirements |
| `variants` | Native evaluation of supported local Portfiles and PortGroups alongside a selected PortIndex |
| Index data | Serialized Tcl list data is parsed in Rust; no Tcl code is evaluated |
| `install` | `-y` / `--dry-run` previews local Portfile or indexed binary dependencies; actual installation is blocked |
| `upgrade`, `uninstall`, activation | Not implemented; fail explicitly before filesystem changes |
| `installed` | Read-only SQLite registry, names/globs, version filters, active/inactive and requested/unrequested selectors |
| `contents` | Registered file lists, active-image preference, version selection, quiet and size/unit output |
| `location` | Recorded image paths, explicit version selection, ambiguity errors, and quiet output |
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
metadata. Quiet mode suppresses headings.

`port --index-file /path/to/ports/PortIndex variants curl` evaluates the local
Portfile named by the index's `portdir`, with PortGroups from that tree's
`_resources/port1.0/group` directory. A `PortIndex` in the working directory also
selects its local tree. Platform variables describe the current Mac; the prefix
currently defaults to `/opt/local`. Requested variants/options are evaluated;
custom build bodies and hooks remain deferred. Missing sources and unsupported
Portfile commands fail explicitly, without falling back to cached metadata or
fetching source code. The downloaded default index alone is insufficient for this
mode. macports.conf and variants.conf defaults are not loaded yet.

Local-tree queries detect the macOS product version, physical CPU architecture,
active CPU count, and available memory through native system APIs. Deployment and
default SDK versions follow the macOS product version (including macOS 26).
Build jobs are resolved lazily from parallel-build settings, CPU count, and the
per-job memory allowance; explicit `build.jobs` overrides remain authoritative.
Build/universal architectures are filtered against the port and default SDK
version, including MacPorts' architecture demotions. This does not locate an
installed SDK or choose a compiler yet.

`deps` and `rdeps` use the same local-tree selection as `variants`. Selected
variants update dependency lists and appear in the full-name heading. Recursive
queries evaluate each reachable port once per root, passing explicit variant and
option choices to dependencies without imposing the root's version constraint.
Excluded dependency phases do not cause source loading. All selected roots and
reachable dependencies are validated before printing. These queries still report
fallback port names for bin/lib/path dependencies rather than probing installed
providers. Configuration files and unsupported PortSystem APIs remain limitations.

`deps --index` groups cached dependencies by phase; `rdeps --index`
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

`port installed` reads `/opt/local/var/macports/registry/registry.db`. Most
selectors do not load a PortIndex; `obsolete` needs one to check catalog membership. Override the path with `--registry-file PATH` or
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
`actinact`, `obsolete`, `requested`, `unrequested`, `leaves`, and `rleaves`. Combine selectors with `not` (or `!`), `and`,
and `or`, in that precedence order; parentheses override precedence and adjacent
expressions form a union. Set operations preserve distinct installed images.
Complements and globs use the installed registry, including ports removed from
the catalog. Nesting is limited to 128 levels. Field/path selectors still require
catalog queries; registry mutations remain unimplemented.

`port installed obsolete` selects installed images whose names are absent from
the current catalog, including inactive images. Lookup is case-insensitive;
`replaced_by` metadata alone does not make a port obsolete. Logical expressions
and version filters apply normally. The catalog uses `--index-file` or the
usual default index, and is loaded only when `obsolete` is evaluated against a
nonempty registry. Empty registries and other registry selectors stay index-free.
`outdated obsolete` has no comparison rows because obsolete ports have no catalog
entry to compare; `outdated not obsolete` compares the remaining active images.

`port installed actinact` selects every image of names that have both active
and inactive installations. Combine it with `and inactive` to list just their
inactive images, or pass a version filter to narrow the results. Names with
only active or only inactive images are excluded. It also works with `outdated`
and needs neither dependency tables nor an index for installed queries.

`port installed leaves` selects unrequested images with no registered dependents.
`port installed rleaves` also includes unrequested images whose dependents are
not needed by any requested port. Both account for active and inactive images;
dependency cycles terminate. These selectors compose with logical expressions
and version filters, and also work with `outdated`. Dependency data is loaded
once, only when needed, within the same read snapshot as installed images.
They are query selectors only: no ports are removed.

`installed` and `outdated` also accept `dependentof:PORT` and
`rdependentof:PORT` for direct and recursive registered dependents. For example,
`port installed rdependentof:zlib and requested` lists requested dependents.
The target is an exact, case-insensitive installed name; missing targets select
nothing. Results include all installed images of each dependent name, and can
be filtered with a separate version argument, such as `dependentof:zlib @1.0`.
Recursive traversal deduplicates shared branches, terminates cycles, and may
include the target itself when a cycle returns to it. No catalog is loaded.
These selectors are not yet available in catalog actions such as `echo`.

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

`port location pkg2` prints the registry's recorded image path; `-q` hides the
heading. Both active and inactive images are eligible. If several images match,
specify a plain version or complete version/revision/variant string, for example
`port location curl @8.10.1_0+ssl`. Unlike `contents`, an active image does not
resolve ambiguity automatically. This action currently accepts explicit names
only. It reads paths from the same registry snapshot as image selection, without
checking, opening, or extracting the recorded images. Missing location columns
or NULL values produce explicit errors, leaving other registry queries usable.

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

## Installation planning

`port -y install curl` (or `port install --dry-run curl`) previews a binary
installation plan using local Portfiles when available, otherwise indexed metadata,
and the read-only registry. It supports
multiple roots and indexed expressions, deduplicates shared dependencies, and
orders `depends_lib`/`depends_run` ports before their dependents. Build and test
dependencies are excluded from this binary-only preview. Rows distinguish new
installs, matching inactive images needing activation, and already-active images.
Explicit roots remain marked requested even when another root depends on them.

This is a preliminary plan, not an installer: archive availability, signatures,
platform compatibility, and activation hooks have not been checked. Replaced ports
and conflicting fully-resolved active images fail explicitly.

When the selected index has a local source tree alongside it, previews evaluate
supported Portfiles and PortGroups for the requested ports and their runtime
dependencies. Explicit variants and port options are passed to dependencies;
an explicitly requested dependency uses its own inputs. Version checks use the
evaluated version. Shared dependencies with conflicting evaluated metadata fail
before output. Missing or unsupported sources fail instead of mixing stale index
metadata into an evaluated plan. PortSystem initialization and deferred build or
activation hooks are not executed.

Without local sources, non-default variants and port options still fail explicitly.
Official indexes often omit default-variant metadata:
these entries now appear as `resolve ... [default variants unresolved]`, allowing
`port -y install curl` to preview its dependency graph. Provider dependencies
(`bin:`, `lib:`, `path:`) use their indexed fallback port with an explicit
`provider checks pending` annotation; existing external providers are not probed.
Unresolved defaults never approve an active image or an archive identity, and
explicit variant requests still require sufficient metadata. In index-only mode,
only the indexed version is supported.
Cycles and missing dependencies are rejected before any plan is printed.

Use `--index-file PATH` and `--registry-file PATH` for an entirely local preview.
Without an explicit index, normal index initialization can populate the metadata
cache. Planning never downloads package archives, writes the registry, or installs
files. Ordinary `port install` remains blocked before index/registry access until
archive verification, extraction, activation, and registry transactions are ready.
`-y` currently applies only to `install`; other actions reject it.

Native `dlist` APIs now select and evaluate dependency items using required
and optional dependency tokens. They preserve provider order, select required
closures without recursive stack growth, stop on handler failures, and support
completion predicates, custom selectors, handler argument prefixes, and failure
reasons. `upvar` supports scalar and whole-array aliases used by these callbacks;
array-element aliases are still unsupported. Qualified global references such as
`$::name`, `${::build.jobs}`, and `$::env(KEY)` resolve through the same storage
and lazy-default hooks as unqualified globals, while preserving local shadows.
Full namespace evaluation, imports, and command resolution are still incomplete.
These are scheduling primitives:
the full PortSystem target runner, state files, privilege handling, and built-in
phase procedures are not connected to ordinary installation yet.

The library's `portfile::Session` and `open_from_tree` retain the interpreter
alongside the initial metadata snapshot. Later literal-argument procedure calls
share the original globals, arrays, procedures, selected variants, and lazy
defaults. User target wrappers can therefore access their Portfile context.
This API is a building block for the phase scheduler; it does not run native
initialization or prerequisites. External commands remain disabled unless the
caller explicitly attaches a `system` executor.
Metadata queries continue to discard the interpreter after extracting metadata.

`Session::enable_system` supplies an explicit environment and a host executor.
It supports `system command`, `-W directory`, `-notty`, `-nice value`, and `--`, using
`/bin/sh -c` as MacPorts does. Each call reads the live `env` array, including
removed variables, without changing the Rust process environment or directory.
The build runner streams both output streams with phase/port prefixes and returns
PID/exit status for Tcl `CHILDSTATUS` and `CHILDKILLED` errors. Scripts can catch
ordinary command failures. `-notty` starts a child session without a controlling
terminal, and `-nice` requests an absolute child priority; denied priority changes
are ignored as in MacPorts. Cancellation stops the child process group in either
mode. Callback and direct-I/O flags remain explicit errors. The synchronous interpreter must run on a worker when
using the async build runner; this does not yet wire ordinary CLI installation,
PortSystem target scheduling, or cancellation of the interpreter itself.

With execution enabled, `command_exec` assembles command/pre/post argument shell
fragments, supports `-varprefix`, `-notty`, per-command `.nice`, and shell
prefixes/suffixes, prepares the command
directory, applies the command environment and explicit compiler/SDK settings,
and restores the session environment after success or failure. The executor uses
fallible `SystemRequest::into_command` to prepare directories relative to its
working directory. `parse_environment`, `append_to_environment_value`, and
`environment_array_to_string` implement the corresponding native helpers.

`portbuild::build_main` now executes configured build commands, including
make/ninja job arguments and make directory logging. `do-build` can delegate to
it from a custom build block. Enabling execution installs lazy defaults for
source/staging directories, configure/build/test/destroot command directories,
compiler include/library paths, verbose compiler logging, and Xcode versus CLT
developer-directory selection. Explicit values and Portfile defaults take
precedence. Native `findBinary` prefers executable configuration hints, then
uses `macports::binaryInPath` to search the session's explicit PATH without
consulting the process PATH. The default build command selects make/BSD make/GNU make or
the configured Xcode command using upstream build-type/platform rules. Missing
tools produce warnings and fail before build execution. Callers still supply
the prefix/work directory, SDK, timestamp, execution environment, and selected
Xcode directory/command when needed. Compiler discovery and full PortSystem
initialization remain unfinished.
Native progress callbacks remain unsupported. A macOS integration test compiles a C program and verifies
prefixed compiler diagnostics through this path.

`portconfigure::configure_main` runs autoreconf, automake, and autoconf in order,
then selects xmkmf/Makefiles generation or the configured configure command.
It combines user environment values with compiler wrappers, pipe/SDK flags,
tool and language options, pkg-config paths, canonical architecture flags, and
universal or march/mtune settings. Custom configure blocks can delegate through
`do-configure`. Failures stop subsequent commands, retain live logs, and restore
the session environment. Callers must still initialize the compiler options;
compiler discovery, option dependency traces, privileged ccache initialization,
SDK whitelist defaults, and full CLI phase scheduling remain unfinished. Native
configure start/finish callbacks are described below.

`portdestroot::destroot_main` executes the inherited build command with the
default `install` target and `DESTDIR=${destroot}`, honoring Portfile overrides.
Custom staging blocks can delegate through `do-destroot`. The host must provide
`system_options(clonebin_path)` (empty disables wrappers); its temporary PATH
prefix is restored on success and failure. Tests stage an executable into a
temporary prefix with live logs. Native non-root destroot finish now performs
staging cleanup and restores the saved mask. Native non-root startup now prepares the hierarchy and mask for ports without
startup items and for supported custom or executable-based launchd services. Privilege transitions and privileged ownership conversion remain unfinished.

The CLI captures and restores its inherited creation mask at the start of
`main`, before starting runtimes or concurrent work. New Portfile sessions inherit
that immutable startup value. Embedding hosts can instead supply or override it
with `Session::initialize_creation_mask`. Native `umask mode` accepts BSD octal
and symbolic modes and returns the previous mask. Each child applies the mask
after fork, and newly prepared command directories receive the same permissions;
existing directories retain theirs. The Rust process's global umask is never
changed during evaluation or execution. Library hosts without a startup capture
must supply the initial mask explicitly. Destroot finish restores the saved mask on both success and failure; native
destroot startup saves and sets the configured staging mask. Prepared-target
execution also restores the mask when a staging hook or body fails before finish.

`porttest::test_main` runs configured test commands when `test.run` is true,
using the inherited build command/directory and the default `test` target.
Custom test blocks can delegate through `do-test`; child status and environment
cleanup use the same execution path as builds. On macOS, native staged-file
inspection warns about mismatched or missing Mach-O architectures before running
port tests. `get_canonical_archs` selects noarch, selected universal architectures,
the configured build architecture, or the host fallback using upstream rules.
Symlinks and non-binary data are skipped. `test.ignore_archs yes` disables the
scan; disabling `test.run` does not. Temporary privilege elevation for a process
that has dropped root privileges remains explicitly unsupported. Full CLI test
scheduling remains unfinished.

The native `macho` library now inspects CPU architectures in thin Mach-O files,
32/64-bit universal containers, and Unix static archives, including BSD extended
member names. It reads headers through bounded seeks rather than loading whole
files, validates container ranges and load-command lengths, and limits nesting
and inspection work. Tests include real arm64/x86_64 compiler output and a
static library. The test phase uses this parser for staged-file validation.

The `portfile` library now evaluates an initial set of Portfile metadata commands
in Rust using [Molt](https://docs.rs/molt/latest/molt/), without `tclsh` or libtcl.
It supports language substitutions, expressions, procedures and loops, option
set/append/prepend/delete/replace operations, explicit option overrides, subports,
variant declarations, conditional defaults, requirements and conflicts. Selected
variant bodies update dependency/build metadata in requirement order. Hook bodies
are retained without execution, and configuration variables are supplied explicitly
instead of inheriting the process environment.

Custom target declarations such as `configure {}`, `build { ... }`, and
`destroot { ... }` are retained separately from pre/post hooks. The last override
for a phase wins, including empty bodies that disable its built-in action;
pre/post hooks accumulate in declaration order. Only selected variant/subport/
platform bodies register overrides. These scripts are not executed during
metadata evaluation. After an override, `do-<phase>` invokes the original
procedure captured when the phase was registered, passing its phase name.
Most built-in phase procedures still need native implementations; the build
procedure can run with explicitly supplied configuration.

PortSystem dependency-item records now support `ditem_create`, key access,
append/unique append, membership, snapshots, and deletion. `target_new` and
target-property setters retain custom metadata; standard phase handles expose
their names, requirements, and procedure references. Hook and override declarations
update these records and define their user-procedure wrappers with global-variable
access. Records are returned with the evaluation result. `target_provides` adds
provided tokens and registers custom phase declarations and pre/post hooks.
Existing phase commands retain their original binding when another target provides
the same token; pre/post declarations bind to the latest provider. Renaming a
phase declaration preserves its bound target identity. Full target execution,
state-file integration, and the remaining built-in phase procedures are incomplete.

The `commands` declaration creates the standard command-option families, including
`.cmd`, `.dir`, `.args`, and `.env`. Configure/compiler and livecheck metadata
setters accept Portfile values and user overrides. Compiler/SDK selection and
their native defaults and option traces remain incomplete; declaring these fields
does not approve a build configuration or execute the corresponding programs.

PortGroups can be imported from explicitly configured local directories or
embedded sources, with deterministic search precedence and one evaluation per
name/version. Nested and recursive includes share the interpreter. Imported
source bytes are fingerprinted with SHA-256 and returned with their size and
origin for later registry recording. Failed imports invalidate the evaluation,
even when caught by script code. Group-defined options accept user overrides. `platform` blocks evaluate OS,
subplatform, release and architecture-family conditions using explicit machine
variables, and execute matching bodies in the caller's scope.

Lazy `default` values now resolve on global variable reads, including reads from
expressions and procedures. Explicit writes/unsets cancel defaults; re-registering
a default updates its template without overriding explicit values. Computed
`distname` and `distfiles` follow name/version and archive-format changes. The
`use_*` extraction choices act as radio buttons while preserving an explicit
`extract.suffix`. Circular defaults and excessive evaluation depth are reported.
The [vendored Molt core](../vendor/molt/PORT-RS-PATCHES.md) provides host variable hooks;
its upstream tests run in the macOS CI matrix alongside port-rs tests.
Integer arguments and expressions recognize Tcl 9's case-insensitive `0x`, `0o`,
`0b`, and `0d` prefixes; plain leading-zero integers are decimal. The interpreter
still uses signed 64-bit integers rather than Tcl's arbitrary-precision numbers.
The interpreter also implements `string map` for PortGroup-generated procedures
and template values, including ordered substitutions and `-nocase` matching.
`split`, `lrange`, and end-relative/arithmetic `lindex` indices support Portfile
list and URL transformations without an external interpreter.
The native `quotemeta` helper escapes non-word characters for livecheck patterns.
`shellescape` (also `macports::shellescape`) applies MacPorts' character escaping
for shell arguments. Native `realpath` resolves existing paths and symlinks with
catchable errors; `portutil::get_oldbuildpath` reproduces the legacy URL/path-based
build-directory mapping. Execution-enabled `portutil::create_workpath` creates
`.home`/`.tmp`, repairs the legacy build link, and migrates pre-2.11 directories
or older linked build trees while preserving file identity on the same volume.
`chownAsRoot` applies recursive link-aware ownership when already running as root
and is a no-op for ordinary users. Temporary privilege elevation, cross-volume
migration, state-file lifecycle policy, and CLI integration remain unfinished.
The `build::state::StateFile` library reads and appends MacPorts' line-based
version/checksum/target/variant records while holding an exclusive `flock`.
It supports nonblocking acquisition, waiting, duplicate suppression, v1 timestamp
and v2 checksum checks, and recorded-variant comparison. Unsupported filesystem
locking is exposed to the caller, matching upstream's permitted fallback.
Execution-enabled sessions can take ownership of a locked file through
`Session::attach_build_state`. The resulting handle supports `check_statefile`,
`write_statefile`, `get_statefile_value`, `check_statefile_variants`, and `close`.
Output variables use the caller's scope; closing the handle or dropping the
session releases the lock. State class patterns currently use Rust regex syntax,
so Tcl-specific advanced expressions remain a compatibility gap.
`Session::open_build_state` implements real-build checksum initialization,
v1/v2 invalidation, future-Portfile rejection, and ignore-different handling
once the host has prepared the work directory and privileges. Unknown versions
are returned to the host for warnings. Empty state files are initialized as
fresh (including recovery from an interrupted first open). Stale-work cleanup
retains the work directory and locked state inode, unlike upstream's
pre-lock directory deletion; its records are reset only after cleanup succeeds.
This serializes cooperating Rust builders, but cannot protect against external
tools unlinking the state file. The native PortSystem `open_statefile` command now connects work-directory
creation/migration, optional work symlinks, state opening, and session handles.
It reads configuration in global scope even when invoked inside a procedure.
Dry runs skip directory and link mutations and invalidation, open existing state
without locking, and use `/dev/null` when state is absent, matching upstream.
Root execution is explicitly unsupported until the privilege worker exists;
the command must not change process-wide credentials in the concurrent CLI.
State warnings currently go to stderr; lock-wait progress and full UI routing,
writable-tree ownership preparation, runtime `.home`/`.tmp` recreation after
invalidation, and CLI phase scheduling remain unfinished.
`Session::execute_target` runs a prepared dependency item's init, prerun,
pre-hooks, body, post-hooks, and postrun callbacks. Initialization runs before
completed-state and dry-run checks. Failures stop later callbacks and leave the
phase unrecorded; successful non-always stateful targets append completion.
The caller owns the state handle and its lifetime. This is the callback/state
portion of `target_run`: runtime-package loading, prerequisites, environment,
working directory, sandbox/tracing, privileges, and log copying still belong to
the host driver and are not supplied by this method.
Native `portconfigure::configure_finish` dispatches enabled checks for implicit
function declarations, implicit int, and incompatible function-pointer types.
The checks scan nested config.log, CMakeError.log, and meson-log.txt files in
Rust without grep, match warning flags at line ends, and apply case-insensitive
Tcl-style whitelist globs. Diagnostics use the shared progress manager. The
implicit-function whitelist must currently be supplied by the host or Portfile;
SDK-dependent whitelist defaults are not implemented, and missing configuration
fails explicitly. Native configure startup validates upstream compiler identifiers and shares
compiler-fallback warning suppression with build startup. A configured compiler
is still a host input; discovery, fallback selection, and compiler debug-name
reporting are not implemented. Ccache startup retains upstream's root requirement;
initialization under root remains unsupported until the privilege worker exists.
A real-shell test exercises configure start, main, and finish through the prepared
target runtime, including state-based skipping on a second invocation.
Native `portbuild::build_start` and `porttest::test_start` now emit phase notices;
build startup also warns once per session about compiler fallback when the host
sets `portconfigure::no_default_compiler_allowed`. This flag defaults to false;
compiler discovery/selection itself remains unfinished. `Session::set_progress`
shares the installer's progress manager with these callbacks, preserving quiet
mode and concurrent download bars; notices honor `UI_PREFIX`. Real-make tests
exercise both built-in phases through the prepared runtime and verify that a
second invocation skips their completed work.
`Session::run_prepared_target` adds automatic state opening/closing and the
PortSystem environment lifecycle: build-local HOME/TMPDIR, DEVELOPER_DIR during
user hooks/body, then database HOME and no TMPDIR after execution. Cleanup also
runs after callback errors or caught unsupported operations. Changes stay in the
session's env array, not the process environment. The dry-run argument governs
both opening and execution, and the previous global dry-run option is restored.
Runtime packages, working-directory handling, sandbox/tracing, privileges,
log copying, the full PortSystem `target_run` command, and CLI phase scheduling
remain unfinished.
Native `file join`, `dirname`, `tail`, `extension`, `rootname`, `split`,
`pathtype`, and `separator` construct and inspect Unix paths without filesystem
access. These follow the current MacPorts-bundled Tcl 9 lexical rules, including
literal tildes, retained dot components, and `//server` roots.
Native `file normalize` makes paths absolute, handles dot components and parent
symlinks, preserves a final symlink, and tolerates nonexistent suffixes. It is
available during metadata evaluation and does not create directories. Empty input
remains empty, following Tcl. Read-only
`file exists`, `isfile`, `isdirectory`, `readable`, `writable`, `executable`, and
`owned` inspect the host filesystem, follow symlinks, and return false on lookup
errors. Access checks use native OS permissions, not just mode bits; relative
paths use the process working directory. These queries run during metadata
evaluation too, so compiler/dependency choices can depend on host files.
`file size`, `mtime`, `atime`, `type`, `readlink`, `stat`, and `lstat` expose
read-only metadata with catchable POSIX lookup errors. Stat follows symlinks;
lstat and type inspect the link itself. Stat results support both Tcl 9 dictionary
returns and caller-scope arrays. Execution-enabled sessions can set `file atime`
and `file mtime` using whole-second timestamps, preserving the other field's
seconds and clearing subsecond precision in both fields, as Tcl does. Setters
follow symlinks and return the value read back from the filesystem. Metadata
sessions cannot set timestamps. Readlink currently requires a UTF-8 target.
Execution-enabled sessions support native `file mkdir` with multiple paths,
recursive parents, directory symlinks, and catchable POSIX errors. New directories
use the session's creation mask; existing directory permissions are preserved.
Relative paths use the process working directory, as with the read-only queries.
Directory creation remains unavailable during metadata evaluation.
Native `file delete` supports multiple paths, `--`, and recursive `-force`.
Missing files are ignored; directory links are unlinked without traversing their
contents. Nonempty directories require `-force`, which grants owner access to the
root directory and restores its original permissions on failure. Errors remain
catchable, and deletion is unavailable during metadata evaluation.
Native `file rename` moves files, links, and directories on the same filesystem
in execution-enabled sessions, with multiple sources, directory destinations,
`--`, and `-force`. Source links move as links; destination links to directories
are followed. Cross-volume copy/delete fallback remains unimplemented and returns
EXDEV while retaining the source.
Native `file link` reads symbolic targets during metadata evaluation and creates
symbolic or hard links in execution-enabled sessions. Creation requires an
existing target. Relative symbolic targets resolve from the link's parent;
relative hard-link targets resolve from the process working directory.
Existing destinations are never overwritten, and link errors are catchable.
Native `fs-traverse` supports sorted physical traversal, `-depth`, `-tails`,
`-ignoreErrors`, caller-scope loop variables, and Tcl loop control. Explicit
root symlinks are followed; nested directory symlinks and other filesystems are
not descended into. Paths currently require UTF-8. The native
`portutil::get_latest_path_mtime` helper finds the latest regular-file timestamp
using the same traversal. Automatic `source_date_epoch` still needs Git-history
resolution before its full upstream default can be enabled.
Native `file attributes path -permissions` reads Unix modes; execution-enabled
sessions can update them with numeric, nine-character, or Tcl-style symbolic
permission strings. Native `-owner` and `-group` attributes query account names
(with numeric fallback) and set names or numeric IDs using reentrant OS lookups
and `chown`. Multiple attribute updates run in order. Queries and updates follow
symlinks; setters remain unavailable during metadata evaluation. Other attributes
and complete attribute listings remain explicitly unsupported.
Other filesystem-backed `file` operations and subcommand abbreviations remain
unimplemented.

This evaluator is connected to local-tree `variants`, `deps`, and `rdeps` queries,
but not installation.
The complete PortSystem API still needs implementation, including namespace
commands and build-stage defaults used by larger PortGroups. Unsupported operations fail; guarded attempts to
call unsupported I/O commands are recorded and cannot yield approved metadata.
Pinned, unmodified [curl fixtures](../tests/fixtures/curl/README.md) now test its
default variants, explicit resolver/TLS variants, conflicts, dependency metadata,
and local-tree CLI queries. Library tests supply explicit build inputs; CLI tests
cover both native host detection and explicit overrides. They do not run PortSystem
initialization, universal-build setup, hooks, or installation, and do not establish
compatibility with the wider ports tree.

The installer library now includes streaming Signify/Ed25519 `.sig`
authentication with the pinned MacPorts 2025 archive key. Verification reads
64 KiB chunks, authenticates through EOF, and returns the byte count and SHA-256.
Verified staging copies those exact bytes into an anonymous temporary file with
a caller-specified size limit; later readers cannot accidentally reopen a changed
source archive. Failed authentication or staging never publishes an archive.
These are installer building blocks, not yet connected to `port install` or its
preview. Legacy RSA `.rmd160` signatures and activation are still pending. No external signature tool is invoked.

Authenticated `.tbz2` archives can now be inspected in Rust without extraction.
Inspection validates `+CONTENTS` identity and inventory against tar entries,
reads regular files and link metadata, and can match name/version/revision/epoch/
variants to an installation-plan item. It bounds decompressed bytes, entry count,
control-file size, and PAX metadata. The reader rejects traversal, duplicate and
ASCII case-colliding paths, descendants of non-directories, special files, invalid
hard links, malformed/truncated streams, and trailing archive data. Absolute
symlink targets remain inert metadata and are never followed. `+PORTFILE` is not
evaluated. Selected local PAX fields (including paths and macOS file flags) are
supported; global PAX, GNU long-name/sparse entries, and PAX size overrides fail
explicitly. Filesystem-specific collision checks, platform compatibility, and
activation policy are still required before this can become an installer.

Authenticated images can also be prepared in an owner-private temporary directory.
Preparation first inspects the complete archive and checks its package identity
against a resolved plan item. It then copies payloads in archive order, separates
control files, creates hard links, and creates symlinks last without following
them. Staged files remain non-executable with mode 0600; original modes and flags
are retained as metadata for later activation. Failures and dropped images remove
the entire temporary directory. This does not write into the live prefix, execute
Portfile hooks, or register installed ports.

The native registry writer now initializes schema 1.215 and provides explicit
transactions for image registration, file ownership, activation, deactivation,
requested flags, and removal. Failed registrations and activations roll back to
savepoints; dropping a transaction rolls it back. Existing older or newer schemas
are refused without migration. This API does not yet coordinate live filesystem
changes, and read-only commands never initialize a registry.

Archive targets follow MacPorts' Darwin release, `darwin_any`, and `any_any`
naming rules. Native targets use arm64 or x86_64 from machine detection; resolved
build properties can select noarch, an explicit build architecture, or sorted
universal architectures. Long variant suffixes use MacPorts' RIPEMD-160 filename
fallback through the pure Rust `ripemd` crate. Target-aware preparation checks
the signed `+CONTENTS` architecture set before extraction. Platform declarations
and build architecture still require resolved metadata; missing PortIndex fields
are not sufficient to approve a target.

Signed archive transport now uses Tokio and reqwest with bounded parallel
transfers and ordered mirror fallback. Each archive and its `.sig` are fetched
under size limits; signature verification runs on a blocking worker and retains
the same private temporary file without a second archive copy. A mirror with an
invalid signature is rejected. Batch failures drain pending transfers and drop
all staged results. This transport is available to the installation engine;
archive URL selection and the ordinary install command are not yet connected.

The source-phase process runner streams stdout and stderr live, without requiring
verbose mode. Every displayed line is labeled, for example
`-> building git: CC builtin/commit.o`. Partial lines are flushed immediately;
interleaved streams get separate labeled lines, and carriage-return progress is
rendered as labeled updates. Terminal prefixes use the CLI accent color, while
redirected output remains plain text. Transfer bars pause during compiler output.
Build failures preserve the exit status, and cancellation or a broken output sink
stops the build process group, including compiler children. The runner is tested
independently; source-stage scheduling and normal `port build`/`port install`
integration remain unfinished.

## Download engine and progress

Tokio and reqwest provide bounded concurrent HTTP(S) transfers with a shared
connection pool and Rustls TLS. Downloads stream to uniquely named temporary
files rather than accumulating bodies in memory. SHA-256 and optional size
checks run before atomic, no-clobber publication. Verified files are reused;
corrupt existing files are reported and preserved. Retries and alternative
mirrors are supported. A batch failure still drains other pending downloads.

Help and progress use the Rocksky CLI palette: teal **`#00E8C6`** headings and
phase markers, cyan **`#00C6E8`** command/option names and active progress,
violet **`#8264FF`** arguments, green **`#64E882`** valid values, and coral
**`#FF6464`** errors. Help retains automatic color detection and respects
`NO_COLOR`; redirected help stays plain.

On a TTY, concurrent transfers share an indicatif `MultiProgress` renderer with
cyan compact spinners, per-file bars, byte counts, throughput, and ETA. An unset
`TERM` still permits TTY progress; `TERM=dumb` disables it. Hidden progress does
not start per-transfer tick timers. It handles unknown content
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

Once installation works end to end, benchmark it against the original `port` on
the same Mac and publish a compact results table in the README. Record hardware,
OS/tool versions, ports and variants, exact commands, repeated-run timings, and
cold/warm cache conditions. Keep detailed methodology and results in this document;
report measured differences without assuming a speedup.

## Code organization

- `src/main.rs`, `src/cli.rs`, `src/cli/actions.rs`: CLI options, command chaining,
  action expansion, and exit handling.
- `src/commands/`: action-specific arguments and handlers for search, info,
  dependency queries, downloads, and basic index inspection.
- `src/build/`: asynchronous source-phase execution and live prefixed logs.
- `src/portfile/`: Rust metadata evaluation, options, and variant selection.
- `src/registry/`: SQLite readers, explicit transactional writes, and MacPorts version comparison.
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

[MIT](../LICENSE) — Copyright (c) 2026 Tsiry Sandratraina <tsiry.sndr@rocksky.app>.

`Session::clean_destroot_libtool` supplies the libtool portion of destroot finish.
It recognizes the first-line libtool marker, removes archives when requested,
or clears dependency_libs quote contents in regular files without rewriting
symlinks. Absolute symlink targets are checked inside the staging root. Link
recognition precedes deletion so traversal order cannot leave recognized aliases
behind. Rewrites preserve inode, hardlinks, owner/group, and permissions and
support non-UTF-8 content. This helper assumes ownership preparation is complete;
it does not independently record phase completion. The native non-root finish
callback composes it with the other cleanup steps.

`Session::check_destroot_layout` implements destroot finish's advisory layout
check. It reports all violations, including hidden entries, while accepting the
standard prefix children and configured application/framework directories plus
LaunchAgents, LaunchDaemons, and StartupItems. The exact `yes` override skips
inspection and emits the upstream notice. Prefix ancestors and the prefix itself
follow directory symlinks as upstream glob does; permitted subtrees are not
recursively checked. Non-UTF-8 filenames fail explicitly. An empty or absent
stage is not a layout violation by itself: destroot finish runs the separate
empty-stage guard before this check. The report does not authorize
activation or mark staging complete.

`Session::prune_destroot_directories` implements keepdir markers, post-order
empty-directory removal, and the empty-stage error. New directories use 0755;
new `.turd_<subport>` markers use 0644. Existing markers and directory permissions
are preserved. Relative and external absolute paths are placed under destroot;
paths already inside it remain there. Path normalization follows the lexical
rules in [Tcllib fileutil](https://github.com/tcltk/tcllib/blob/master/modules/fileutil/fileutil.tcl),
with component-based containment and rejection of existing outward directory
symlinks instead of permitting escapes. Pruning never recursively deletes
nonempty directories or follows descendant directory links. The native non-root
finish callback runs this after libtool cleanup and before compression.

`Session::compress_destroot_manpages` normalizes plain, gzip, and bzip2 pages in
physical man/cat section directories into level-9 gzip with no filename or time
in its header. Compression streams through a temporary file; decode failures
preserve the source and existing destination. Successful outputs retain source
owner/group and filesystem timestamps and use mode 0444. The gzip implementation
uses [flate2's Rust backend](https://docs.rs/flate2/latest/flate2/), with the
existing bundled bzip2 decoder; no compression executable is required. Compressed
bytes need not match the system gzip implementation. Relative and staged absolute
links are updated after pages, including repeated passes for link chains; name
collisions fail without replacing the colliding link. Descendant directory links
are skipped and an outward-resolving prefix is rejected. The native non-root
finish callback runs this before the layout check.

`portdestroot::destroot_finish` now composes stray index-file removal, libtool
cleanup, keepdir/pruning and empty-stage checks, man-page compression, and layout
warnings for non-root execution. On macOS it also attempts the sparse-file
workaround using SEEK_HOLE and native clonefile into a private temporary directory;
errors are ignored as upstream treats this workaround as best effort. The saved
`portdestroot::oldmask` is saved by native startup; callers invoking finish
directly must initialize it themselves. The session mask is restored on failures as well as success. Tests
verify that the prepared-target runtime records completion only after finish
succeeds. Privileged finish fails before cleanup because build-account ownership
conversion and the privilege worker are still missing; ordinary CLI installation
remains unavailable.

`Session::prepare_destroot_hierarchy` now prepares the directory hierarchy portion
of non-root destroot startup in process. It reads the installed
`portsharepath/install/{prefix,macosx}.mtree` specifications, creates the Darwin
application/framework directories, and applies the prefix hierarchy. Both
specifications are parsed before filesystem changes. The native directory reader
supports `/set`, `/unset`, repeated root blocks, directory entries, octal modes,
and ownership attributes from MacPorts' templates. Existing extra files remain
untouched; directory modes are updated. Non-root staging retains caller ownership.
Symlinks in hierarchy entries are rejected rather than followed. The caller must
provide a private, already-created destroot and an initialized creation mask.
The native `portdestroot::destroot_start` callback uses this helper after optional
cleanup (`destroot.clean` exactly `yes`) and staging-mask setup. It checks root
requirements and unsupported startup-item requests before cleanup. Ports without
startup items, with an explicitly disabled single item, or with a supported
custom launchd plist can complete the non-root staging lifecycle. Executable-based generated plists and port-supplied custom plists are supported
for single and multiple startup items, including generated phase wrappers. The default staging mask comes
from `system_options(destroot_umask)`; the host must supply it or an explicit
`destroot.umask`. Startup errors restore the prior mask. General
mtree file/link/checksum specifications, escaped filenames, and wildcard entries
are explicitly rejected rather than silently interpreted as directory templates.


Native `portstartupitem::startupitem_create` stages port-supplied regular
`startupitem.custom_file` for launchd. It honors the item name, unique name, plist
filename, and LaunchAgents/LaunchDaemons location. Copying preserves file bytes,
permissions, and timestamps and refuses an existing destination, matching the
non-forced copy in upstream startup preparation. The link uses the absolute
installed path; non-root staging puts it under `${prefix}/etc/${location}` even
when `startupitem.install` is enabled. Existing links are replaced without
following their targets. No service is loaded and no external command is run.
A disabled type skips staging. When type is omitted, creation must be enabled
and the host supplies `system_options(startupitem_type)` for type selection.
Privileged placement still fails explicitly. Custom source symlinks, path components containing slashes or
parent references, and symlinked hierarchy directories are rejected. Tests cover
metadata preservation, names, link replacement, disabled items, collision handling,
and custom-plist staging through the complete native non-root destroot lifecycle.


Multiple custom startup items use the native `startupitems` name/value list.
Each `name` begins or revisits an item; revisiting merges attributes while resetting
its derived unique name and plist filename, matching upstream dictionary behavior.
Missing attributes inherit global `startupitem.*` settings, while derived names
remain item-specific. An omitted type uses the global type default, including the
global creation flag, rather than the per-item creation flag. Disabled items are
skipped. All item definitions and custom source files are validated before any
item is staged, so an invalid later definition does not leave an earlier plist
behind. Filesystem copy/link failures can still leave a partially staged batch;
no completion state is recorded on failure. Tests cover inheritance, mixed
LaunchAgents/LaunchDaemons, repeated names, disabled items, and invalid batches.


Executable-based startup items now generate launchd XML natively when no init,
start, stop, or restart wrapper code is requested. The plist launches daemondo
with the executable's Tcl argument list and supports network-change restarts,
event verbosity, PID styles (none/auto/clean/manual), user/group, stdout/stderr
paths, and debug settings. Disabled and KeepAlive match upstream defaults.
Default PID paths add the staged run directory to `destroot.keepdirs` once.
Upstream diagnostics for unknown PID styles or excess PID arguments remain
non-fatal. Global verbosity and stderr defaults follow upstream multi-item
inheritance. XML text is escaped and forbidden XML characters fail before writes.
New generated files use mode 0644 subject to the session mask; existing regular
files are truncated in place, retaining mode, ownership, and hard links. Existing
symlinks are rejected. Tests parse generated XML using macOS plutil, check exact
argument arrays and optional fields, and run generation through native destroot
finish, including its keepdir marker. Generation never starts the service.


Startup items with init/start/stop/restart fragments, or without a direct
executable, now generate the daemondo shell wrapper natively. Script fragments
are Tcl lists of shell lines. Default start/stop phases invoke the subport's
`${prefix}/etc/startup/${subport}.sh`; default restart calls Stop then Start.
The plist receives separate start, stop, and restart command arrays. The default
PID style remains exec when an executable was supplied and none otherwise.
Wrappers use mode 0755 subject to the staging mask and retain existing-file
metadata when regenerated. Prefix assignments and default script paths are
shell-quoted when needed. User shell fragments are preserved. Malformed lists,
NUL bytes, wrapper/plist filename collisions, and symlink destinations fail
explicitly. Tests execute harmless phase scripts, validate plist arguments, cover
custom/default restart and quoted prefixes, and verify that wrappers survive the
native destroot finish lifecycle. Staging itself does not execute the scripts.
