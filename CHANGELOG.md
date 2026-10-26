# Changelog

## Unreleased

- Rewrote the API around the new `commands!` macro.
- Added `AsyncShell`, which reads input asynchronously and runs async commands. Ctrl-C cancels a running async command.
- Added custom shell state, custom prompts, error handlers, and fallible I/O.
- Added a `heapless` feature and the `LineBuffer` trait for fixed-size command storage without `alloc`.
- Moved the crate out of rust-os-blog into its own repository.

## 0.1.0 - 2026-10-18

- Document the API and add a working example and README badges.
- Accept iterators of commands in `with_commands`.
