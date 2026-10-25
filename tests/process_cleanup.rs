#![cfg(unix)]

use std::{
    fs,
    io::{BufRead, BufReader, ErrorKind, Read},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::json;
use tempfile::TempDir;

const SOURCE: &str = include_str!("fixtures/process_tree.rs");

struct Analysis {
    _directory: TempDir,
    project: PathBuf,
    log: PathBuf,
    listener: UnixListener,
    child: Child,
    descendant_group: Option<i32>,
}

impl Analysis {
    fn start(source: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        let scratch = directory.path().join("scratch");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::create_dir(&scratch).unwrap();
        fs::write(
            project.join("Cargo.toml"),
            include_str!("fixtures/overlap/Cargo.toml"),
        )
        .unwrap();
        fs::write(project.join("src/lib.rs"), source).unwrap();
        fs::write(
            project.join("mutants.json"),
            json!([{
                "name": "wait for a descendant",
                "file": "src/lib.rs",
                "span": {"start": {"line": 2, "column": 5}, "end": {"line": 2, "column": 7}},
                "replacement": "wait_for_descendant()",
            }])
            .to_string(),
        )
        .unwrap();
        let socket = directory.path().join("worker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let log = directory.path().join("stderr.log");
        let child = Command::new(env!("CARGO_BIN_EXE_cargo-test-prune"))
            .current_dir(&project)
            .env("CARGO_NET_OFFLINE", "true")
            .env("TMPDIR", &scratch)
            .env("TEST_PRUNE_SOCKET", socket)
            .args([
                "analyze",
                "--mutants-file",
                "mutants.json",
                "--timeout",
                "2",
            ])
            .stdout(Stdio::null())
            .stderr(fs::File::create(&log).unwrap())
            .spawn()
            .unwrap();
        Self {
            _directory: directory,
            project,
            log,
            listener,
            child,
            descendant_group: None,
        }
    }

    fn descendant(&mut self) -> (UnixStream, PathBuf) {
        let deadline = Instant::now() + Duration::from_secs(20);
        let stream = loop {
            match self.listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => panic!("accept descendant: {error}"),
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "analysis exited before descendant started:\n{}",
                fs::read_to_string(&self.log).unwrap()
            );
            assert!(Instant::now() < deadline, "descendant did not connect");
            thread::sleep(Duration::from_millis(10));
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let pid = line.trim().parse().unwrap();
        let group = unsafe { libc::getpgid(pid) };
        assert!(group > 0 && group != unsafe { libc::getpgrp() });
        self.descendant_group = Some(group);
        line.clear();
        reader.read_line(&mut line).unwrap();
        let workspace = PathBuf::from(line.trim());
        assert!(workspace.join("Cargo.toml").exists());
        assert_ne!(workspace, self.project.canonicalize().unwrap());
        (reader.into_inner(), workspace)
    }

    fn finish(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "analysis did not exit promptly");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Analysis {
    fn drop(&mut self) {
        // Failed mutation tests must not leave the fixture's process tree running.
        if let Some(group) = self.descendant_group {
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn assert_descendant_stopped(mut stream: UnixStream) {
    // EOF observes termination even when an orphan has not yet been reaped.
    assert_eq!(
        stream
            .read(&mut [0])
            .expect("descendant is still alive after analysis exited"),
        0
    );
}

#[test]
fn ctrl_c_stops_descendants_and_removes_the_workspace() {
    let source = SOURCE.replacen("    ()", "    wait_for_descendant()", 1);
    let mut analysis = Analysis::start(&source);
    let (stream, workspace) = analysis.descendant();
    unsafe {
        assert_eq!(libc::kill(analysis.child.id() as i32, libc::SIGINT), 0);
    }
    let status = analysis.finish();
    assert_descendant_stopped(stream);
    assert_eq!(
        status.code(),
        Some(1),
        "interruption should exit through normal cleanup"
    );
    assert!(
        fs::read_to_string(&analysis.log)
            .unwrap()
            .contains("analysis interrupted")
    );
    assert!(!workspace.exists(), "interrupted workspace was not removed");
    assert!(!analysis.project.join("test-prune.json").exists());
    assert_eq!(
        fs::read_to_string(analysis.project.join("src/lib.rs")).unwrap(),
        source
    );
}
