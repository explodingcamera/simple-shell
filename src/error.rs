use core::fmt;

/// An output failure or a command error.
#[derive(Debug)]
pub enum Error {
    /// Input exceeded the fixed command capacity.
    LineTooLong,
    /// Input exceeded the fixed argument capacity, including the command name.
    TooManyArguments,
    /// A static error message, stored without allocation.
    Message(&'static str),
    /// No command matched the input.
    CommandNotFound,
    /// Formatting failed without an underlying output error.
    Formatting,
    /// An I/O error from a standard-library adapter.
    #[cfg(feature = "std")]
    Io(std::io::Error),
}

/// The shell's result type.
pub type Result<T = ()> = core::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, output: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LineTooLong => output.write_str("command buffer is full"),
            Self::TooManyArguments => output.write_str("command exceeds 64 arguments"),
            Self::Message(message) => output.write_str(message),
            Self::CommandNotFound => output.write_str("command not found"),
            Self::Formatting => output.write_str("formatting failed"),
            #[cfg(feature = "std")]
            Self::Io(error) => error.fmt(output),
        }
    }
}

impl core::error::Error for Error {
    #[cfg(feature = "std")]
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<&'static str> for Error {
    fn from(message: &'static str) -> Self {
        Self::Message(message)
    }
}

impl From<fmt::Error> for Error {
    fn from(_: fmt::Error) -> Self {
        Self::Formatting
    }
}

#[cfg(feature = "std")]
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
