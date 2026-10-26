use core::{
    fmt::Write,
    ops::{Deref, DerefMut},
};

use crate::{
    CallbackResult, ErrorHandler, Prompt, Result, ShellContext, ShellState, SyncCommands,
    buffer::{Arguments, DefaultBuffer, LineBuffer},
    input::Input,
    state::{CTRL_D, ShellStorage},
};

/// An interactive shell with blocking input.
pub struct Shell<'a, R: CallbackResult = (), S = (), C = (), B = DefaultBuffer> {
    storage: ShellStorage<'a, R, S>,
    read: fn(&mut [u8]) -> Result<usize>,
    input: Input,
    line: B,
    commands: &'a C,
}

impl<'a, R: CallbackResult> Shell<'a, R> {
    /// Creates a shell with output and blocking input functions.
    ///
    /// `read` fills the supplied buffer and returns its byte count, or `Ok(0)` for EOF.
    pub fn new(write: fn(&str) -> R, read: fn(&mut [u8]) -> Result<usize>) -> Self {
        Self {
            storage: ShellStorage::new(write),
            read,
            input: Input::new(),
            line: DefaultBuffer::default(),
            commands: &(),
        }
    }
}

impl<'a, R: CallbackResult, S, C, B: LineBuffer> Shell<'a, R, S, C, B> {
    /// Attaches application data, available to handlers as `ctx.data`.
    /// Configure this first. Changing state resets the commands and hooks.
    pub fn with_state<T>(self, data: T) -> Shell<'a, R, T, (), B> {
        Shell {
            storage: self.storage.with_data(data),
            read: self.read,
            input: self.input,
            line: self.line,
            commands: &(),
        }
    }

    /// Sets the borrowed command collection, replacing any previous collection.
    pub fn with_commands<T: SyncCommands<R, S>>(self, commands: &'a T) -> Shell<'a, R, S, T, B> {
        let mut storage = self.storage;
        storage.help = commands;
        Shell {
            storage,
            read: self.read,
            input: self.input,
            line: self.line,
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
    pub fn with_buffer<T: LineBuffer>(self, buffer: T) -> Shell<'a, R, S, C, T> {
        Shell {
            storage: self.storage,
            read: self.read,
            input: self.input,
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

    /// Runs until EOF, Ctrl-D on an empty line, or an I/O error.
    ///
    /// Returns the first output error without writing again.
    pub fn run(&mut self) -> Result
    where
        C: SyncCommands<R, S>,
    {
        self.context().print_prompt();
        loop {
            self.storage.write.take_error()?;
            let Some(byte) = self.input.read(self.read)? else {
                return Ok(());
            };
            if byte == CTRL_D && self.line.as_str().is_empty() {
                let _ = writeln!(self.context());
                return self.storage.write.take_error();
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
        }
    }
}

impl<R: CallbackResult, S, C, B> Deref for Shell<'_, R, S, C, B> {
    type Target = ShellState<S>;
    fn deref(&self) -> &Self::Target {
        &self.storage.state
    }
}

impl<R: CallbackResult, S, C, B> DerefMut for Shell<'_, R, S, C, B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.storage.state
    }
}
