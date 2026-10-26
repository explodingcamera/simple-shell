use core::{
    fmt::Write,
    future::poll_fn,
    ops::{Deref, DerefMut},
    pin::pin,
    task::{Context, Poll},
};

use crate::{
    CallbackResult, Commands, ErrorHandler, Prompt, Result, ShellContext, ShellState,
    buffer::{Arguments, DefaultBuffer, LineBuffer},
    input::Input,
    state::{CTRL_C, CTRL_D, ShellStorage},
};

const INPUT_BUDGET: usize = 64;

enum CommandOutcome {
    Finished(Result),
    Interrupted,
    EndOfInput,
}

/// An interactive shell with wake-driven, buffered async input.
///
/// Before returning [`Poll::Pending`], the input callback must register the waker
/// and arrange to wake it when input becomes available, without losing notifications.
pub struct AsyncShell<'a, R: CallbackResult = (), S = (), C = (), B = DefaultBuffer> {
    storage: ShellStorage<'a, R, S>,
    poll_read: fn(&mut Context<'_>, &mut [u8]) -> Poll<Result<usize>>,
    input: Input,
    line: B,
    remaining_reads: usize,
    commands: &'a C,
}

impl<'a, R: CallbackResult> AsyncShell<'a, R> {
    /// Creates a shell with output and polling input functions.
    /// Input fills the supplied buffer and returns its byte count, or `Ok(0)` for EOF.
    pub fn new(
        write: fn(&str) -> R,
        poll_read: fn(&mut Context<'_>, &mut [u8]) -> Poll<Result<usize>>,
    ) -> Self {
        Self {
            storage: ShellStorage::new(write),
            poll_read,
            input: Input::new(),
            line: DefaultBuffer::default(),
            remaining_reads: INPUT_BUDGET,
            commands: &(),
        }
    }
}

impl<'a, R: CallbackResult, S, C, B: LineBuffer> AsyncShell<'a, R, S, C, B> {
    /// Attaches application data, available to handlers as `ctx.data`.
    /// Configure this first. Changing state resets the commands and hooks.
    pub fn with_state<T>(self, data: T) -> AsyncShell<'a, R, T, (), B> {
        AsyncShell {
            storage: self.storage.with_data(data),
            poll_read: self.poll_read,
            input: self.input,
            line: self.line,
            remaining_reads: self.remaining_reads,
            commands: &(),
        }
    }

    /// Sets the borrowed command collection, replacing any previous collection.
    pub fn with_commands<T: Commands<R, S>>(self, commands: &'a T) -> AsyncShell<'a, R, S, T, B> {
        let mut storage = self.storage;
        storage.help = commands;
        AsyncShell {
            storage,
            poll_read: self.poll_read,
            input: self.input,
            line: self.line,
            remaining_reads: self.remaining_reads,
            commands,
        }
    }

    /// Sets the prompt writer. The default writes `"> "`.
    pub fn with_prompt<P: Prompt<S, R>>(mut self, prompt: &'a P) -> Self {
        self.storage.prompt = prompt;
        self
    }

    /// Sets the handler for command errors and unmatched input.
    pub fn with_error_handler<H: ErrorHandler<S, R>>(mut self, handler: &'a H) -> Self {
        self.storage.error_handler = handler;
        self
    }

    /// Replaces the command line storage, for example with a `heapless::String<N>`.
    ///
    /// Lines that do not fit in fixed-size storage are rejected.
    pub fn with_buffer<T: LineBuffer>(self, buffer: T) -> AsyncShell<'a, R, S, C, T> {
        AsyncShell {
            storage: self.storage,
            poll_read: self.poll_read,
            input: self.input,
            remaining_reads: self.remaining_reads,
            line: buffer,
            commands: self.commands,
        }
    }

    /// Returns the line being edited.
    pub fn command(&self) -> &str {
        self.line.as_str()
    }

    /// Returns the cursor's byte offset in [`Self::command`], always at a character boundary.
    pub fn cursor(&self) -> usize {
        self.storage.state.editor.cursor
    }

    /// Borrows the shell's application state and formatted output.
    pub fn context(&mut self) -> ShellContext<'_, R, S> {
        self.storage.context()
    }

    async fn read_byte(&mut self) -> Result<Option<u8>> {
        poll_fn(|cx| {
            if self.remaining_reads == 0 {
                self.remaining_reads = INPUT_BUDGET;
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            let byte = self.input.poll_read(cx, self.poll_read);
            match byte {
                Poll::Ready(_) => self.remaining_reads -= 1,
                Poll::Pending => self.remaining_reads = INPUT_BUDGET,
            }
            byte
        })
        .await
    }

    /// Runs until EOF, Ctrl-D on an empty line, an I/O error, or the future is dropped.
    ///
    /// Input is processed in bounded batches to let other tasks run. EOF drops a
    /// pending command, and Ctrl-C drops it and resumes editing. Other input
    /// received while it runs is discarded. Synchronous commands cannot be cancelled.
    pub async fn run(&mut self) -> Result
    where
        C: Commands<R, S>,
    {
        self.context().print_prompt();
        loop {
            self.storage.write.take_error()?;
            let Some(byte) = self.read_byte().await? else {
                return Ok(());
            };
            if byte == CTRL_D && self.line.as_str().is_empty() {
                let _ = writeln!(self.context());
                return self.storage.write.take_error();
            }
            if self.storage.context().handle_byte(&mut self.line, byte) {
                self.storage.write.take_error()?;
                let keep_running = self.run_command().await;
                self.storage.context().finish_line(&mut self.line);
                if !keep_running? {
                    return Ok(());
                }
                self.context().print_prompt();
            }
        }
    }

    async fn run_command(&mut self) -> Result<bool>
    where
        C: Commands<R, S>,
    {
        let argv = match Arguments::parse(self.line.as_str()) {
            Ok(argv) => argv,
            Err(error) => {
                self.context().report_error("", &error);
                return Ok(true);
            }
        };
        let Some(name) = argv.first() else {
            return Ok(true);
        };
        let mut last_was_cr = self.storage.state.editor.last_was_cr;
        let outcome = {
            let mut ctx = self.storage.context();
            let mut future = pin!(self.commands.run(&argv, &mut ctx));
            poll_fn(|cx| {
                if let Poll::Ready(result) = future.as_mut().poll(cx) {
                    return Poll::Ready(Ok(CommandOutcome::Finished(result)));
                }
                for _ in 0..INPUT_BUDGET {
                    match self.input.poll_read(cx, self.poll_read) {
                        Poll::Ready(Ok(Some(CTRL_C))) => {
                            return Poll::Ready(Ok(CommandOutcome::Interrupted));
                        }
                        Poll::Ready(Ok(Some(_))) => last_was_cr = false,
                        Poll::Ready(Ok(None)) => {
                            return Poll::Ready(Ok(CommandOutcome::EndOfInput));
                        }
                        Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
                cx.waker().wake_by_ref();
                Poll::Pending
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

impl<R: CallbackResult, S, C, B> Deref for AsyncShell<'_, R, S, C, B> {
    type Target = ShellState<S>;
    fn deref(&self) -> &Self::Target {
        &self.storage.state
    }
}

impl<R: CallbackResult, S, C, B> DerefMut for AsyncShell<'_, R, S, C, B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.storage.state
    }
}
