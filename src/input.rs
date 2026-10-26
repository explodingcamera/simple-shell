use core::task::{Context, Poll};

use crate::{Error, Result};

/// Buffered input shared by the blocking and asynchronous shells.
pub(super) struct Input {
    bytes: [u8; 64],
    start: usize,
    end: usize,
}

impl Input {
    /// Creates an empty read buffer.
    pub(super) fn new() -> Self {
        Self {
            bytes: [0; 64],
            start: 0,
            end: 0,
        }
    }

    fn next(&mut self) -> Option<u8> {
        if self.start == self.end {
            return None;
        }
        let byte = self.bytes[self.start];
        self.start += 1;
        Some(byte)
    }

    fn filled(&mut self, length: usize) -> Result<Option<u8>> {
        if length > self.bytes.len() {
            return Err(Error::Message(
                "input callback returned an invalid byte count",
            ));
        }
        self.start = 0;
        self.end = length;
        Ok(self.next())
    }

    /// Returns a buffered byte, refilling synchronously when necessary.
    pub(super) fn read(&mut self, read: fn(&mut [u8]) -> Result<usize>) -> Result<Option<u8>> {
        if let Some(byte) = self.next() {
            return Ok(Some(byte));
        }
        let length = read(&mut self.bytes)?;
        self.filled(length)
    }

    /// Returns a buffered byte or polls for another batch. `None` means EOF.
    pub(super) fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        read: fn(&mut Context<'_>, &mut [u8]) -> Poll<Result<usize>>,
    ) -> Poll<Result<Option<u8>>> {
        if let Some(byte) = self.next() {
            return Poll::Ready(Ok(Some(byte)));
        }
        read(cx, &mut self.bytes).map(|length| self.filled(length?))
    }
}
