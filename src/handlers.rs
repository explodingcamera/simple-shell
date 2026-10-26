use core::fmt::Write;

use crate::{CallbackResult, Error, ShellContext};

/// Draws the prompt using application state and the shell's output.
pub trait Prompt<S, R: CallbackResult = ()>: Sync {
    /// Writes a prompt without allocating a string.
    fn write_prompt(&self, ctx: &mut ShellContext<'_, R, S>);
}

/// Reports command failures and unmatched input.
///
/// Output failures are returned by `run()` instead of being sent to this hook.
pub trait ErrorHandler<S, R: CallbackResult = ()>: Sync {
    /// Writes or otherwise handles an error for the named command.
    fn write_error(&self, name: &str, error: &Error, ctx: &mut ShellContext<'_, R, S>);
}

impl<S, R: CallbackResult, F: Fn(&mut ShellContext<'_, R, S>) + Sync> Prompt<S, R> for F {
    fn write_prompt(&self, ctx: &mut ShellContext<'_, R, S>) {
        self(ctx);
    }
}

impl<S, R: CallbackResult, F: Fn(&str, &Error, &mut ShellContext<'_, R, S>) + Sync>
    ErrorHandler<S, R> for F
{
    fn write_error(&self, name: &str, error: &Error, ctx: &mut ShellContext<'_, R, S>) {
        self(name, error, ctx);
    }
}

pub(crate) struct DefaultHandlers;

impl<S, R: CallbackResult> Prompt<S, R> for DefaultHandlers {
    fn write_prompt(&self, ctx: &mut ShellContext<'_, R, S>) {
        let _ = ctx.write_str("> ");
    }
}

impl<S, R: CallbackResult> ErrorHandler<S, R> for DefaultHandlers {
    fn write_error(&self, name: &str, error: &Error, ctx: &mut ShellContext<'_, R, S>) {
        if name.is_empty() {
            let _ = writeln!(ctx, "{error}");
        } else {
            let _ = writeln!(ctx, "{name}: {error}");
        }
    }
}
