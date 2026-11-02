//! Bounded, cancellable execution of `jik check`.
//!
//! Dropping the returned future kills the direct child, not its descendants.

use std::{
    ffi::OsStr,
    fmt, io,
    path::Path,
    process::ExitStatus,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time,
};

/// Captured bytes are left undecoded, including for unsuccessful exit statuses.
#[derive(Debug)]
pub struct CheckOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
pub enum CheckError {
    Spawn(io::Error),
    Io(io::Error),
    Timeout,
    OutputLimit { limit: usize },
}

impl fmt::Display for CheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "failed to spawn jik check: {e}"),
            Self::Io(e) => write!(f, "jik check I/O failed: {e}"),
            Self::Timeout => write!(f, "jik check timed out"),
            Self::OutputLimit { limit } => write!(f, "jik check output exceeded {limit} bytes"),
        }
    }
}

impl std::error::Error for CheckError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn(e) | Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// Run `executable check entry` without a shell. `output_limit` is the total
/// number of bytes permitted across both streams, not a per-stream limit.
/// The deadline covers waiting for the child and EOF on both pipes.
pub async fn run_check(
    executable: impl AsRef<OsStr>,
    entry: impl AsRef<Path>,
    timeout: Duration,
    output_limit: usize,
) -> Result<CheckOutput, CheckError> {
    let mut child = Command::new(executable)
        .arg("check")
        .arg(entry.as_ref())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(CheckError::Spawn)?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let used = AtomicUsize::new(0);
    time::timeout(timeout, async {
        let (status, stdout, stderr) = tokio::try_join!(
            async { child.wait().await.map_err(CheckError::Io) },
            read_bounded(stdout, &used, output_limit),
            read_bounded(stderr, &used, output_limit),
        )?;
        Ok(CheckOutput {
            status,
            stdout,
            stderr,
        })
    })
    .await
    .map_err(|_| CheckError::Timeout)?
}

async fn read_bounded(
    mut input: impl AsyncRead + Unpin,
    used: &AtomicUsize,
    limit: usize,
) -> Result<Vec<u8>, CheckError> {
    let mut result = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = input.read(&mut buf).await.map_err(CheckError::Io)?;
        if n == 0 {
            return Ok(result);
        }
        let mut previous = used.load(Ordering::Relaxed);
        loop {
            let next = previous
                .checked_add(n)
                .filter(|&total| total <= limit)
                .ok_or(CheckError::OutputLimit { limit })?;
            match used.compare_exchange_weak(previous, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(actual) => previous = actual,
            }
        }
        result.extend_from_slice(&buf[..n]);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    // Concurrent fork/exec can transiently inherit another test's writable script
    // descriptor and cause ETXTBSY when that script is executed.
    static PROCESS_TEST: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    struct Fake(PathBuf);
    impl Fake {
        fn new(body: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "jik-check-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            let executable = path.join("fake jik");
            fs::write(&executable, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path)
        }
        fn executable(&self) -> PathBuf {
            self.0.join("fake jik")
        }
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn arguments_and_nonzero_status() {
        let _guard = PROCESS_TEST.lock().await;
        let fake = Fake::new("printf '%s\\n%s' \"$1\" \"$2\"; printf problem >&2; exit 7");
        let result = run_check(
            fake.executable(),
            "entry ; $HOME.jik",
            Duration::from_secs(2),
            128,
        )
        .await
        .unwrap();
        assert_eq!(result.status.code(), Some(7));
        assert_eq!(result.stdout, b"check\nentry ; $HOME.jik");
        assert_eq!(result.stderr, b"problem");
    }

    #[tokio::test]
    async fn combined_limit_and_exact_boundary() {
        let _guard = PROCESS_TEST.lock().await;
        let fake = Fake::new("printf 1234; printf 5678 >&2");
        let exact = run_check(fake.executable(), "x", Duration::from_secs(2), 8).await;
        assert!(exact.is_ok(), "exact boundary failed: {exact:?}");
        assert!(matches!(
            run_check(fake.executable(), "x", Duration::from_secs(2), 7).await,
            Err(CheckError::OutputLimit { limit: 7 })
        ));
        assert!(matches!(
            run_check(fake.executable(), "x", Duration::from_secs(2), 0).await,
            Err(CheckError::OutputLimit { limit: 0 })
        ));
    }

    #[tokio::test]
    async fn timeout_and_missing_program() {
        let _guard = PROCESS_TEST.lock().await;
        let fake = Fake::new("exec sleep 30");
        let result = run_check(fake.executable(), "x", Duration::from_millis(50), 128).await;
        assert!(
            matches!(result, Err(CheckError::Timeout)),
            "unexpected result: {result:?}"
        );
        assert!(
            matches!(run_check(fake.0.join("missing"), "x", Duration::from_secs(2), 128).await, Err(CheckError::Spawn(e)) if e.kind() == io::ErrorKind::NotFound)
        );
    }
    #[tokio::test]
    async fn real_compiler_diagnostic_contract() {
        let _guard = PROCESS_TEST.lock().await;
        let compiler = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../jik/jik");
        if !compiler.is_file() {
            eprintln!(
                "skipping real compiler test: {} not found",
                compiler.display()
            );
            return;
        }
        let temp = Fake::new(":");
        let entry = temp.0.join("invalid source.jik");
        fs::write(&entry, "func main() -> int:\n    return unknown\nend\n").unwrap();
        let output = run_check(&compiler, &entry, Duration::from_secs(10), 1024 * 1024)
            .await
            .unwrap();
        assert!(!output.status.success());
        let parsed = crate::diagnostics::parse(std::str::from_utf8(&output.stderr).unwrap());
        assert!(
            parsed
                .located
                .iter()
                .any(|diagnostic| diagnostic.path == entry
                    && diagnostic.line == 1
                    && diagnostic.byte_column == 11
                    && diagnostic.message.contains("undefined symbol")),
            "{parsed:?}"
        );
    }

    #[tokio::test]
    async fn cancellation_kills_child() {
        let _guard = PROCESS_TEST.lock().await;
        let fake = Fake::new("echo $$ > \"$2\"; exec sleep 30");
        let pidfile = fake.0.join("pid");
        let mut task = tokio::spawn(run_check(
            fake.executable(),
            pidfile.clone(),
            Duration::from_secs(30),
            128,
        ));
        let pid = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(pid) = fs::read_to_string(&pidfile) {
                    if !pid.trim().is_empty() {
                        break pid.trim().to_owned();
                    }
                }
                tokio::select! {
                    result = &mut task => panic!("child exited before writing pid: {result:?}"),
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {}
                }
            }
        })
        .await
        .expect("child did not write pid within deadline");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if !std::process::Command::new("kill")
                    .args(["-0", &pid])
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap()
                    .success()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("child was not killed and reaped");
    }
}
