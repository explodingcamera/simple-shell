use core::{
    fmt::Write,
    future::poll_fn,
    pin::pin,
    task::{Context, Poll},
};

#[cfg(feature = "embedded-io")]
use crate::io::EmbeddedIo;
use crate::{
    CallbackResult, Commands, Result,
    buffer::{Arguments, DefaultBuffer, LineBuffer},
    io::{AsyncInput, InputBuffer, Output},
    macros::impl_shell,
    state::{CTRL_C, CTRL_D, ShellStorage},
};

enum CommandOutcome {
    Finished(Result),
    Interrupted,
    EndOfInput,
}

/// An interactive shell with async input and cancellable async commands.
pub struct AsyncShell<'a, O, I, S = (), C = (), B = DefaultBuffer> {
    storage: ShellStorage<'a, S, O>,
    input: I,
    buffer: InputBuffer,
    line: B,
    commands: &'a C,
}

impl<'a> AsyncShell<'a, (), ()> {
    /// Creates a shell with an output function and a polling input function.
    ///
    /// Both can be closures that capture state. `poll_read` fills the supplied buffer
    /// and returns its byte count, or `Ok(0)` at the end of input. Before returning
    /// [`Poll::Pending`], it must register the waker and wake it when input arrives.
    pub fn new<W, R, F>(write: W, poll_read: F) -> AsyncShell<'a, W, F>
    where
        W: FnMut(&str) -> R + Send,
        R: CallbackResult,
        F: FnMut(&mut Context<'_>, &mut [u8]) -> Poll<Result<usize>>,
    {
        AsyncShell::with_io(write, poll_read)
    }

    /// Creates a shell from an [`embedded_io::Write`] writer and an
    /// [`embedded_io_async::Read`] reader, such as the halves of a UART.
    #[cfg(feature = "embedded-io")]
    pub fn new_embedded<W: embedded_io::Write + Send, R: embedded_io_async::Read>(
        writer: W,
        reader: R,
    ) -> AsyncShell<'a, EmbeddedIo<W>, EmbeddedIo<R>> {
        AsyncShell::with_io(EmbeddedIo(writer), EmbeddedIo(reader))
    }
}

impl<'a, O: Output + Send, I: AsyncInput> AsyncShell<'a, O, I> {
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

impl_shell!(AsyncShell, AsyncInput, Commands);

impl<O: Output + Send, I: AsyncInput, S, C: Commands<S>, B: LineBuffer>
    AsyncShell<'_, O, I, S, C, B>
{
    /// Processes the next input byte and runs the command when a line is submitted.
    ///
    /// Use it instead of [`Self::run`] to handle other events between bytes, for
    /// example by selecting it against a log channel and calling [`Self::print_above`].
    /// Print the first prompt with `shell.context().print_prompt()`. Returns
    /// `Ok(false)` at the end of input or after Ctrl-D on an empty line.
    ///
    /// Dropping the future while it waits for input is safe if the input is
    /// cancel-safe. Dropping it while a command runs cancels the command, like Ctrl-C.
    pub async fn next(&mut self) -> Result<bool> {
        self.storage.write.take_error()?;
        let editor = &mut self.storage.state.editor;
        if editor.command_running {
            editor.command_running = false;
            self.storage.context().interrupt(&mut self.line);
            self.context().print_prompt();
        }
        let Some(byte) = self.buffer.read_async(&mut self.input).await? else {
            return Ok(false);
        };
        if byte == CTRL_D && self.line.as_str().is_empty() && !self.storage.state.editor.ignore_eof
        {
            let _ = writeln!(self.context());
            self.storage.write.take_error()?;
            return Ok(false);
        }
        if self.storage.context().handle_byte(&mut self.line, byte) {
            self.storage.write.take_error()?;
            self.storage.state.editor.command_running = true;
            let keep_running = self.run_command().await;
            self.storage.state.editor.command_running = false;
            self.storage.context().finish_line(&mut self.line);
            if !keep_running? {
                return Ok(false);
            }
            self.context().print_prompt();
        }
        self.storage.write.take_error()?;
        Ok(true)
    }

    /// Runs until the end of input, Ctrl-D on an empty line, an I/O error, or the
    /// future is dropped.
    ///
    /// Input is processed in bounded batches to let other tasks run. Ctrl-C cancels a
    /// running async command and the end of input drops it. Other input received
    /// while a command runs is discarded. Synchronous commands cannot be cancelled.
    pub async fn run(&mut self) -> Result {
        self.context().print_prompt();
        while self.next().await? {}
        Ok(())
    }

    async fn run_command(&mut self) -> Result<bool> {
        let argv = match Arguments::parse(self.line.as_str()) {
            Ok(argv) => argv,
            Err(error) => {
                self.storage.context().report_error("", &error);
                return Ok(true);
            }
        };
        let Some(name) = argv.first() else {
            return Ok(true);
        };
        let mut last_was_cr = self.storage.state.editor.last_was_cr;
        let outcome = {
            let mut ctx = self.storage.context();
            let mut command = pin!(self.commands.run(&argv, &mut ctx));
            let mut watch = pin!(watch_input(
                &mut self.buffer,
                &mut self.input,
                &mut last_was_cr
            ));
            poll_fn(|cx| {
                if let Poll::Ready(result) = command.as_mut().poll(cx) {
                    return Poll::Ready(Ok(CommandOutcome::Finished(result)));
                }
                watch.as_mut().poll(cx)
            })
            .await
        };
        self.storage.state.editor.last_was_cr = last_was_cr;
        self.storage.write.take_error()?;
        match outcome? {
            CommandOutcome::Interrupted => self.storage.context().interrupt(&mut self.line),
            CommandOutcome::Finished(Err(error)) => {
                self.storage.context().report_error(name, &error)
            }
            CommandOutcome::EndOfInput => return Ok(false),
            CommandOutcome::Finished(Ok(())) => {}
        }
        Ok(true)
    }
}

/// Reads input while a command runs, until Ctrl-C or the end of input.
async fn watch_input(
    buffer: &mut InputBuffer,
    input: &mut impl AsyncInput,
    last_was_cr: &mut bool,
) -> Result<CommandOutcome> {
    loop {
        match buffer.read_async(input).await? {
            Some(CTRL_C) => return Ok(CommandOutcome::Interrupted),
            Some(_) => *last_was_cr = false,
            None => return Ok(CommandOutcome::EndOfInput),
        }
    }
}
