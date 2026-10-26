use core::{
    fmt::{self, Write},
    future::{Future, ready},
    marker::PhantomData,
};

use crate::{CallbackResult, Error, Result, ShellContext};

/// Description and aliases shared by synchronous and async commands.
pub trait CommandInfo: Sync {
    /// Returns the description shown by the help screen.
    fn help(&self) -> &str;
    /// Returns the primary name followed by aliases. An empty name is the fallback.
    /// An empty slice matches nothing.
    fn names(&self) -> &[&str];

    /// Returns whether this command handles `name`. The fallback handles every name.
    fn matches(&self, name: &str) -> bool {
        let names = self.names();
        names.first() == Some(&"") || names.contains(&name)
    }
}

/// A synchronous command that can be stored in a borrowed [`CommandList`].
pub trait Command<S = ()>: CommandInfo {
    /// Runs the handler. `argv[0]` is the invoked name.
    fn run(&self, argv: &[&str], ctx: &mut ShellContext<'_, S>) -> Result;
}

/// A command whose concrete future is retained by a typed command list.
pub trait AsyncCommand<S = ()>: CommandInfo {
    /// Runs the handler without boxing its future.
    fn run_async(
        &self,
        args: &[&str],
        shell: &mut ShellContext<'_, S>,
    ) -> impl core::future::Future<Output = Result>;
}

/// A borrowed list of command names and handlers.
///
/// Use `CommandList<'static>` with manual [`ShellCommand`] constructors for static lists.
/// [`commands!`](crate::commands) retains concrete handler types instead.
/// This alias does not allocate or own commands.
pub type CommandList<'a, S = ()> = &'a [&'a dyn Command<S>];

/// Marks an async callback without allocating or erasing its future.
#[doc(hidden)]
pub struct Async<F>(F);

/// A command registered with a [`Shell`](crate::Shell) or [`AsyncShell`](crate::AsyncShell).
///
/// The handler receives an argument vector including the invoked name at index
/// zero. It can return `()` or a result whose error converts to [`Error`].
pub struct ShellCommand<'a, F, S = ()> {
    /// Description shown by [`ShellContext::print_help_screen`].
    pub help: &'a str,
    /// Handler for the command. Errors are printed with the command name.
    pub func: F,
    /// Primary name followed by aliases.
    pub names: &'a [&'a str],
    output: PhantomData<fn() -> S>,
}

impl<'a, F, C, S> ShellCommand<'a, F, S>
where
    F: Fn(&[&str], &mut ShellContext<'_, S>) -> C + Sync,
    C: CallbackResult,
{
    /// Creates a command with its primary name, aliases, and handler.
    pub const fn new(names: &'a [&'a str], func: F) -> Self {
        Self {
            help: "",
            func,
            names,
            output: PhantomData,
        }
    }
}

impl<'a, F, S> ShellCommand<'a, F, S> {
    /// Creates an async command without boxing its future.
    ///
    /// Async commands can only be registered with [`AsyncShell`](crate::AsyncShell).
    pub const fn new_async<C>(names: &'a [&'a str], func: F) -> ShellCommand<'a, Async<F>, S>
    where
        F: AsyncFn(&[&str], &mut ShellContext<'_, S>) -> C + Sync,
        C: CallbackResult,
    {
        ShellCommand {
            help: "",
            func: Async(func),
            names,
            output: PhantomData,
        }
    }

    /// Sets the description displayed by the help screen.
    pub const fn with_help(mut self, help: &'a str) -> Self {
        self.help = help;
        self
    }
}

impl<F: Sync, S> CommandInfo for ShellCommand<'_, F, S> {
    fn help(&self) -> &str {
        self.help
    }

    fn names(&self) -> &[&str] {
        self.names
    }
}

