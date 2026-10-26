// These tests rely on growable input storage.
#![cfg(feature = "alloc")]

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fmt::Write,
    future::Future,
    pin::{Pin, pin},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};

use simple_shell::{
    AsyncShell, CommandList, Commands, Error, ErrorHandler, Prompt, Shell, ShellCommand,
    ShellContext, commands,
};

thread_local! {
    static OUTPUT: RefCell<String> = const { RefCell::new(String::new()) };
    static INPUT: RefCell<VecDeque<u8>> = const { RefCell::new(VecDeque::new()) };
    static INPUT_WAKER: RefCell<Option<Waker>> = const { RefCell::new(None) };
    static WRITE_CALLS: Cell<usize> = const { Cell::new(0) };
    static FAIL_AFTER: Cell<usize> = const { Cell::new(0) };
}

fn write(text: &str) {
    OUTPUT.with(|output| output.borrow_mut().push_str(text));
}

#[test]
fn async_task_is_send() {
    async fn task() {
        let commands = commands! {
            "help" => async |_, shell| shell.print_help_screen(),
            "panic" => async |_, _| -> () { panic!("test panic") },
            "echo" => async |argv, _| {
                for arg in argv.iter().skip(1) { write(arg); }
            },
            "error" => async |_, _| -> simple_shell::Result { Err("failed".into()) },
        };
        let mut shell = AsyncShell::new(write, poll_read).with_commands(&commands);
        let _ = shell.run().await;
    }
    fn require_send(_: impl Future + Send + 'static) {}
    require_send(task());
}

fn read(buffer: &mut [u8]) -> simple_shell::Result<usize> {
    INPUT.with(|input| {
        let mut input = input.borrow_mut();
        let length = buffer.len().min(input.len());
        for byte in &mut buffer[..length] {
            *byte = input.pop_front().expect("input length checked");
        }
        Ok(length)
    })
}

fn fallible_write(text: &str) -> Result<(), &'static str> {
    let calls = WRITE_CALLS.get();
    WRITE_CALLS.set(calls + 1);
    if calls >= FAIL_AFTER.get() {
        Err("output failed")
    } else {
        write(text);
        Ok(())
    }
}

fn poll_read(cx: &mut Context<'_>, buffer: &mut [u8]) -> Poll<simple_shell::Result<usize>> {
    match read(buffer) {
        Ok(0) => {
            INPUT_WAKER.with(|waker| *waker.borrow_mut() = Some(cx.waker().clone()));
            Poll::Pending
        }
        result => Poll::Ready(result),
    }
}

fn poll_once<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

struct TestUi;

impl Prompt<usize> for TestUi {
    fn write_prompt(&self, ctx: &mut ShellContext<'_, usize>) {
        let count = ctx.data;
        let _ = write!(ctx, "app({count})> ");
    }
}

impl ErrorHandler<usize> for TestUi {
    fn write_error(&self, name: &str, error: &Error, ctx: &mut ShellContext<'_, usize>) {
        let _ = writeln!(ctx, "error[{name}]: {error}");
    }
}

fn os_prompt(ctx: &mut ShellContext<'_>) {
    let _ = ctx.write_str("os> ");
}

fn unexpected_error(_: &str, _: &Error, _: &mut ShellContext<'_>) {
    panic!("output error sent to command hook");
}

struct WakeCount(AtomicUsize);

impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

struct DropCount<'a>(&'a AtomicUsize);

impl Drop for DropCount<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

static STATE_COMMANDS: CommandList<'static, usize> = &[
    &ShellCommand::new(&["increment", "i"], |_, shell| shell.data += 1)
        .with_help("Increment the counter."),
    &ShellCommand::new(&["stop"], |_, shell| {
        FAIL_AFTER.set(WRITE_CALLS.get());
        shell.print_prompt();
    }),
];

#[test]
fn custom_command_set_needs_no_help_implementation() {
    struct CustomCommands;
    impl Commands<usize> for CustomCommands {
        async fn run(
            &self,
            argv: &[&str],
            ctx: &mut ShellContext<'_, usize>,
        ) -> simple_shell::Result {
            match argv {
                ["increment"] => {
                    ctx.data += 1;
                    Ok(())
                }
                ["fail"] => Err("custom failure".into()),
                _ => Err(Error::CommandNotFound),
            }
        }
    }
    const CUSTOM: CustomCommands = CustomCommands;
    INPUT.with(|input| input.borrow_mut().extend(b"increment\nfail\nmissing\n"));
    let mut shell = AsyncShell::new(write, poll_read)
        .with_state(0usize)
        .with_commands(&CUSTOM);
    {
        let mut run = pin!(shell.run());
        let _: &dyn Send = &run;
        assert!(poll_once(run.as_mut()).is_pending());
    }
    assert_eq!(shell.data, 1);
    OUTPUT.with(|output| {
        let output = output.borrow();
        assert!(output.contains("fail: custom failure\n"));
        assert!(output.contains("missing: command not found\n"));
    });
}

