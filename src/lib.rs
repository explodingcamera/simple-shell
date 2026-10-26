//! A simple `no_std` shell for embedded systems.
//!
//! Pass [`Shell::new`] functions for writing text and reading bytes, then
//! register commands with [`Shell::with_commands`] and call [`Shell::run`].
//! For async input, use [`AsyncShell`].
//!
//! # Example
//!
//! ```no_run
//! use simple_shell::{Shell, commands};
//!
//! use std::io::{self, Read, Write};
//!
//! fn write(text: &str) {
//!     print!("{text}");
//!     io::stdout().flush().expect("failed to flush stdout");
//! }
//!
//! fn read(buffer: &mut [u8]) -> simple_shell::Result<usize> {
//!     io::stdin().read(buffer).map_err(|_| "stdin read failed".into())
//! }
//!
//! fn main() {
//!     let commands = commands! {
//!         /// Show available commands.
//!         "help" | "?" | "h" => |_, shell| shell.print_help_screen(),
//!         /// Exit the shell.
//!         "exit" => |_, _| -> () { std::process::exit(0) },
//!     };
//!     let mut shell = Shell::new(write, read).with_commands(&commands);
//!
//!     shell.run().expect("shell output failed");
//! }
//! ```
//!
//! # Features
//!
//! - `alloc` (default): growable command and argument storage. Without it, commands
//!   are limited to 64 arguments, including the command name.
//! - `heapless`: fixed-size command storage for targets without `alloc`. Lines are
//!   limited to 256 bytes by default. Pass a larger buffer with
//!   `with_buffer(heapless::String::<1024>::new())`.
//! - `std`: conversions from `std::io::Error`.
//!
//! With neither `alloc` nor `heapless`, implement [`LineBuffer`] for your own storage
//! and pass it to `with_buffer`.
//!
//! A line ending in `\` continues on the next line, indented past the prompt.
#![no_std]
#![forbid(unsafe_code)]
#[cfg(feature = "alloc")]
extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod async_shell;
mod buffer;
mod command;
mod error;
mod handlers;
mod input;
mod macros;
mod shell;
mod state;
mod writer;
pub use async_shell::AsyncShell;
pub use buffer::{DefaultBuffer, LineBuffer};
pub use command::{
    Async, AsyncCommand, Command, CommandInfo, CommandList, Commands, ShellCommand, SyncCommands,
};
pub use error::{Error, Result};
pub use handlers::{ErrorHandler, Prompt};
pub use shell::Shell;
pub use state::{ShellContext, ShellState};

/// Converts a callback's return value into a result.
///
/// Callbacks can return `()` for success or `Result<(), E>` for fallible work.
pub trait CallbackResult {
    /// Converts the callback's return value into a result.
    fn into_result(self) -> Result;
}

impl CallbackResult for () {
    fn into_result(self) -> Result {
        Ok(())
    }
}

impl<E: Into<Error>> CallbackResult for core::result::Result<(), E> {
    fn into_result(self) -> Result {
        self.map_err(Into::into)
    }
}
