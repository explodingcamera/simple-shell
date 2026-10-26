use core::{
    fmt::{self, Write},
    ops::{Deref, DerefMut},
};

use crate::{
    CallbackResult, Commands, Error, ErrorHandler, Prompt, buffer::LineBuffer,
    handlers::DefaultHandlers, writer::ShellWriter,
};

pub(super) const CTRL_C: u8 = 3;
pub(super) const CTRL_D: u8 = 4;
const CTRL_H: u8 = 8;
const CTRL_L: u8 = 12;
const ENTER: u8 = 13;
const BACKSPACE: u8 = 127;
const ESCAPE: u8 = 27;
const ANSI_CLEAR_SCREEN: &str = "\x1b[2J\x1b[1;1H";

#[derive(Clone, Copy, Default)]
enum EscapeState {
    #[default]
    None,
    Start,
    Csi,
    Parameters,
    Ss3,
}

/// Owned application and editing state. Both shell types dereference to this state.
pub struct ShellState<S = ()> {
    /// Application data available to command handlers.
    pub data: S,
    pub(super) editor: Editor,
}

/// Line editing state, independent of the application data type.
///
/// The line itself is stored by the shell and passed to each editing method.
#[derive(Default)]
pub(super) struct Editor {
    /// Byte offset of the cursor at a UTF-8 character boundary in the current line.
    pub(super) cursor: usize,
    /// Byte offset where the current line starts after backslash continuations.
    line_start: usize,
    /// Display width of the last prompt, used to indent continuation lines.
    prompt_width: usize,
    /// Whether to suppress LF after a submitted CR.
    pub(super) last_was_cr: bool,
    /// Incomplete UTF-8 input, kept outside the valid command string.
    utf8: [u8; 4],
    /// Number of incomplete UTF-8 bytes.
    utf8_len: usize,
    /// Discard the rest of a line after storage overflow.
    discarding: bool,
    /// Partial terminal escape sequence, retained between input bytes.
    escape: EscapeState,
}

impl Editor {
    fn clear_line(&mut self, line: &mut dyn LineBuffer) {
        line.clear();
        self.cursor = 0;
        self.line_start = 0;
    }
}

/// Borrowed command context with application state and formatted output.
///
/// Dereferences to [`ShellState`], so application data is available as `ctx.data`.
pub struct ShellContext<'a, R: CallbackResult = (), S = ()> {
    state: &'a mut ShellState<S>,
    write: &'a mut ShellWriter<R>,
    prompt: &'a dyn Prompt<S, R>,
    help: &'a dyn Commands<R, S>,
    error_handler: &'a dyn ErrorHandler<S, R>,
}

/// Owned storage shared by both shells, lent to handlers through `ShellContext`.
pub(super) struct ShellStorage<'a, R: CallbackResult, S> {
    pub(super) state: ShellState<S>,
    pub(super) write: ShellWriter<R>,
    /// Callback that draws the prompt before each input line.
    pub(super) prompt: &'a dyn Prompt<S, R>,
    /// Help output for the configured command collection.
    pub(super) help: &'a dyn Commands<R, S>,
    /// Callback for command failures and unmatched input.
    pub(super) error_handler: &'a dyn ErrorHandler<S, R>,
}

impl<R: CallbackResult, S> fmt::Write for ShellContext<'_, R, S> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.write.write_str(text)
    }
}

impl<'a, R: CallbackResult> ShellStorage<'a, R, ()> {
    /// Creates empty editing state with an output callback.
    pub(super) fn new(write: fn(&str) -> R) -> Self {
        Self {
            state: ShellState {
                data: (),
                editor: Editor::default(),
            },
            write: ShellWriter::new(write),
            prompt: &DefaultHandlers,
            help: &(),
            error_handler: &DefaultHandlers,
        }
    }
}

impl<'a, R: CallbackResult, S> ShellStorage<'a, R, S> {
    /// Changes the application data without changing the editor or output.
    pub(super) fn with_data<T>(self, data: T) -> ShellStorage<'a, R, T> {
        ShellStorage {
            state: ShellState {
                data,
                editor: self.state.editor,
            },
            write: self.write,
            prompt: &DefaultHandlers,
            help: &(),
            error_handler: &DefaultHandlers,
        }
    }

    /// Borrows editor state and output for a handler.
    pub(super) fn context(&mut self) -> ShellContext<'_, R, S> {
        ShellContext {
            state: &mut self.state,
            write: &mut self.write,
            prompt: self.prompt,
            help: self.help,
            error_handler: self.error_handler,
        }
    }
}

