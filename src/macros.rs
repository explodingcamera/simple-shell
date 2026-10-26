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