#[test]
fn static_commands_use_application_state() {
    FAIL_AFTER.set(usize::MAX);
    INPUT.with(|input| input.borrow_mut().extend(b"i\nstop\n"));
    let mut shell = Shell::new(fallible_write, read)
        .with_state(0usize)
        .with_prompt(&TestUi)
        .with_commands(&STATE_COMMANDS);

    assert!(matches!(shell.run(), Err(Error::Message("output failed"))));
    assert_eq!(shell.data, 1);
    assert_eq!(STATE_COMMANDS[1].help(), "");
    assert_eq!(STATE_COMMANDS[1].names(), ["stop"]);
    OUTPUT.with(|output| {
        let output = output.borrow();
        assert!(output.starts_with("app(0)> "));
        assert!(output.contains("app(1)> "));
    });
}

#[test]
fn mixed_handlers_and_catch_all() {
    let calls = AtomicUsize::new(0);
    let manual = ShellCommand::new(&["fail"], |_, _| Err("failed")).with_help("report an error");
    let commands = commands! {
        "ok" | "o" => |_, shell| {
            calls.fetch_add(1, Ordering::Relaxed);
            let _ = writeln!(shell, "ok");
        },
        manual,
        /// hidden fallback
        _ => |args, _| {
            assert_eq!(args, ["external", "arg"]);
            calls.fetch_add(1, Ordering::Relaxed);
        },
    };
    INPUT.with(|input| input.borrow_mut().extend(b"o\nfail\nexternal arg\n\n"));
    let mut shell = AsyncShell::new(write, poll_read).with_commands(&commands);
    {
        let mut run = pin!(shell.run());
        assert!(poll_once(run.as_mut()).is_pending());
    }
    shell.context().print_help_screen();
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    OUTPUT.with(|output| {
        let output = output.borrow();
        assert!(output.contains("fail: failed\n"));
        assert!(!output.contains("hidden fallback"));
    });
}

#[test]
fn async_commands_borrow_state() {
    let mut count = 0usize;
    let manual = ShellCommand::new_async(
        &["manual", "m"],
        async |args, shell: &mut ShellContext<'_, &mut usize>| {
            assert_eq!(args, ["m", "hello"]);
            *shell.data += 1;
        },
    )
    .with_help("manual increment");
    let commands = commands! {
        manual,
        "inline" | "i" => async |_, shell: &mut ShellContext<'_, &mut usize>| {
            *shell.data += 1;
        },
        "sync" => |_, shell: &mut ShellContext<'_, &mut usize>| *shell.data += 1,
    };
    INPUT.with(|input| input.borrow_mut().extend(b"m hello\ni\nsync\n"));
    let mut shell = AsyncShell::new(write, poll_read)
        .with_state(&mut count)
        .with_commands(&commands);
    {
        let mut run = pin!(shell.run());
        let _: &dyn Send = &run;
        assert!(poll_once(run.as_mut()).is_pending());
    }
    assert_eq!(*shell.data, 3);
}

#[test]
fn async_errors_and_fallback() {
    let calls = AtomicUsize::new(0);
    let manual = ShellCommand::new_async(&["fail"], async |_, _| Err("failed"));
    let commands = commands! {
        manual,
        "help" => |_, shell| shell.print_help_screen(),
        _ => async |args, _| {
            assert_eq!(args, ["module.wasm", "arg"]);
            calls.fetch_add(1, Ordering::Relaxed);
        },
    };
    INPUT.with(|input| input.borrow_mut().extend(b"fail\nmodule.wasm arg\nhelp\n"));
    let mut shell = AsyncShell::new(write, poll_read).with_commands(&commands);
    let mut run = pin!(shell.run());
    assert!(poll_once(run.as_mut()).is_pending());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    OUTPUT.with(|output| {
        let output = output.borrow();
        assert!(output.contains("fail: failed\n"));
        assert!(output.contains("Commands:"));
    });
}

