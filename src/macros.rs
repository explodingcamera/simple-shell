/// Builds a borrowed command collection without allocating.
///
/// Entries use `"name" => handler`, with optional doc comments for help text.
/// Names separated by `|` register aliases, with the first name as primary.
/// Use `"name" => async |args, shell| { ... }` for an unboxed async command.
/// Function paths also work, including associated functions. Prefix async paths
/// with `async`, for example `"save" => async Handlers::save`.
/// Manual [`crate::ShellCommand`] constructors can also appear as entries.
///
/// A final `_` entry handles unmatched input. It receives all input tokens,
/// including the command name. Internally its name is the empty string.
/// Tables retain concrete handler types and are inferred with `let`. Inline
/// `const` tables work without naming the generated type.
///
/// ```
/// use simple_shell::{commands, Commands, Shell, ShellCommand};
///
/// let commands = const { commands! {
///     /// Show commands.
///     "help" | "h" | "?" => |_, shell| shell.print_help_screen(),
///     ShellCommand::new(&["custom"], |_, _| {}).with_help("custom command"),
/// } };
/// let shell = Shell::new(|_| {}, |_| Ok(0)).with_commands(&commands);
/// let mut help = String::new();
/// commands.write_help(&mut help).expect("String output succeeds");
/// assert!(help.contains("Show commands."));
/// ```
///
/// Async handlers require [`crate::AsyncShell`]:
///
/// ```compile_fail
/// use simple_shell::{commands, Shell};
/// let commands = commands! { "wait" => async |_, _| {} };
/// Shell::new(|_| {}, |_| Ok(0)).with_commands(&commands);
/// ```
///
/// The catch-all must be last:
///
/// ```compile_fail
/// use simple_shell::commands;
/// let commands = commands! {
///     _ => |_, _| {},
///     "unreachable" => |_, _| {},
/// };
/// ```
#[macro_export]
macro_rules! commands {
    (@parse [$($entries:tt)*] []) => {
        $crate::commands!(@chain $($entries)*)
    };
    (@parse [$($entries:tt)*] [] , $($rest:tt)*) => {
        $crate::commands!(@parse [$($entries)*] [] $($rest)*)
    };
    (@parse [$($entries:tt)*] [$($doc:literal,)*]
        #[doc = $next:literal] $($rest:tt)*
    ) => {
        $crate::commands!(@parse [$($entries)*] [$($doc,)* $next,] $($rest)*)
    };
    (@parse [$($entries:tt)*] [$($doc:literal,)*]
        $name:tt $(| $short:literal)* =>
        async $($capture:ident)? |$($arg:tt $(: $arg_type:ty)?),* $(,)?| -> $result:ty $body:block,
        $($rest:tt)*
    ) => {
        $crate::commands!(@next [
            $($entries)*
            $crate::ShellCommand::new_async(
                &[$crate::commands!(@name $name), $($short),*],
                async $($capture)? |$($arg $(: $arg_type)?),*| -> $result { $body },
            ).with_help(concat!($($doc, "\n",)*).trim_ascii()),
        ] [$name] $($rest)*)
    };
    (@parse [$($entries:tt)*] [$($doc:literal,)*]
        $name:tt $(| $short:literal)* =>
        async $($capture:ident)? |$($arg:tt $(: $arg_type:ty)?),* $(,)?| $body:expr,
        $($rest:tt)*
    ) => {
        $crate::commands!(@next [
            $($entries)*
            $crate::ShellCommand::new_async(
                &[$crate::commands!(@name $name), $($short),*],
                async $($capture)? |$($arg $(: $arg_type)?),*| $body,
            ).with_help(concat!($($doc, "\n",)*).trim_ascii()),
        ] [$name] $($rest)*)
    };
    (@parse [$($entries:tt)*] [$($doc:literal,)*]
        $name:tt $(| $short:literal)* => async $handler:expr, $($rest:tt)*
    ) => {
        $crate::commands!(@next [
            $($entries)*
            $crate::ShellCommand::new_async(
                &[$crate::commands!(@name $name), $($short),*], $handler,
            ).with_help(concat!($($doc, "\n",)*).trim_ascii()),
        ] [$name] $($rest)*)
    };
    (@parse [$($entries:tt)*] [$($doc:literal,)*]
        $name:tt $(| $short:literal)* => $handler:expr, $($rest:tt)*
    ) => {
        $crate::commands!(@next [
            $($entries)*
            $crate::ShellCommand::new(
                &[$crate::commands!(@name $name), $($short),*], $handler,
            ).with_help(concat!($($doc, "\n",)*).trim_ascii()),
        ] [$name] $($rest)*)
    };
    (@parse [$($entries:tt)*] [] $command:expr, $($rest:tt)*) => {
        $crate::commands!(@parse [$($entries)* $command,] [] $($rest)*)
    };
    (@next [$($entries:tt)*] [_] $($rest:tt)*) => {
        $crate::commands!(@fallback [$($entries)*] $($rest)*)
    };
    (@next [$($entries:tt)*] [$name:tt] $($rest:tt)*) => {
        $crate::commands!(@parse [$($entries)*] [] $($rest)*)
    };
    (@fallback [$($entries:tt)*] $(,)*) => {
        $crate::commands!(@chain $($entries)*)
    };
    (@fallback $($invalid:tt)*) => { compile_error!("the catch-all command must be last") };
    (@chain) => { () };
    (@chain $command:expr, $($rest:tt)*) => {
        (&$command, $crate::commands!(@chain $($rest)*))
    };
    (@name _) => { "" };
    (@name $name:literal) => { $name };
    (@parse $($invalid:tt)*) => {
        compile_error!("expected name => handler or a command expression")
    };
    ($($entries:tt)*) => {
        $crate::commands!(@parse [] [] $($entries)* ,)
    };
}

