use core::{
    future::{Future, poll_fn},
    task::{Context, Poll},
};

use crate::{CallbackResult, Error, Result};

/// Destination for shell output.
///
/// Implemented for closures that return `()` or a `Result`, and for the I/O
/// adapters used by the `new_std` and `new_embedded` constructors.
pub trait Output {
    /// Writes all of `text`.
    fn write(&mut self, text: &str) -> Result;
}

impl<F: FnMut(&str) -> R, R: CallbackResult> Output for F {
    fn write(&mut self, text: &str) -> Result {
        self(text).into_result()
    }
}

/// Blocking input for a [`Shell`](crate::Shell).
///
/// Implemented for closures that fill a buffer and return its byte count, or `Ok(0)`
/// at the end of input.
pub trait Input {
    /// Fills `buffer` and returns the byte count, or `Ok(0)` at the end of input.
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize>;
}

impl<F: FnMut(&mut [u8]) -> Result<usize>> Input for F {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        self(buffer)
    }
}

/// Async input for an [`AsyncShell`](crate::AsyncShell).
///
/// Implemented for polling closures that fill a buffer. Before returning
/// [`Poll::Pending`], they must register the waker and wake it when input is
/// available.
pub trait AsyncInput {
    /// Fills `buffer` and returns the byte count, or `Ok(0)` at the end of input.
    fn read(&mut self, buffer: &mut [u8]) -> impl Future<Output = Result<usize>>;
}

impl<F: FnMut(&mut Context<'_>, &mut [u8]) -> Poll<Result<usize>>> AsyncInput for F {
    fn read(&mut self, buffer: &mut [u8]) -> impl Future<Output = Result<usize>> {
        poll_fn(move |cx| self(cx, buffer))
    }
}

/// An `embedded-io` writer or reader, as stored by `new_embedded`.
#[cfg(feature = "embedded-io")]
pub struct EmbeddedIo<T>(pub(crate) T);

#[cfg(feature = "embedded-io")]
impl<T: embedded_io::Write> Output for EmbeddedIo<T> {
    fn write(&mut self, text: &str) -> Result {
        let error = |error: T::Error| Error::EmbeddedIo(embedded_io::Error::kind(&error));
        self.0.write_all(text.as_bytes()).map_err(error)?;
        self.0.flush().map_err(error)
    }
}

#[cfg(feature = "embedded-io")]
impl<T: embedded_io::Read> Input for EmbeddedIo<T> {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        self.0
            .read(buffer)
            .map_err(|error| Error::EmbeddedIo(embedded_io::Error::kind(&error)))
    }
}

#[cfg(feature = "embedded-io")]
impl<T: embedded_io_async::Read> AsyncInput for EmbeddedIo<T> {
    async fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        self.0
            .read(buffer)
            .await
            .map_err(|error| Error::EmbeddedIo(embedded_io::Error::kind(&error)))
    }
}

/// A `std::io` writer or reader, as stored by `Shell::new_std`.
#[cfg(feature = "std")]
pub struct StdIo<T>(pub(crate) T);

#[cfg(feature = "std")]
impl<T: std::io::Write> Output for StdIo<T> {
    fn write(&mut self, text: &str) -> Result {
        self.0.write_all(text.as_bytes())?;
        Ok(self.0.flush()?)
    }
}

#[cfg(feature = "std")]
impl<T: std::io::Read> Input for StdIo<T> {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize> {
        Ok(self.0.read(buffer)?)
    }
}

/// Bytes read in one batch, shared by blocking and async input.
pub(crate) struct InputBuffer {
    bytes: [u8; 64],
    start: usize,
    end: usize,
    /// Bytes left before async input yields to other tasks.
    budget: usize,
}

impl InputBuffer {
    pub(crate) fn new() -> Self {
        Self {
            bytes: [0; 64],
            start: 0,
            end: 0,
            budget: 64,
        }
    }

    fn next(&mut self) -> Option<u8> {
        let byte = *self.bytes[self.start..self.end].first()?;
        self.start += 1;
        Some(byte)
    }

    fn filled(&mut self, length: usize) -> Result<Option<u8>> {
        if length > self.bytes.len() {
            return Err(Error::Message(
                "input callback returned an invalid byte count",
            ));
        }
        (self.start, self.end) = (0, length);
        Ok(self.next())
    }

    /// Returns a buffered byte, refilling when necessary. `None` means end of input.
    pub(crate) fn read(&mut self, input: &mut impl Input) -> Result<Option<u8>> {
        if let Some(byte) = self.next() {
            return Ok(Some(byte));
        }
        let length = input.read(&mut self.bytes)?;
        self.filled(length)
    }

    /// Like [`Self::read`], but yields to other tasks after every 64 bytes.
    pub(crate) async fn read_async(&mut self, input: &mut impl AsyncInput) -> Result<Option<u8>> {
        if self.budget == 0 {
            self.budget = self.bytes.len();
            yield_now().await;
        }
        self.budget -= 1;
        if let Some(byte) = self.next() {
            return Ok(Some(byte));
        }
        let length = input.read(&mut self.bytes).await?;
        self.filled(length)
    }
}

/// Returns `Pending` once so that other tasks can run.
async fn yield_now() {
    let mut yielded = false;
    poll_fn(|cx| {
        if yielded {
            return Poll::Ready(());
        }
        yielded = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    })
    .await;
}
