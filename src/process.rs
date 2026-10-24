use std::{
    fs::File,
    io::{Read, Seek},
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

pub fn handle_interrupts() -> Result<()> {
    ctrlc::set_handler(|| INTERRUPTED.store(true, Ordering::Relaxed))?;
    Ok(())
}

pub struct Output {
    pub status: Option<ExitStatus>,
    pub stdout: String,
    pub stderr: String,
    pub elapsed_ms: u64,
}

impl Output {
    pub fn success(&self) -> bool {
        self.status.is_some_and(|status| status.success())
    }

    pub fn diagnostic(&self) -> String {
        format!("{}\n{}", self.stderr, self.stdout)
            .chars()
            .take(8_000)
            .collect()
    }
}

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        // Cargo and tests can spawn children; kill the whole group on timeout.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn run(command: &mut Command, timeout: Duration) -> Result<Output> {
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    command
        .stdin(Stdio::null())
        .stdout(stdout.try_clone()?)
        .stderr(stderr.try_clone()?);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let start = Instant::now();
    let mut child = Running(
        command
            .spawn()
            .with_context(|| format!("start {command:?}"))?,
    );
    let status = loop {
        ensure!(!INTERRUPTED.load(Ordering::Relaxed), "analysis interrupted");
        if let Some(status) = child.0.try_wait()? {
            break Some(status);
        }
        if start.elapsed() >= timeout {
            break None;
        }
        thread::sleep(Duration::from_millis(10));
    };
    drop(child);
    Ok(Output {
        status,
        stdout: read(&mut stdout)?,
        stderr: read(&mut stderr)?,
        elapsed_ms: start.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
    })
}

fn read(file: &mut File) -> Result<String> {
    file.rewind()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
