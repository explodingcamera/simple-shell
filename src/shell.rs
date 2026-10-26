use core::fmt::Write;

#[cfg(feature = "embedded-io")]
use crate::io::EmbeddedIo;
#[cfg(feature = "std")]
use crate::io::StdIo;
use crate::{
    CallbackResult, Result, SyncCommands,
    buffer::{Arguments, DefaultBuffer, LineBuffer},
    io::{Input, InputBuffer, Output},
    macros::impl_shell,
    state::{CTRL_D, ShellStorage},
};

/// An interactive shell with blocking input.
pub struct Shell<'a, O, I, S = (), C = (), B = DefaultBuffer> {
    storage: ShellStorage<'a, S, O>,
    input: I,
    buffer: InputBuffer,
    line: B,
    commands: &'a C,
}

impl<'a> Shell<'a, (), ()> {
    /// Creates a shell with output and blocking input functions.
    ///
    /// Both can be closures that capture state. `read` fills the supplied buffer and
    /// returns its byte count, or `Ok(0)` at the end of input.
    pub fn new<W, R, F>(write: W, read: F) -> Shell<'a, W, F>
    where
        W: FnMut(&str) -> R + Send,
        R: CallbackResult,
        F: FnMut(&mut [u8]) -> Result<usize>,
    {
        Shell::with_io(write, read)
    }

    /// Creates a shell from a [`std::io::Write`] writer and a [`std::io::Read`] reader.
    #[cfg(feature = "std")]
    pub fn new_std<W: std::io::Write + Send, R: std::io::Read>(
        writer: W,
        reader: R,
    ) -> Shell<'a, StdIo<W>, StdIo<R>> {
        Shell::with_io(StdIo(writer), StdIo(reader))
    }

    /// Creates a shell from an [`embedded_io::Write`] writer and an
    /// [`embedded_io::Read`] reader, such as the halves of a UART.
    #[cfg(feature = "embedded-io")]
    pub fn new_embedded<W: embedded_io::Write + Send, R: embedded_io::Read>(
        writer: W,
        reader: R,
    ) -> Shell<'a, EmbeddedIo<W>, EmbeddedIo<R>> {
        Shell::with_io(EmbeddedIo(writer), EmbeddedIo(reader))
    }
}

impl<'a, O: Output + Send, I: Input> Shell<'a, O, I> {
    fn with_io(output: O, input: I) -> Self {
        Self {
            storage: ShellStorage::new(output),
            input,
            buffer: InputBuffer::new(),
            line: DefaultBuffer::default(),
            commands: &(),
        }
    }
}

impl_shell!(Shell, Input, SyncCommands);

impl<O: Output + Send, I: Input, S, C: SyncCommands<S>, B: LineBuffer> Shell<'_, O, I, S, C, B> {
    /// Processes one input byte and runs the command when a line is submitted.
    ///
    /// Use it to drive the shell from a main loop or an interrupt handler instead of
    /// [`Self::run`]. The shell's input is unused, so create it with a placeholder
    /// such as `|_| Ok(0)`. Print the first prompt with `shell.context().print_prompt()`.
    /// Returns `Ok(false)` after Ctrl-D on an empty line.
    pub fn feed(&mut self, byte: u8) -> Result<bool> {
        self.storage.write.take_error()?;
        if byte == CTRL_D && self.line.as_str().is_empty() && !self.storage.state.editor.ignore_eof
        {
            let _ = writeln!(self.context());
            self.storage.write.take_error()?;
            return Ok(false);
        }
        if self.storage.context().handle_byte(&mut self.line, byte) {
            self.storage.write.take_error()?;
            let mut ctx = self.storage.context();
            match Arguments::parse(self.line.as_str()) {
                Ok(argv) => {
                    if let Some(name) = argv.first()
                        && let Err(error) = self.commands.run_sync(&argv, &mut ctx)
                    {
                        ctx.report_error(name, &error);
                    }
                }
                Err(error) => ctx.report_error("", &error),
            }
            ctx.finish_line(&mut self.line);
            ctx.print_prompt();
        }
        self.storage.write.take_error()?;
        Ok(true)
    }

    /// Runs until the end of input, Ctrl-D on an empty line, or an I/O error.
    ///
    /// Returns the first output error without writing again.
    pub fn run(&mut self) -> Result {
        self.context().print_prompt();
        loop {
            self.storage.write.take_error()?;
            let Some(byte) = self.buffer.read(&mut self.input)? else {
                return Ok(());
            };
            if !self.feed(byte)? {
                return Ok(());
            }
        }
    }
}
