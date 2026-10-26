use std::{
    io,
    process::{Command, Stdio},
};

use simple_shell::{Shell, commands};

/// Lets the shell handle echo, line editing, and Ctrl-C instead of the terminal.
fn raw_input(enable: bool) {
    let flags = if enable {
        ["-echo", "-icanon", "-isig"]
    } else {
        ["echo", "icanon", "isig"]
    };
    let _ = Command::new("stty")
        .args(flags)
        .stderr(Stdio::null())
        .status();
}

fn main() -> simple_shell::Result {
    let commands = const {
        commands! {
            /// Show commands.
            "help" | "?" | "h" => |_, shell| shell.print_help_screen(),
            /// Exit the shell.
            "exit" | "quit" => |_, _| -> () {
                raw_input(false);
                std::process::exit(0)
            },
        }
    };
    let mut shell = Shell::new_std(io::stdout(), io::stdin()).with_commands(&commands);
    raw_input(true);
    let result = shell.run();
    raw_input(false);
    result
}
