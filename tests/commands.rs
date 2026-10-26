use simple_shell::{
    AsyncShell, CommandList, Commands, Shell, ShellCommand, ShellContext, SyncCommands, commands,
};

fn noop(_: &[&str], _: &mut ShellContext<'_>) {}

#[test]
fn empty_names_do_not_match() {
    let unnamed = ShellCommand::new(&[], |_, _| Err("unnamed command ran"));
    let named = ShellCommand::new(&["ok"], |_, shell| {
        shell.data += 1usize;
    });
    let fallback = ShellCommand::new(&[""], |_, shell| {
        shell.data += 10usize;
    });
    let commands = commands! { unnamed, named, fallback };
    let list: CommandList<'_, (), usize> = &[&unnamed, &named, &fallback];
    for set in [&commands as &dyn SyncCommands<(), usize>, &list] {
        let mut shell = Shell::new(|_| {}, |_| Ok(0)).with_state(0usize);
        set.run_sync(&[], &mut shell.context())
            .expect("empty input is ignored");
        set.run_sync(&["ok"], &mut shell.context())
            .expect("named command succeeds");
        set.run_sync(&["missing"], &mut shell.context())
            .expect("fallback succeeds");
        assert_eq!(shell.data, 11);
    }
    let mut shell = Shell::new(|_| {}, |_| Ok(0)).with_state(0usize);
    smol::block_on(commands.run(&["ok"], &mut shell.context())).expect("named command succeeds");
    assert_eq!(shell.data, 1);
    smol::block_on(().run(&[], &mut shell.context())).expect("empty input is ignored");
    ().run_sync(&[], &mut shell.context())
        .expect("empty input is ignored");
}

#[test]
fn infers_sync_state() {
    let commands = const {
        commands! {
            /// Increment the counter.
            "increment" | "i" => |_, shell| { shell.data += 1; },
            "help" => |_, shell| shell.print_help_screen(),
        }
    };
    let mut shell = Shell::new(|_| {}, |_| Ok(0))
        .with_state(0usize)
        .with_commands(&commands);
    commands
        .run_sync(&["i"], &mut shell.context())
        .expect("handler succeeds");
    assert_eq!(shell.data, 1);
    let mut help = String::new();
    commands
        .write_help(&mut help)
        .expect("String output succeeds");
    assert!(help.contains("Increment the counter."));
}

#[test]
fn builds_const_async_commands() {
    let commands = const {
        commands! {
            /// Show commands.
            "help" | "h" => async |_, shell| shell.print_help_screen(),
            "panic" => async |_, _| -> () { panic!("test panic") },
            /// Return a result.
            "result" => async |_, _| -> simple_shell::Result { Ok(()) },
            "sync" => noop,
            _ => async |_, _| {},
        }
    };
    let mut help = String::new();
    commands
        .write_help(&mut help)
        .expect("String output succeeds");
    assert!(help.contains("Show commands."));
    assert!(help.contains("aliases: h"));
    assert!(help.contains("Return a result."));
    let mut shell =
        AsyncShell::new(|_| {}, |_, _| core::task::Poll::Pending).with_commands(&commands);
    let future = shell.run();
    let _: &dyn Send = &future;
}

#[test]
fn async_move_captures_state() {
    let value = 42usize;
    let commands = commands! {
        "value" => async move |_, shell| { shell.data = value; },
    };
    let mut shell = AsyncShell::new(|_| {}, |_, _| core::task::Poll::Pending).with_state(0usize);
    let mut ctx = shell.context();
    let future = commands.run(&["value"], &mut ctx);
    let _: &dyn Send = &future;
    smol::block_on(future).expect("handler succeeds");
    assert_eq!(shell.data, value);
}

#[test]
fn calls_function_paths() {
    fn increment(_: &[&str], shell: &mut ShellContext<'_, (), usize>) {
        shell.data += 1;
    }
    async fn increment_async(argv: &[&str], shell: &mut ShellContext<'_, (), usize>) {
        increment(argv, shell);
    }
    struct Handlers;
    impl Handlers {
        fn increment(argv: &[&str], shell: &mut ShellContext<'_, (), usize>) {
            increment(argv, shell);
        }
        async fn increment_async(argv: &[&str], shell: &mut ShellContext<'_, (), usize>) {
            increment_async(argv, shell).await;
        }
    }
    let commands = const {
        commands! {
            "function" => increment,
            "method" => Handlers::increment,
            "async-function" => async increment_async,
            "async-method" | "alias" => async Handlers::increment_async,
        }
    };
    let mut shell = AsyncShell::new(|_| {}, |_, _| core::task::Poll::Pending).with_state(0usize);
    for name in ["function", "method", "async-function", "alias"] {
        smol::block_on(commands.run(&[name], &mut shell.context())).expect("handler succeeds");
    }
    assert_eq!(shell.data, 4);
}
