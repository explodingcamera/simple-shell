// These tests rely on growable input storage.
#![cfg(feature = "alloc")]

use simple_shell::{AsyncShell, Error, Result, Shell, commands};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fmt::Write,
    future::Future,
    pin::pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
};

thread_local! {
    static INPUT: RefCell<VecDeque<u8>> = const { RefCell::new(VecDeque::new()) };
    static OUTPUT: RefCell<String> = const { RefCell::new(String::new()) };
    static READS: Cell<usize> = const { Cell::new(0) };
}

fn write(text: &str) {
    OUTPUT.with(|output| output.borrow_mut().push_str(text));
}

fn read(buffer: &mut [u8]) -> Result<usize> {
    READS.set(READS.get() + 1);
    INPUT.with(|input| {
        let mut input = input.borrow_mut();
        let length = input.len().min(buffer.len());
        for byte in &mut buffer[..length] {
            *byte = input.pop_front().expect("input length checked");
        }
        Ok(length)
    })
}

fn poll_read(_: &mut Context<'_>, buffer: &mut [u8]) -> Poll<Result<usize>> {
    let result = read(buffer);
    if matches!(result, Ok(0)) {
        Poll::Pending
    } else {
        Poll::Ready(result)
    }
}

struct DropCount<'a>(&'a AtomicUsize);
impl Drop for DropCount<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn reports_eof_and_input_failures() {
    assert!(Shell::new(|_| {}, |_| Ok(0)).run().is_ok());
    assert!(smol::block_on(AsyncShell::new(|_| {}, |_, _| Poll::Ready(Ok(0))).run()).is_ok());
    let result = Shell::new(|_| {}, |_| Err("input failed".into())).run();
    assert!(matches!(result, Err(Error::Message("input failed"))));
    let result = smol::block_on(
        AsyncShell::new(|_| {}, |_, _| Poll::Ready(Err("input failed".into()))).run(),
    );
    assert!(matches!(result, Err(Error::Message("input failed"))));
    let result = Shell::new(|_| {}, |buffer| Ok(buffer.len() + 1)).run();
    assert!(matches!(
        result,
        Err(Error::Message(
            "input callback returned an invalid byte count"
        ))
    ));
    let result = smol::block_on(
        AsyncShell::new(|_| {}, |_, buffer| Poll::Ready(Ok(buffer.len() + 1))).run(),
    );
    assert!(matches!(
        result,
        Err(Error::Message(
            "input callback returned an invalid byte count"
        ))
    ));
}

