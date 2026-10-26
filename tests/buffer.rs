#![cfg(feature = "heapless")]

use std::{cell::RefCell, collections::VecDeque};

use simple_shell::{Shell, commands};

thread_local! {
    static INPUT: RefCell<VecDeque<u8>> = const { RefCell::new(VecDeque::new()) };
    static OUTPUT: RefCell<String> = const { RefCell::new(String::new()) };
}

fn write(text: &str) {
    OUTPUT.with(|output| output.borrow_mut().push_str(text));
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

#[test]
fn rejects_overflow_and_recovers() {
    let mut input = Vec::new();
    // A rejected line must never dispatch its otherwise valid prefix.
    input.extend_from_slice(b"increment ");
    input.extend(std::iter::repeat_n(b'x', 1024));
    input.extend_from_slice(b"\x7f\x1b[\n");
    input.extend_from_slice(b"increment ");
    input.extend_from_slice("é".repeat(512).as_bytes());
    input.extend_from_slice(b"\n");
    input.extend_from_slice(b"increment ");
    input.extend_from_slice(b"x ".repeat(64).as_slice());
    input.extend_from_slice(b"\n");
    input.extend_from_slice(b"increment ");
    input.extend(std::iter::repeat_n(b'x', 1024));
    input.extend_from_slice(b"\x03increment\n");
    INPUT.with(|queue| queue.borrow_mut().extend(input));
    let commands = commands! {
        "increment" => |_, shell| { shell.data += 1; },
    };
    let mut shell = Shell::new(write, read)
        .with_state(0usize)
        .with_commands(&commands)
        .with_buffer(heapless::String::<1024>::new());
    shell.run().expect("output succeeds");
    // Without alloc, arguments are limited to 64, including the command name.
    assert_eq!(shell.data, if cfg!(feature = "alloc") { 2 } else { 1 });
    OUTPUT.with(|output| {
        let output = output.borrow();
        assert_eq!(output.matches("command buffer is full").count(), 2);
        assert_eq!(
            output.contains("command exceeds 64 arguments"),
            !cfg!(feature = "alloc")
        );
    });
}
