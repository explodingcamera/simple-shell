#[cfg(feature = "std")]
#[test]
fn preserves_io_errors() {
    use simple_shell::Error;
    use std::{error::Error as _, io};

    let mut shell = simple_shell::Shell::new(
        |_| Err::<(), _>(io::Error::new(io::ErrorKind::PermissionDenied, "denied")),
        |_| panic!("read after output failure"),
    );
    let error = match shell.run() {
        Err(error) => error,
        Ok(()) => panic!("expected an output error"),
    };
    assert_eq!(error.to_string(), "denied");
    assert!(error.source().is_some());
    match error {
        Error::Io(error) => assert_eq!(error.kind(), io::ErrorKind::PermissionDenied),
        other => panic!("expected an I/O error, got {other:?}"),
    }
}
