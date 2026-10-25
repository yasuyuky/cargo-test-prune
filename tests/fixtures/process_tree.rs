pub fn work() {
    ()
}

fn wait_for_descendant() {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "descendant", "--ignored"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap()
        .wait()
        .unwrap();
}

#[test]
fn runs_work() {
    work();
}

#[test]
#[ignore]
fn descendant() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(std::env::var_os("TEST_PRUNE_SOCKET").unwrap()).unwrap();
    writeln!(stream, "{}", std::process::id()).unwrap();
    writeln!(stream, "{}", std::env::current_dir().unwrap().display()).unwrap();
    // The observer keeps this socket open, so only process termination closes it.
    stream.read_exact(&mut [0]).unwrap();
}
