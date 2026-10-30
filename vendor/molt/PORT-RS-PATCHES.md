# Port-rs Molt changes

Source: the `molt` 0.3.1 crates.io release from
https://github.com/wduquette/molt/tree/f944b08aae5e0d6578e6c2796ea68d36c23c9e32/molt

The BSD-3-Clause license is preserved in LICENSE.txt, retrieved from that commit's
repository root (the crates.io package did not contain the license file).

Local changes:

- Host hooks for reads and changes to global scalar variables, following global
  aliases while keeping same-named local variables independent.
- Mutable scalar/variable reads so a lazy value can evaluate commands.
- Scoped global evaluation and data substitution for MacPorts defaults.
- Preserve the recursion budget on script parse errors and apply it to substitutions.
- Rustfmt formatting for the current toolchain.
- Propagate variable-read errors through append/list/dictionary/increment mutators.
- Checked integer increment and current-toolchain Clippy cleanups.
- Removal of obsolete unused imports and replacement of deprecated float constants.
- Native `string map`, including ordered nonrecursive replacements, literal script
  templates, empty keys, and `-nocase` matching, with standalone interpreter tests.
- `split` and `lrange`, plus end-relative and integer-arithmetic list indices;
  shared index parsing for `lindex` and `lrange`.

- Native `upvar` for scalar and whole-array aliases, including renamed variables,
  relative/absolute scope levels, alias chains, and retained links after unset.
  Array-element aliases remain explicitly unsupported. Global scalar hooks resolve
  the alias target name; mutable lookup no longer uses unsafe transmutation.

- Qualified global variable substitution (`$::name`, `${::build.jobs}`, qualified
  arrays), canonical namespace separators, global/upvar alias resolution, and
  root-variable enumeration. Reject qualified/array-element procedure parameters.
  Unsetting a missing array element no longer creates an invalid placeholder.
  Full namespace evaluation/import/command resolution remains unimplemented.

These hooks are host APIs, not a complete implementation of Tcl's `trace` command.
The upstream source and tests remain available to verify interpreter behavior.