impl<F, C, S> Command<S> for ShellCommand<'_, F, S>
where
    F: Fn(&[&str], &mut ShellContext<'_, S>) -> C + Sync,
    C: CallbackResult,
{
    fn run(&self, argv: &[&str], ctx: &mut ShellContext<'_, S>) -> Result {
        (self.func)(argv, ctx).into_result()
    }
}

impl<F, C, S> AsyncCommand<S> for ShellCommand<'_, F, S>
where
    F: Fn(&[&str], &mut ShellContext<'_, S>) -> C + Sync,
    C: CallbackResult,
{
    fn run_async(
        &self,
        args: &[&str],
        shell: &mut ShellContext<'_, S>,
    ) -> impl core::future::Future<Output = Result> {
        core::future::ready(self.run(args, shell))
    }
}

impl<F, C, S> AsyncCommand<S> for ShellCommand<'_, Async<F>, S>
where
    F: AsyncFn(&[&str], &mut ShellContext<'_, S>) -> C + Sync,
    C: CallbackResult,
{
    async fn run_async(&self, args: &[&str], shell: &mut ShellContext<'_, S>) -> Result {
        (self.func.0)(args, shell).await.into_result()
    }
}

/// A command set, including an optional help screen.
///
/// Implement this for custom command execution. Help output is optional and
/// defaults to nothing. [`crate::commands!`] supplies both execution and help.
pub trait Commands<S = ()>: Sync {
    /// Writes descriptions for the built-in help screen.
    fn write_help(&self, _: &mut dyn Write) -> fmt::Result {
        Ok(())
    }

    /// Runs a command, returning [`Error::CommandNotFound`] if none matched.
    fn run(&self, argv: &[&str], ctx: &mut ShellContext<'_, S>) -> impl Future<Output = Result>
    where
        Self: Sized;
}

/// A command set that can also execute in a blocking [`crate::Shell`].
pub trait SyncCommands<S = ()>: Commands<S> {
    /// Runs a matching command synchronously. `argv[0]` is the invoked name.
    fn run_sync(&self, argv: &[&str], ctx: &mut ShellContext<'_, S>) -> Result;
}

/// Column where descriptions start in the help screen.
const HELP_COLUMN: usize = 16;

/// Writes one help line in the style of GNU `--help` output: names and aliases,
/// then the description aligned at [`HELP_COLUMN`].
fn write_entry(output: &mut dyn Write, command: &dyn CommandInfo) -> fmt::Result {
    let names = command.names();
    if names.first().is_none_or(|name| name.is_empty()) {
        return Ok(());
    }
    output.write_str("  ")?;
    let mut width = 2;
    for (index, name) in names.iter().enumerate() {
        if index > 0 {
            output.write_str(", ")?;
            width += 2;
        }
        output.write_str(name)?;
        width += name.chars().count();
    }
    if command.help().is_empty() {
        return output.write_char('\n');
    }
    // Names too long for the column put the description on the next line.
    if width + 2 > HELP_COLUMN {
        output.write_char('\n')?;
        width = 0;
    }
    for (index, line) in command.help().lines().enumerate() {
        let indent = if index == 0 {
            HELP_COLUMN - width
        } else {
            HELP_COLUMN
        };
        writeln!(output, "{:indent$}{line}", "")?;
    }
    Ok(())
}

impl<S> Commands<S> for () {
    fn run(&self, argv: &[&str], ctx: &mut ShellContext<'_, S>) -> impl Future<Output = Result> {
        ready(self.run_sync(argv, ctx))
    }
}

impl<S> SyncCommands<S> for () {
    fn run_sync(&self, argv: &[&str], _: &mut ShellContext<'_, S>) -> Result {
        if argv.is_empty() {
            Ok(())
        } else {
            Err(Error::CommandNotFound)
        }
    }
}

impl<S> Commands<S> for CommandList<'_, S> {
    fn write_help(&self, output: &mut dyn Write) -> fmt::Result {
        for command in *self {
            write_entry(output, *command)?;
        }
        Ok(())
    }

    async fn run(&self, argv: &[&str], ctx: &mut ShellContext<'_, S>) -> Result {
        self.run_sync(argv, ctx)
    }
}

impl<S> SyncCommands<S> for CommandList<'_, S> {
    fn run_sync(&self, argv: &[&str], ctx: &mut ShellContext<'_, S>) -> Result {
        let Some(name) = argv.first() else {
            return Ok(());
        };
        match self.iter().find(|command| command.matches(name)) {
            Some(command) => command.run(argv, ctx),
            None => Err(Error::CommandNotFound),
        }
    }
}

impl<S, C, T> Commands<S> for (&C, T)
where
    C: AsyncCommand<S>,
    T: Commands<S>,
{
    fn write_help(&self, output: &mut dyn Write) -> fmt::Result {
        write_entry(output, self.0)?;
        self.1.write_help(output)
    }

    async fn run(&self, argv: &[&str], ctx: &mut ShellContext<'_, S>) -> Result {
        if argv.first().is_some_and(|name| self.0.matches(name)) {
            self.0.run_async(argv, ctx).await
        } else {
            self.1.run(argv, ctx).await
        }
    }
}

impl<S, C, T> SyncCommands<S> for (&C, T)
where
    C: Command<S> + AsyncCommand<S>,
    T: SyncCommands<S>,
{
    fn run_sync(&self, argv: &[&str], ctx: &mut ShellContext<'_, S>) -> Result {
        if argv.first().is_some_and(|name| self.0.matches(name)) {
            self.0.run(argv, ctx)
        } else {
            self.1.run_sync(argv, ctx)
        }
    }
}
