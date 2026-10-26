# simple-shell

[![crates.io](https://img.shields.io/crates/v/simple-shell.svg)](https://crates.io/crates/simple-shell) [![docs.rs](https://img.shields.io/docsrs/simple-shell)](https://docs.rs/simple-shell)

A dependency-free `no_std` shell for embedded systems, with blocking and async input.
It was originally developed as part of [rust-os-blog](https://github.com/explodingcamera/rust-os-blog).

```rust,ignore
use core::fmt::Write;
use simple_shell::{Shell, commands};

let commands = commands! {
    /// Show available commands.
    "help" | "?" => |_, shell| shell.print_help_screen(),
    /// Print the given arguments.
    "echo" => |argv, shell| writeln!(shell, "{}", argv[1..].join(" ")),
};

Shell::new(write, read).with_commands(&commands).run()?;
```

See the [blocking](examples/blocking.rs) and [async](examples/async.rs) examples for complete programs.

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT license](./LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in `simple-shell` by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