#[test]
fn custom_error_handler_receives_command_errors() {
    let commands = commands! {
        "fail" => async |_, _| Err("failed"),
    };
    INPUT.with(|input| input.borrow_mut().extend(b"fail\nmissing\n"));
    let mut shell = AsyncShell::new(write, poll_read)
        .with_state(0usize)
        .with_prompt(&TestUi)
        .with_error_handler(&TestUi)
        .with_commands(&commands);
    let mut run = pin!(shell.run());
    assert!(poll_once(run.as_mut()).is_pending());
    OUTPUT.with(|output| {
        let output = output.borrow();
        assert!(output.contains("error[fail]: failed\n"));
        assert!(output.contains("error[missing]: command not found\n"));
    });
}

#[test]
fn input_wakes_and_yields() {
    let wakes = Arc::new(WakeCount(AtomicUsize::new(0)));
    let waker = Waker::from(wakes.clone());
    let mut cx = Context::from_waker(&waker);
    let mut shell = AsyncShell::new(write, poll_read);
    {
        let mut run = pin!(shell.run());
        assert!(run.as_mut().poll(&mut cx).is_pending());
        assert_eq!(wakes.0.load(Ordering::Relaxed), 0);
        INPUT.with(|input| input.borrow_mut().extend([b'x'; 100]));
        INPUT_WAKER.with(|waker| {
            waker
                .borrow_mut()
                .take()
                .expect("input waker registered")
                .wake()
        });
        assert_eq!(wakes.0.load(Ordering::Relaxed), 1);
        assert!(run.as_mut().poll(&mut cx).is_pending());
        INPUT.with(|input| assert!(!input.borrow().is_empty()));
        assert_eq!(wakes.0.load(Ordering::Relaxed), 2);
        assert!(run.as_mut().poll(&mut cx).is_pending());
    }
    assert_eq!(shell.command().len(), 100);
}

#[test]
fn command_wake_completes_without_input() {
    let ready = AtomicUsize::new(0);
    let command_waker = Mutex::new(None::<Waker>);
    let commands = commands! {
        "wait" => async |_, shell| {
                core::future::poll_fn(|cx| {
                    if ready.load(Ordering::Relaxed) != 0 {
                        Poll::Ready(())
                    } else {
                        *command_waker.lock().expect("waker lock poisoned") = Some(cx.waker().clone());
                        Poll::Pending
                    }
                }).await;
                let _ = writeln!(shell, "done");
        },
    };
    let wakes = Arc::new(WakeCount(AtomicUsize::new(0)));
    let waker = Waker::from(wakes.clone());
    let mut cx = Context::from_waker(&waker);
    INPUT.with(|input| input.borrow_mut().extend(b"wait\r\n"));
    let mut shell = AsyncShell::new(write, poll_read)
        .with_commands(&commands)
        .with_prompt(&os_prompt);
    let mut run = pin!(shell.run());
    assert!(run.as_mut().poll(&mut cx).is_pending());
    assert_eq!(wakes.0.load(Ordering::Relaxed), 0);
    ready.store(1, Ordering::Relaxed);
    command_waker
        .lock()
        .expect("waker lock poisoned")
        .take()
        .expect("command waker registered")
        .wake();
    assert_eq!(wakes.0.load(Ordering::Relaxed), 1);
    assert!(run.as_mut().poll(&mut cx).is_pending());
    INPUT.with(|input| input.borrow_mut().push_back(b'\n'));
    assert!(run.as_mut().poll(&mut cx).is_pending());
    OUTPUT.with(|output| {
        let output = output.borrow();
        assert!(output.contains("done\nos> "));
        assert_eq!(output.matches("os> ").count(), 3);
    });
}

