//! Bounded execution of owner-accepted checkers, and the receipt they leave.
//!
//! The runner is deliberately narrow: a literal argv (no shell), a cleared
//! environment supplied by the caller, a repository working directory, a
//! process group, a wall-clock bound and bounded output capture. It is not an
//! OS security boundary; a receipt says so.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const RECEIPT_SCHEMA: &str = "whetstone.execution-receipt.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Success,
    Violated,
    Unknown,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionReceipt {
    pub schema: String,
    pub checker_id: String,
    pub checker_version: String,
    pub manifest_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_grant_reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_checked_at: Option<String>,
    pub state: ExecutionState,
    pub reason_code: String,
    pub summary: String,
    pub process_started: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_digest: Option<String>,
    #[serde(default)]
    pub consumed_configurations: Vec<ConsumedConfiguration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
    pub limits: LimitEvidence,
    pub process_group_cleanup: String,
    pub isolation_caveat: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumedConfiguration {
    pub id: String,
    pub path: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CapturedOutput {
    pub text: String,
    pub bytes_captured: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitEvidence {
    pub timeout_ms: u64,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub timeout_mechanism: String,
    pub output_mechanism: String,
    pub filesystem_network_memory_process_limits: String,
}

pub fn file_digest(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        hash.update(&chunk[..read]);
    }
    Ok(format!("sha256:{:x}", hash.finalize()))
}

pub(crate) struct RunResult {
    pub(crate) status: ExitStatus,
    pub(crate) elapsed: Duration,
    pub(crate) stdout: CapturedOutput,
    pub(crate) stderr: CapturedOutput,
    pub(crate) timed_out: bool,
}

/// Run a literal argv (no shell) with a cleared environment, a process
/// group, a wall-clock bound and bounded output capture.
pub(crate) fn run_bounded(
    executable: &Path,
    argv: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> io::Result<RunResult> {
    let mut command = Command::new(executable);
    command
        .args(argv)
        .current_dir(cwd)
        .env_clear()
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let pid = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("stdout pipe unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr pipe unavailable"))?;
    let exceeded = Arc::new(AtomicBool::new(false));
    let stdout_reader = spawn_reader(stdout, stdout_limit, Arc::clone(&exceeded));
    let stderr_reader = spawn_reader(stderr, stderr_limit, Arc::clone(&exceeded));
    let began = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if began.elapsed() >= timeout || exceeded.load(Ordering::Relaxed) {
            timed_out = began.elapsed() >= timeout;
            terminate_process_tree(&mut child, pid);
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(5));
    };
    // A checker may leave descendants holding inherited pipes. Kill the fresh
    // process group after its leader exits so reader joins remain bounded.
    cleanup_process_group(pid);
    let stdout = stdout_reader
        .join()
        .map_err(|_| io::Error::other("stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| io::Error::other("stderr reader panicked"))??;
    Ok(RunResult {
        status,
        elapsed: began.elapsed(),
        stdout,
        stderr,
        timed_out,
    })
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
    exceeded: Arc<AtomicBool>,
) -> thread::JoinHandle<io::Result<CapturedOutput>> {
    thread::spawn(move || {
        let mut bytes = Vec::with_capacity(limit.min(16 * 1024));
        let mut chunk = [0_u8; 8 * 1024];
        let mut truncated = false;
        loop {
            let read = reader.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            let remaining = limit.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..read.min(remaining)]);
            if read > remaining {
                truncated = true;
                exceeded.store(true, Ordering::Relaxed);
                break;
            }
        }
        Ok(CapturedOutput {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            bytes_captured: bytes.len(),
            truncated,
        })
    })
}

fn terminate_process_tree(child: &mut Child, pid: u32) {
    #[cfg(unix)]
    unsafe {
        kill(-(pid as i32), 9);
    }
    let _ = child.kill();
}

fn cleanup_process_group(pid: u32) -> String {
    #[cfg(unix)]
    {
        // SAFETY: a negative, freshly spawned process-group ID targets only the
        // checker group established by `CommandExt::process_group(0)`.
        unsafe {
            kill(-(pid as i32), 9);
        }
        "hard:unix-process-group-sigkill".into()
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        "advisory:direct-child-only-on-this-platform".into()
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Digest of an in-memory output, in the receipt's `sha256:` form.
pub fn output_digest(bytes: &[u8]) -> String {
    sha256_bytes(bytes)
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn sh() -> PathBuf {
        PathBuf::from("/bin/sh")
    }

    fn run(script: &str, timeout: Duration, limit: usize) -> RunResult {
        let temp = tempfile::tempdir().expect("temp");
        run_bounded(
            &sh(),
            &["-c".to_string(), script.to_string()],
            temp.path(),
            &BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
            timeout,
            limit,
            limit,
        )
        .expect("spawn")
    }

    #[test]
    fn a_literal_argv_runs_without_a_shell_expanding_it() {
        let temp = tempfile::tempdir().expect("temp");
        let result = run_bounded(
            Path::new("/bin/echo"),
            &["$(touch should-not-run)".to_string()],
            temp.path(),
            &BTreeMap::new(),
            Duration::from_secs(5),
            4_096,
            4_096,
        )
        .expect("spawn");
        assert!(result.status.success());
        assert_eq!(result.stdout.text.trim(), "$(touch should-not-run)");
        assert!(!temp.path().join("should-not-run").exists());
    }

    #[test]
    fn the_environment_is_exactly_what_the_caller_allows() {
        std::env::set_var("WHETSTONE_TEST_LEAK", "leaked");
        let result = run(
            "echo \"[$WHETSTONE_TEST_LEAK]\"",
            Duration::from_secs(5),
            4_096,
        );
        assert_eq!(result.stdout.text.trim(), "[]");
    }

    #[test]
    fn a_timeout_stops_the_process_and_is_reported() {
        let result = run("sleep 5", Duration::from_millis(100), 4_096);
        assert!(result.timed_out);
        assert!(!result.status.success());
        assert!(result.elapsed < Duration::from_secs(4));
    }

    #[test]
    fn output_beyond_the_limit_is_truncated_and_stops_the_run() {
        let result = run("yes whetstone", Duration::from_secs(5), 1_024);
        assert!(result.stdout.truncated);
        assert_eq!(result.stdout.bytes_captured, 1_024);
        assert!(!result.timed_out);
    }

    #[test]
    fn descendants_holding_the_pipes_are_killed_with_the_group() {
        let began = Instant::now();
        let result = run(
            "(sleep 30; echo late) & echo early",
            Duration::from_secs(5),
            4_096,
        );
        assert_eq!(result.stdout.text.trim(), "early");
        assert!(began.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn file_digests_are_sha256() {
        let temp = tempfile::tempdir().expect("temp");
        let path = temp.path().join("f");
        std::fs::write(&path, b"abc").expect("write");
        assert_eq!(
            file_digest(&path).expect("digest"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
