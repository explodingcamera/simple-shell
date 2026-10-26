use std::{
    io::{self, Write},
    pin::Pin,
    process::{Command, Stdio},
    sync::{LazyLock, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use simple_shell::{AsyncShell, commands};
use smol::{Unblock, io::AsyncRead};

static INPUT: LazyLock<Mutex<Unblock<io::Stdin>>> =
    LazyLock::new(|| Mutex::new(Unblock::new(io::stdin())));

fn write(text: &str) -> io::Result<()> {
    let mut stdout = io::stdout();
    stdout.write_all(text.as_bytes())?;
    stdout.flush()
}

fn poll_read(cx: &mut Context<'_>, buffer: &mut [u8]) -> Poll<simple_shell::Result<usize>> {
    let mut input = INPUT.lock().expect("stdin lock poisoned");
    Pin::new(&mut *input)
        .poll_read(cx, buffer)
        .map(|result| result.map_err(Into::into))
}

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
    let commands = commands! {
        /// Show commands.
        "help" | "?" | "h" => |_, shell| shell.print_help_screen(),
        /// Sleep for one second.
        "sleep" => async |_, _| { smol::Timer::after(Duration::from_secs(1)).await; },
        /// Exit the shell.
        "exit" => |_, _| -> () {
            raw_input(false);
            std::process::exit(0)
        },
    };

    let mut shell = AsyncShell::new(write, poll_read).with_commands(&commands);
    raw_input(true);
    let result = smol::block_on(shell.run());
    raw_input(false);
    result
}