#[test]
fn cancellation_drops_the_command() {
    for ctrl_c in [false, true] {
        let drops = AtomicUsize::new(0);
        let commands = commands! {
            "wait" => async |_, _| {
                let _guard = DropCount(&drops);
                core::future::pending::<()>().await;
            },
        };
        INPUT.with(|input| input.borrow_mut().extend(b"wait\n"));
        let mut shell = AsyncShell::new(write, poll_read).with_commands(&commands);
        {
            let mut run = pin!(shell.run());
            assert!(poll_once(run.as_mut()).is_pending());
            assert_eq!(drops.load(Ordering::Relaxed), 0);
            if ctrl_c {
                INPUT.with(|input| {
                    let mut input = input.borrow_mut();
                    input.extend([b'x'; 100]);
                    input.push_back(3);
                });
                assert!(poll_once(run.as_mut()).is_pending());
                assert_eq!(drops.load(Ordering::Relaxed), 0);
                assert!(poll_once(run.as_mut()).is_pending());
                assert_eq!(drops.load(Ordering::Relaxed), 1);
                OUTPUT.with(|output| assert!(output.borrow().ends_with("^C\n> ")));
            }
        }
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn ignores_escape_parameters() {
    for async_mode in [false, true] {
        WRITE_CALLS.set(0);
        FAIL_AFTER.set(usize::MAX);
        INPUT.with(|input| {
            *input.borrow_mut() = b"ok\x1b[1;5D\nok\x1b[3~\nok\x1bOP\nok\x1b[\nok\x1b\r\nstop\n"
                .iter()
                .copied()
                .collect();
        });
        let commands = commands! {
            "ok" => |_, shell| { shell.data += 1; },
            "stop" => |_, shell| {
                FAIL_AFTER.set(WRITE_CALLS.get());
                shell.print_prompt();
            },
        };
        if async_mode {
            let mut shell = AsyncShell::new(fallible_write, poll_read)
                .with_state(0usize)
                .with_commands(&commands);
            {
                let mut run = pin!(shell.run());
                assert!(matches!(
                    poll_once(run.as_mut()),
                    Poll::Ready(Err(Error::Message("output failed")))
                ));
            }
            assert_eq!(shell.data, 5);
        } else {
            let mut shell = Shell::new(fallible_write, read)
                .with_state(0usize)
                .with_commands(&commands);
            assert!(matches!(shell.run(), Err(Error::Message("output failed"))));
            assert_eq!(shell.data, 5);
        }
    }
}

#[test]
fn edits_partial_escape_sequences() {
    let mut shell = AsyncShell::new(write, poll_read);
    let mut run = pin!(shell.run());
    for bytes in [
        b"discard\x03partial\x1b\x03text\x1b[\x03ab\x1b".as_slice(),
        b"[",
        b"DX\x7fY\r",
    ] {
        INPUT.with(|input| input.borrow_mut().extend(bytes));
        assert!(poll_once(run.as_mut()).is_pending());
    }
    OUTPUT.with(|output| {
        let output = output.borrow();
        assert_eq!(output.matches("^C\n> ").count(), 3);
        assert!(output.contains("aYb: command not found"));
    });
}

#[test]
fn edits_split_utf8_input() {
    let mut shell = AsyncShell::new(write, poll_read);
    let mut run = pin!(shell.run());
    for byte in "éλ🙂".bytes() {
        INPUT.with(|input| input.borrow_mut().push_back(byte));
        assert!(poll_once(run.as_mut()).is_pending());
    }
    // Move across multibyte characters, remove lambda, and insert u-umlaut.
    INPUT.with(|input| input.borrow_mut().extend(b"\x1b[D\x1b[D\x1b[C\x7f"));
    assert!(poll_once(run.as_mut()).is_pending());
    INPUT.with(|input| input.borrow_mut().extend("ü".bytes()));
    assert!(poll_once(run.as_mut()).is_pending());
    INPUT.with(|input| input.borrow_mut().extend(b"\x1b[C\x08\n"));
    assert!(poll_once(run.as_mut()).is_pending());
    OUTPUT.with(|output| assert!(output.borrow().contains("éü: command not found")));
}

#[test]
fn rejects_invalid_utf8() {
    let mut shell = AsyncShell::new(write, poll_read);
    let mut run = pin!(shell.run());
    INPUT.with(|input| {
        input.borrow_mut().extend([
            0xf0, 0x80, 0x80, 0x80, // Overlong encoding.
            0xed, 0xa0, 0x80, // Surrogate code point.
            0xf4, 0x90, 0x80, 0x80, // Above the Unicode range.
            0xc0, 0xaf, 0x80, // Invalid leading and stray continuation bytes.
            0xe2, b'A', // Incomplete sequence interrupted by ASCII.
            0xc2, 0x85, // Unicode control character.
        ])
    });
    assert!(poll_once(run.as_mut()).is_pending());
    INPUT.with(|input| input.borrow_mut().extend("é".bytes()));
    INPUT.with(|input| input.borrow_mut().extend([0xe2, b'\n']));
    assert!(poll_once(run.as_mut()).is_pending());
    OUTPUT.with(|output| assert!(output.borrow().contains("Aé: command not found")));
}

#[test]
fn accepts_line_endings() {
    let mut shell = AsyncShell::new(write, poll_read);
    let mut run = pin!(shell.run());
    for bytes in [b"lf\ncr\rcrlf\r".as_slice(), b"\n\n\r\r"] {
        INPUT.with(|input| input.borrow_mut().extend(bytes));
        assert!(poll_once(run.as_mut()).is_pending());
    }
    OUTPUT.with(|output| {
        let output = output.borrow();
        assert_eq!(output.matches("command not found").count(), 3);
        assert_eq!(output.matches("> ").count(), 7);
    });
}

#[test]
fn ctrl_d_ends_empty_lines() {
    let commands = commands! { "echo" => |argv, shell| writeln!(shell, "{}", argv[1]) };
    let mut shell = AsyncShell::new(write, poll_read).with_commands(&commands);
    // Ctrl-D is ignored while the line has input.
    INPUT.with(|input| input.borrow_mut().extend(b"echo a\x04\n\x04ignored"));
    assert!(matches!(smol::block_on(shell.run()), Ok(())));
    OUTPUT.with(|output| assert!(output.borrow().ends_with("a\n> \n")));
    INPUT.with(|input| input.borrow_mut().clear());
}

#[test]
fn joins_continued_lines() {
    let commands = commands! {
        "echo" => |argv, shell| writeln!(shell, "[{}]", argv[1..].join(" ")),
    };
    let mut shell = AsyncShell::new(write, poll_read).with_commands(&commands);
    let mut run = pin!(shell.run());
    // Backspace and left stop at the continuation, and the backslash is removed.
    INPUT.with(|input| {
        input
            .borrow_mut()
            .extend(b"echo a\\\r\nb\x7f\x7f\x1b[Dc \\\nd\n")
    });
    assert!(poll_once(run.as_mut()).is_pending());
    OUTPUT.with(|output| {
        let output = output.borrow();
        assert_eq!(output.matches("> ").count(), 2, "{output:?}");
        // Continuations are indented four columns past the two-column prompt.
        assert!(output.contains("a\\\n      b"), "{output:?}");
        assert!(output.contains("c \\\n      d"), "{output:?}");
        assert!(output.ends_with("[ac d]\n> "), "{output:?}");
    });
}

#[test]
fn blocking_shell_uses_custom_prompt() {
    let commands = commands! {
        "help" => |_, shell| shell.print_help_screen(),
        "stop" => |_, shell| {
            FAIL_AFTER.set(WRITE_CALLS.get());
            shell.print_prompt();
        },
    };
    FAIL_AFTER.set(usize::MAX);
    INPUT.with(|input| input.borrow_mut().extend(b"partial\x1b\x03help\r\nstop\n"));
    let mut shell = Shell::new(fallible_write, read)
        .with_commands(&commands)
        .with_prompt(&os_prompt);
    assert!(matches!(shell.run(), Err(Error::Message("output failed"))));
    OUTPUT.with(|output| {
        let output = output.borrow();
        assert!(output.starts_with("os> "));
        assert!(output.contains("^C\nos> "));
        assert!(output.contains("Commands:"));
    });
}

#[test]
fn output_errors_stop_the_shell() {
    for async_mode in [false, true] {
        for fail_after in [0, 1, 6] {
            WRITE_CALLS.set(0);
            FAIL_AFTER.set(fail_after);
            INPUT.with(|input| {
                *input.borrow_mut() = b"help\n".iter().copied().collect();
            });
            let commands = commands! {
                "help" => |_, shell| write!(shell, "value: {}", 42),
            };
            if async_mode {
                let mut shell = AsyncShell::new(fallible_write, poll_read)
                    .with_error_handler(&unexpected_error)
                    .with_commands(&commands);
                let mut run = pin!(shell.run());
                assert!(matches!(
                    poll_once(run.as_mut()),
                    Poll::Ready(Err(Error::Message("output failed")))
                ));
            } else {
                let mut shell = Shell::new(fallible_write, read)
                    .with_error_handler(&unexpected_error)
                    .with_commands(&commands);
                assert!(matches!(shell.run(), Err(Error::Message("output failed"))));
            }
            assert_eq!(WRITE_CALLS.get(), fail_after + 1);
            if fail_after == 0 {
                INPUT.with(|input| assert_eq!(input.borrow().len(), 5));
            }
        }
    }
}

#[test]
fn submission_error_prevents_dispatch() {
    for async_mode in [false, true] {
        WRITE_CALLS.set(0);
        FAIL_AFTER.set(2);
        INPUT.with(|input| input.borrow_mut().extend(b"x\n"));
        let commands = commands! {
            "x" => |_, _| -> () { panic!("command ran after output failure") },
        };
        if async_mode {
            let mut shell = AsyncShell::new(fallible_write, poll_read).with_commands(&commands);
            let mut run = pin!(shell.run());
            assert!(matches!(
                poll_once(run.as_mut()),
                Poll::Ready(Err(Error::Message("output failed")))
            ));
        } else {
            let mut shell = Shell::new(fallible_write, read).with_commands(&commands);
            assert!(matches!(shell.run(), Err(Error::Message("output failed"))));
        }
        assert_eq!(WRITE_CALLS.get(), 3);
    }
}