impl<R: CallbackResult, S> Deref for ShellContext<'_, R, S> {
    type Target = ShellState<S>;
    fn deref(&self) -> &Self::Target {
        self.state
    }
}

impl<R: CallbackResult, S> DerefMut for ShellContext<'_, R, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.state
    }
}

impl<R: CallbackResult, S> ShellContext<'_, R, S> {
    /// Handles an input byte, including partial escape and UTF-8 sequences.
    /// Returns `true` when `line` holds a submitted command.
    pub(super) fn handle_byte(&mut self, line: &mut dyn LineBuffer, b: u8) -> bool {
        let skip_lf = self.editor.last_was_cr && b == b'\n';
        self.editor.last_was_cr = b == ENTER;
        if skip_lf {
            return false;
        }
        if self.editor.discarding && b != CTRL_C {
            if b == ENTER || b == b'\n' {
                self.editor.discarding = false;
                self.editor.clear_line(line);
                let _ = writeln!(self.write);
                self.report_error("", &Error::LineTooLong);
                self.print_prompt();
            }
            return false;
        }
        if b == ESCAPE {
            self.editor.escape = EscapeState::Start;
            self.editor.utf8_len = 0;
            return false;
        }
        if b.is_ascii_control() {
            self.editor.escape = EscapeState::None;
        }
        match self.editor.escape {
            EscapeState::None => {}
            EscapeState::Start => {
                self.editor.escape = match b {
                    b'[' => EscapeState::Csi,
                    b'O' => EscapeState::Ss3,
                    _ => EscapeState::None,
                };
                return false;
            }
            EscapeState::Csi | EscapeState::Parameters => {
                if (0x20..=0x3f).contains(&b) {
                    self.editor.escape = EscapeState::Parameters;
                } else {
                    let simple = matches!(self.editor.escape, EscapeState::Csi);
                    self.editor.escape = EscapeState::None;
                    if simple && (0x40..=0x7e).contains(&b) {
                        self.handle_csi_key(line, b);
                    }
                }
                return false;
            }
            EscapeState::Ss3 => {
                self.editor.escape = EscapeState::None;
                self.handle_csi_key(line, b);
                return false;
            }
        }
        if !b.is_ascii() {
            let editor = &mut self.state.editor;
            if (0xc2..=0xf4).contains(&b) {
                editor.utf8[0] = b;
                editor.utf8_len = 1;
            } else if (0x80..=0xbf).contains(&b) && editor.utf8_len > 0 {
                editor.utf8[editor.utf8_len] = b;
                editor.utf8_len += 1;
                match core::str::from_utf8(&editor.utf8[..editor.utf8_len]) {
                    Ok(text) => {
                        let character = text.chars().next();
                        editor.utf8_len = 0;
                        if let Some(character) = character
                            && !character.is_control()
                        {
                            self.insert_char(line, character);
                        }
                    }
                    Err(error) if error.error_len().is_some() || editor.utf8_len == 4 => {
                        editor.utf8_len = 0;
                    }
                    Err(_) => {}
                }
            } else {
                editor.utf8_len = 0;
            }
            return false;
        }
        self.editor.utf8_len = 0;
        match b {
            CTRL_C => {
                self.interrupt(line);
                self.print_prompt();
            }
            CTRL_L => self.handle_clear(line),
            ENTER | b'\n' if line.as_str().ends_with('\\') => self.continue_line(line),
            ENTER | b'\n' => {
                let _ = writeln!(self.write);
                return true;
            }
            BACKSPACE | CTRL_H => self.handle_backspace(line),
            b' '..=b'~' => self.insert_char(line, b as char),
            _ => {}
        }
        false
    }

    fn insert_char(&mut self, line: &mut dyn LineBuffer, character: char) {
        let at_end = self.editor.cursor == line.as_str().len();
        if let Err(error) = line.insert_char(self.editor.cursor, character) {
            if matches!(error, Error::LineTooLong) {
                self.editor.discarding = true;
            } else {
                self.report_error("", &error);
            }
            return;
        }
        self.editor.cursor += character.len_utf8();
        if at_end {
            let _ = write!(self.write, "{}", character);
        } else {
            self.redraw_line(line);
        }
    }

    fn redraw_line(&mut self, line: &dyn LineBuffer) {
        let _ = self.write.write_str("\r\x1b[2K");
        if self.editor.line_start == 0 {
            self.print_prompt();
        } else {
            self.indent_continuation();
        }
        let (editor, line) = (&self.state.editor, line.as_str());
        let _ = self
            .write
            .write_str(&line[editor.line_start..editor.cursor]);
        // Let the terminal determine the prefix's display width.
        let _ = self.write.write_str("\x1b7");
        let _ = self.write.write_str(&line[editor.cursor..]);
        let _ = self.write.write_str("\x1b8");
    }

    fn handle_clear(&mut self, line: &dyn LineBuffer) {
        self.clear_screen();
        self.redraw_line(line);
    }

    fn handle_backspace(&mut self, line: &mut dyn LineBuffer) {
        if self.editor.cursor > self.editor.line_start
            && let Some(character) = line.as_str()[..self.editor.cursor].chars().next_back()
        {
            self.editor.cursor -= character.len_utf8();
            line.remove_char(self.editor.cursor);
            self.redraw_line(line);
        }
    }

    /// Joins the next line after a trailing backslash, like POSIX shells.
    fn continue_line(&mut self, line: &mut dyn LineBuffer) {
        let editor = &mut self.state.editor;
        line.remove_char(line.as_str().len() - 1);
        editor.line_start = line.as_str().len();
        editor.cursor = editor.line_start;
        let _ = writeln!(self.write);
        self.indent_continuation();
    }

    /// Indents continuation lines past the prompt, like fish.
    fn indent_continuation(&mut self) {
        let _ = write!(self.write, "{:1$}", "", self.editor.prompt_width + 4);
    }

    /// Clears a submitted line after command execution.
    pub(super) fn finish_line(&mut self, line: &mut dyn LineBuffer) {
        self.editor.clear_line(line);
    }

    /// Discards partial input and displays a keyboard interrupt.
    pub(super) fn interrupt(&mut self, line: &mut dyn LineBuffer) {
        self.editor.clear_line(line);
        self.editor.last_was_cr = false;
        self.editor.utf8_len = 0;
        self.editor.discarding = false;
        self.editor.escape = EscapeState::None;
        let _ = writeln!(self.write, "^C");
    }

    fn handle_csi_key(&mut self, line: &dyn LineBuffer, b: u8) {
        match b {
            b'C' => {
                if let Some(character) = line.as_str()[self.editor.cursor..].chars().next() {
                    self.editor.cursor += character.len_utf8();
                    self.redraw_line(line);
                }
            }
            b'D' if self.editor.cursor > self.editor.line_start
                && let Some(character) =
                    line.as_str()[..self.editor.cursor].chars().next_back() =>
            {
                self.editor.cursor -= character.len_utf8();
                self.redraw_line(line);
            }
            _ => {}
        }
    }

    /// Prints the registered commands, their descriptions, and aliases.
    pub fn print_help_screen(&mut self) {
        let _ = writeln!(self, "available commands:");
        let help = self.help;
        let _ = help.write_help(self);
    }

    /// Prints the configured prompt.
    pub fn print_prompt(&mut self) {
        let prompt = self.prompt;
        prompt.write_prompt(self);
        self.editor.prompt_width = self.write.column;
    }

    /// Reports a command failure using the configured error handler.
    pub fn report_error(&mut self, name: &str, error: &Error) {
        // A formatting error caused by failed output belongs to run(), not this hook.
        if self.write.error.is_some() {
            return;
        }
        let handler = self.error_handler;
        handler.write_error(name, error, self);
    }

    /// Sends ANSI escape sequences to clear the screen and move the cursor home.
    pub fn clear_screen(&mut self) {
        let _ = self.write_str(ANSI_CLEAR_SCREEN);
    }
}
