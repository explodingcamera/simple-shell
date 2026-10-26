// These tests rely on growable input storage.
#![cfg(feature = "alloc")]

use simple_shell::{AsyncShell, Error, Result, Shell, commands};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
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
