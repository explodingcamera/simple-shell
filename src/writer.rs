use crate::{CallbackResult, Error, Result};

/// Calls the output adapter and retains its first error.
pub(crate) struct ShellWriter<R: CallbackResult> {
    print: fn(&str) -> R,
    pub(super) error: Option<Error>,
    /// Display column after the last write, ignoring ANSI escape sequences.
    pub(super) column: usize,
    escape: bool,
    csi: bool,
}

impl<R: CallbackResult> core::fmt::Write for ShellWriter<R> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if self.error.is_some() {
            return Err(core::fmt::Error);
        }
        (self.print)(s).into_result().map_err(|error| {
            self.error = Some(error);
            core::fmt::Error
        })?;
        for character in s.chars() {
            if self.csi {
                self.csi = !('@'..='~').contains(&character);
            } else if self.escape {
                self.escape = false;
                self.csi = character == '[';
            } else {
                match character {
                    '\x1b' => self.escape = true,
                    '\r' | '\n' => self.column = 0,
                    c if !c.is_control() => self.column += 1,
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

impl<R: CallbackResult> ShellWriter<R> {
    /// Creates a writer with no pending output error.
    pub fn new(print: fn(&str) -> R) -> Self {
        Self {
            print,
            error: None,
            column: 0,
            escape: false,
            csi: false,
        }
    }

    /// Takes the original error so the run loop can stop before reading more input.
    pub fn take_error(&mut self) -> Result {
        self.error.take().map_or(Ok(()), Err)
    }
}
