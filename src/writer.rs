use crate::{Error, Result, io::Output};

/// Writes to the shell's output and retains its first error.
///
/// The output is the last field, so `&mut ShellWriter<O>` coerces to
/// `&mut ShellWriter<dyn Output>` for handlers.
pub(crate) struct ShellWriter<O: ?Sized> {
    pub(super) error: Option<Error>,
    /// Display column after the last write, ignoring ANSI escape sequences.
    pub(super) column: usize,
    /// Writes `\n` as `\r\n`.
    pub(super) crlf: bool,
    last_was_cr: bool,
    escape: bool,
    csi: bool,
    output: O,
}

impl<O> ShellWriter<O> {
    /// Creates a writer with no pending output error.
    pub(crate) fn new(output: O) -> Self {
        Self {
            error: None,
            column: 0,
            crlf: false,
            last_was_cr: false,
            escape: false,
            csi: false,
            output,
        }
    }
}

impl<O: Output + ?Sized> ShellWriter<O> {
    /// Takes the original error so the run loop can stop before reading more input.
    pub(crate) fn take_error(&mut self) -> Result {
        self.error.take().map_or(Ok(()), Err)
    }

    fn write_crlf(&mut self, text: &str) -> Result {
        let mut rest = text;
        while let Some(index) = rest.find('\n') {
            let line = &rest[..index];
            let after_cr = line.ends_with('\r') || (line.is_empty() && self.last_was_cr);
            self.output.write(line)?;
            self.output.write(if after_cr { "\n" } else { "\r\n" })?;
            self.last_was_cr = false;
            rest = &rest[index + 1..];
        }
        if !rest.is_empty() {
            self.output.write(rest)?;
            self.last_was_cr = rest.ends_with('\r');
        }
        Ok(())
    }
}

impl<O: Output + ?Sized> core::fmt::Write for ShellWriter<O> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if self.error.is_some() {
            return Err(core::fmt::Error);
        }
        let result = if self.crlf {
            self.write_crlf(s)
        } else {
            self.output.write(s)
        };
        result.map_err(|error| {
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
