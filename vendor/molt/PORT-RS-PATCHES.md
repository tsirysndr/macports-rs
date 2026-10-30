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

These hooks are host APIs, not a complete implementation of Tcl's `trace` command.
The upstream source and tests remain available to verify interpreter behavior.