#[test]
fn preserves_input_after_cancellation() {
    INPUT.with(|input| input.borrow_mut().extend(b"wait\n\x03increment\n"));
    let commands = commands! {
        "wait" => async |_, _| core::future::pending::<()>().await,
        "increment" => |_, ctx| { ctx.data += 1; },
    };
    let mut shell = AsyncShell::new(write, poll_read)
        .with_state(0usize)
        .with_commands(&commands);
    {
        let mut run = pin!(shell.run());
        assert!(
            run.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(shell.data, 1);
    assert_eq!(READS.get(), 2, "one bulk read followed by one pending read");
    OUTPUT.with(|output| assert!(output.borrow().contains("^C\n> ")));
}

#[test]
fn eof_drops_pending_command() {
    INPUT.with(|input| input.borrow_mut().extend(b"wait\n"));
    let drops = AtomicUsize::new(0);
    let commands = commands! {
        "wait" => async |_, _| {
            let _guard = DropCount(&drops);
            core::future::pending::<()>().await;
        },
    };
    let mut shell =
        AsyncShell::new(write, |_, buffer| Poll::Ready(read(buffer))).with_commands(&commands);
    assert!(smol::block_on(shell.run()).is_ok());
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[test]
fn skips_echo_for_echoing_clients() {
    INPUT.with(|input| input.borrow_mut().extend(b"echo hi \\\nthere\n"));
    let commands = commands! {
        "echo" => |argv, shell| { let _ = writeln!(shell, "{}", argv[1..].join(" ")); },
    };
    Shell::new(write, read)
        .with_echo(false)
        .with_commands(&commands)
        .run()
        .expect("output succeeds");
    OUTPUT.with(|output| assert_eq!(*output.borrow(), "> hi there\n> "));
}

#[test]
fn parses_quoted_arguments() {
    INPUT.with(|input| {
        input
            .borrow_mut()
            .extend(b"echo \"a b\" 'c d' e\"f \"\"\necho \"oops\n")
    });
    let commands = commands! {
        "echo" => |argv, shell| { let _ = writeln!(shell, "{}", argv[1..].join("|")); },
    };
    Shell::new(write, read)
        .with_echo(false)
        .with_commands(&commands)
        .run()
        .expect("output succeeds");
    OUTPUT.with(|output| assert_eq!(*output.borrow(), "> a b|c d|e\"f|\n> unclosed quote\n> "));
}

#[test]
fn ignores_ctrl_d_when_configured() {
    INPUT.with(|input| input.borrow_mut().extend(b"\x04count\n"));
    let commands = commands! {
        "count" => |_, shell| { shell.data += 1; },
    };
    let mut shell = Shell::new(write, read)
        .with_ignore_eof(true)
        .with_state(0usize)
        .with_commands(&commands);
    shell.run().expect("output succeeds");
    assert_eq!(shell.data, 1);
}

#[test]
fn writes_crlf_line_endings() {
    INPUT.with(|input| input.borrow_mut().extend(b"lines\n"));
    let commands = commands! {
        "lines" => |_, shell| { let _ = write!(shell, "a\nb\r\n"); },
    };
    Shell::new(write, read)
        .with_crlf(true)
        .with_commands(&commands)
        .run()
        .expect("output succeeds");
    OUTPUT.with(|output| assert_eq!(*output.borrow(), "> lines\r\na\r\nb\r\n> "));
}

#[test]
fn prints_above_partial_input() {
    let commands = commands! { "ls" => |_, _| {} };
    let mut shell = Shell::new(write, |_| Ok(0)).with_commands(&commands);
    shell.context().print_prompt();
    for byte in *b"ls" {
        assert!(shell.feed(byte).expect("output succeeds"));
    }
    shell.print_above("log").expect("output succeeds");
    assert_eq!(shell.command(), "ls");
    OUTPUT.with(|output| {
        assert_eq!(
            *output.borrow(),
            "> ls\r\x1b[2Klog\n\r\x1b[2K> ls\x1b7\x1b8"
        );
    });
}

#[cfg(feature = "std")]
#[test]
fn runs_with_std_io() {
    let mut output = Vec::new();
    let commands = commands! { "hi" => |_, shell| { let _ = writeln!(shell, "hello"); } };
    Shell::new_std(&mut output, &b"hi\n"[..])
        .with_commands(&commands)
        .run()
        .expect("output succeeds");
    assert_eq!(output, b"> hi\nhello\n> ");
}

#[cfg(feature = "embedded-io")]
#[test]
fn runs_with_embedded_io() {
    let commands = commands! { "hi" => |_, shell| { let _ = writeln!(shell, "hello"); } };
    let mut output = [0; 32];
    Shell::new_embedded(&mut output[..], &b"hi\n"[..])
        .with_commands(&commands)
        .run()
        .expect("output succeeds");
    assert!(output.starts_with(b"> hi\nhello\n> "));
    let mut output = [0; 32];
    smol::block_on(
        AsyncShell::new_embedded(&mut output[..], &b"hi\n"[..])
            .with_commands(&commands)
            .run(),
    )
    .expect("output succeeds");
    assert!(output.starts_with(b"> hi\nhello\n> "));
}

#[test]
fn next_recovers_after_cancelled_command() {
    INPUT.with(|input| input.borrow_mut().extend(b"wait\n"));
    let commands = commands! { "wait" => async |_, _| core::future::pending::<()>().await };
    let mut shell = AsyncShell::new(write, poll_read).with_commands(&commands);
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..4 {
        assert!(matches!(
            pin!(shell.next()).poll(&mut cx),
            Poll::Ready(Ok(true))
        ));
    }
    // Dropping the future while the command runs cancels it.
    assert!(pin!(shell.next()).poll(&mut cx).is_pending());
    assert!(pin!(shell.next()).poll(&mut cx).is_pending());
    assert_eq!(shell.command(), "");
    OUTPUT.with(|output| assert_eq!(*output.borrow(), "wait\n^C\n> "));
}
