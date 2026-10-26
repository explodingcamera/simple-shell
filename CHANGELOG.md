# Changelog

## Unreleased

- Rewrote the API around the new `commands!` macro.
- Added `AsyncShell`, which reads input asynchronously and runs async commands. Ctrl-C cancels a running async command.
- Added custom shell state, custom prompts, error handlers, fallible I/O, quoted arguments, and options to disable echo or ignore Ctrl-D.
- Added a `heapless` feature and the `LineBuffer` trait for fixed-size command storage without `alloc`.
- Added `new_std` for `std::io` types, an `embedded-io` feature with `new_embedded`, and support for closures that capture state as input and output.
- Added `Shell::feed`, `AsyncShell::next`, and `print_above` for driving the shell from your own loop and printing logs while someone types.
- Added an option for `\r\n` line endings.
- Moved the crate out of rust-os-blog into its own repository.

## 0.1.0 - 2026-10-18

- Document the API and add a working example and README badges.
- Accept iterators of commands in `with_commands`.
