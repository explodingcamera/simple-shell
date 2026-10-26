use std::io::{self, Read, Write};

use simple_shell::{Shell, commands};

fn write(text: &str) -> io::Result<()> {
    let mut stdout = io::stdout();
    stdout.write_all(text.as_bytes())?;
    stdout.flush()
}

fn read(buffer: &mut [u8]) -> simple_shell::Result<usize> {
    io::stdin().read(buffer).map_err(Into::into)
}

fn main() -> simple_shell::Result {
    let commands = const {
        commands! {
            /// Show commands.
            "help" | "?" | "h" => |_, shell| shell.print_help_screen(),
            /// Exit the shell.
            "exit" | "quit" => |_, _| -> () { std::process::exit(0) },
        }
    };
    let mut shell = Shell::new(write, read).with_commands(&commands);
    shell.run()
}
