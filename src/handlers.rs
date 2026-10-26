use core::fmt::Write;

use crate::{Error, ShellContext};

/// Draws the prompt using application state and the shell's output.
pub trait Prompt<S>: Sync {
    /// Writes a prompt without allocating a string.
    fn write_prompt(&self, ctx: &mut ShellContext<'_, S>);
}

/// Reports command failures and unmatched input.
///
/// Output failures are returned by `run()` instead of being sent to this hook.
pub trait ErrorHandler<S>: Sync {
    /// Writes or otherwise handles an error for the named command.
    fn write_error(&self, name: &str, error: &Error, ctx: &mut ShellContext<'_, S>);
}

impl<S, F: Fn(&mut ShellContext<'_, S>) + Sync> Prompt<S> for F {
    fn write_prompt(&self, ctx: &mut ShellContext<'_, S>) {
        self(ctx);
    }
}

impl<S, F: Fn(&str, &Error, &mut ShellContext<'_, S>) + Sync> ErrorHandler<S> for F {
    fn write_error(&self, name: &str, error: &Error, ctx: &mut ShellContext<'_, S>) {
        self(name, error, ctx);
    }
}

pub(crate) struct DefaultHandlers;

impl<S> Prompt<S> for DefaultHandlers {
    fn write_prompt(&self, ctx: &mut ShellContext<'_, S>) {
        let _ = ctx.write_str("> ");
    }
}

impl<S> ErrorHandler<S> for DefaultHandlers {
    fn write_error(&self, name: &str, error: &Error, ctx: &mut ShellContext<'_, S>) {
        if name.is_empty() {
            let _ = writeln!(ctx, "{error}");
        } else {
            let _ = writeln!(ctx, "{name}: {error}");
        }
    }
}
