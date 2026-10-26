use core::ops::Deref;

use crate::Result;

/// The default command storage: a growing `String` with the `alloc` feature,
/// a 256-byte `heapless::String` with only `heapless`, or `EmptyBuffer` with neither.
#[cfg(feature = "alloc")]
pub type DefaultBuffer = alloc::string::String;
/// The default command storage: a growing `String` with the `alloc` feature,
/// a 256-byte `heapless::String` with only `heapless`, or `EmptyBuffer` with neither.
#[cfg(all(feature = "heapless", not(feature = "alloc")))]
pub type DefaultBuffer = heapless::String<256>;
/// The default command storage: a growing `String` with the `alloc` feature,
/// a 256-byte `heapless::String` with only `heapless`, or `EmptyBuffer` with neither.
#[cfg(not(any(feature = "alloc", feature = "heapless")))]
pub type DefaultBuffer = EmptyBuffer;

/// Storage without capacity, the default when neither `alloc` nor `heapless` is enabled.
///
/// Every line is rejected. Pass your own [`LineBuffer`] to `with_buffer` instead.
#[cfg(not(any(feature = "alloc", feature = "heapless")))]
#[derive(Default)]
pub struct EmptyBuffer;

#[cfg(not(any(feature = "alloc", feature = "heapless")))]
impl LineBuffer for EmptyBuffer {
    fn as_str(&self) -> &str {
        ""
    }

    fn insert_char(&mut self, _: usize, _: char) -> Result {
        Err(crate::Error::LineTooLong)
    }

    fn remove_char(&mut self, _: usize) {}

    fn clear(&mut self) {}
}

/// UTF-8 storage for the line being edited.
///
/// Implemented for `alloc::string::String` with the `alloc` feature and for
/// `heapless::String<N>` with the `heapless` feature. Implement it for your own
/// fixed-size storage to avoid both. Pass one to `with_buffer`
/// on a shell to choose the storage.
pub trait LineBuffer {
    /// Returns the stored line.
    fn as_str(&self) -> &str;
    /// Inserts a character at a byte offset on a character boundary.
    /// Returns [`LineTooLong`](crate::Error::LineTooLong) if the storage is full.
    fn insert_char(&mut self, index: usize, character: char) -> Result;
    /// Removes the character at a byte offset on a character boundary.
    fn remove_char(&mut self, index: usize);
    /// Clears the line without changing the storage capacity.
    fn clear(&mut self);
}

#[cfg(feature = "alloc")]
impl LineBuffer for alloc::string::String {
    fn as_str(&self) -> &str {
        self
    }

    fn insert_char(&mut self, index: usize, character: char) -> Result {
        self.insert(index, character);
        Ok(())
    }

    fn remove_char(&mut self, index: usize) {
        self.remove(index);
    }

    fn clear(&mut self) {
        self.clear();
    }
}

#[cfg(feature = "heapless")]
impl<const N: usize> LineBuffer for heapless::String<N> {
    fn as_str(&self) -> &str {
        self
    }

    fn insert_char(&mut self, index: usize, character: char) -> Result {
        self.insert(index, character)
            .map_err(|_| crate::Error::LineTooLong)
    }

    fn remove_char(&mut self, index: usize) {
        self.remove(index);
    }

    fn clear(&mut self) {
        self.clear();
    }
}

pub(super) struct Arguments<'a> {
    #[cfg(feature = "alloc")]
    args: alloc::vec::Vec<&'a str>,
    #[cfg(not(feature = "alloc"))]
    args: [&'a str; 64],
    #[cfg(not(feature = "alloc"))]
    len: usize,
}

/// Splits a line at whitespace. A word that starts with `"` or `'` runs to the
/// matching quote, and the quotes are removed. Quotes inside a word are kept.
fn split_words(line: &str) -> impl Iterator<Item = Result<&str>> {
    let mut rest = line;
    core::iter::from_fn(move || {
        rest = rest.trim_start();
        let (word, remaining) = match rest.chars().next()? {
            quote @ ('"' | '\'') => match rest[1..].split_once(quote) {
                Some(split) => split,
                None => {
                    rest = "";
                    return Some(Err(crate::Error::UnclosedQuote));
                }
            },
            _ => rest.split_once(char::is_whitespace).unwrap_or((rest, "")),
        };
        rest = remaining;
        Some(Ok(word))
    })
}

impl<'a> Arguments<'a> {
    pub(super) fn parse(line: &'a str) -> Result<Self> {
        #[cfg(feature = "alloc")]
        {
            Ok(Self {
                args: split_words(line).collect::<Result<_>>()?,
            })
        }
        #[cfg(not(feature = "alloc"))]
        {
            let mut result = Self {
                args: [""; 64],
                len: 0,
            };
            for arg in split_words(line) {
                let arg = arg?;
                if result.len == result.args.len() {
                    return Err(crate::Error::TooManyArguments);
                }
                result.args[result.len] = arg;
                result.len += 1;
            }
            Ok(result)
        }
    }
}

impl<'a> Deref for Arguments<'a> {
    type Target = [&'a str];
    fn deref(&self) -> &Self::Target {
        #[cfg(feature = "alloc")]
        {
            &self.args
        }
        #[cfg(not(feature = "alloc"))]
        {
            &self.args[..self.len]
        }
    }
}