/// Implements the builder methods and accessors shared by [`Shell`](crate::Shell) and
/// [`AsyncShell`](crate::AsyncShell), which have the same fields.
macro_rules! impl_shell {
    ($shell:ident, $input:ident, $commands:ident) => {
        impl<'a, O: crate::io::Output + Send, I: crate::io::$input, S, C, B: crate::LineBuffer>
            $shell<'a, O, I, S, C, B>
        {
            /// Attaches application data, available to handlers as `ctx.data`.
            /// Configure this first. Changing state resets the commands and hooks.
            pub fn with_state<T>(self, data: T) -> $shell<'a, O, I, T, (), B> {
                $shell {
                    storage: self.storage.with_data(data),
                    input: self.input,
                    buffer: self.buffer,
                    line: self.line,
                    commands: &(),
                }
            }

            /// Sets the borrowed command collection, replacing any previous collection.
            pub fn with_commands<T: crate::$commands<S>>(
                self,
                commands: &'a T,
            ) -> $shell<'a, O, I, S, T, B> {
                let mut storage = self.storage;
                storage.help = commands;
                $shell {
                    storage,
                    input: self.input,
                    buffer: self.buffer,
                    line: self.line,
                    commands,
                }
            }

            /// Sets the prompt writer. The default writes `"> "`.
            pub fn with_prompt<P: crate::Prompt<S>>(mut self, prompt: &'a P) -> Self {
                self.storage.prompt = prompt;
                self
            }

            /// Sets the handler for command errors and unmatched input.
            pub fn with_error_handler<H: crate::ErrorHandler<S>>(mut self, handler: &'a H) -> Self {
                self.storage.error_handler = handler;
                self
            }

            /// Sets whether the shell echoes input, which is the default.
            ///
            /// Disable it for clients that echo locally and send whole lines, such as IDE
            /// serial monitors or a terminal in cooked mode. Editing keys still work, but
            /// the shell does not redraw the line.
            pub fn with_echo(mut self, enabled: bool) -> Self {
                self.storage.state.editor.client_echo = !enabled;
                self
            }

            /// Sets whether Ctrl-D on an empty line is ignored, like `set -o ignoreeof` in bash.
            ///
            /// By default, Ctrl-D on an empty line ends [`Self::run`]. The shell still stops
            /// at the end of input, when the input returns `Ok(0)`.
            pub fn with_ignore_eof(mut self, ignore: bool) -> Self {
                self.storage.state.editor.ignore_eof = ignore;
                self
            }

            /// Sets whether output line endings are written as `\r\n`, which raw serial
            /// terminals and telnet expect. This includes output from commands.
            pub fn with_crlf(mut self, enabled: bool) -> Self {
                self.storage.write.crlf = enabled;
                self
            }

            /// Replaces the command line storage, for example with a `heapless::String<N>`.
            ///
            /// Lines that do not fit in fixed-size storage are rejected.
            pub fn with_buffer<T: crate::LineBuffer>(self, buffer: T) -> $shell<'a, O, I, S, C, T> {
                $shell {
                    storage: self.storage,
                    input: self.input,
                    buffer: self.buffer,
                    line: buffer,
                    commands: self.commands,
                }
            }

            /// Returns the line being edited.
            pub fn command(&self) -> &str {
                self.line.as_str()
            }

            /// Returns the cursor's byte offset in [`Self::command`], always at a character
            /// boundary.
            pub fn cursor(&self) -> usize {
                self.storage.state.editor.cursor
            }

            /// Borrows the shell's application state and formatted output.
            pub fn context(&mut self) -> crate::ShellContext<'_, S> {
                self.storage.context()
            }

            /// Prints a message on its own line above the prompt, then redraws the prompt
            /// and the partial input. Use it for log output while someone is typing.
            pub fn print_above(&mut self, message: impl core::fmt::Display) -> crate::Result {
                self.storage.context().print_above(&self.line, message);
                self.storage.write.take_error()
            }
        }

        impl<O, I, S, C, B> core::ops::Deref for $shell<'_, O, I, S, C, B> {
            type Target = crate::ShellState<S>;
            fn deref(&self) -> &Self::Target {
                &self.storage.state
            }
        }

        impl<O, I, S, C, B> core::ops::DerefMut for $shell<'_, O, I, S, C, B> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.storage.state
            }
        }
    };
}
pub(crate) use impl_shell;
