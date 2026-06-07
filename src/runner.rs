use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use std::os::unix::process::CommandExt;

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

use crate::repo::RepoId;

/// Which output stream a line came from. Used to color stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Execution events flowing from workers to the UI.
#[derive(Debug, Clone)]
pub enum RunnerEvent {
    Started {
        repo: RepoId,
        command: String,
    },
    Line {
        repo: RepoId,
        stream: Stream,
        text: String,
    },
    Finished {
        repo: RepoId,
        /// Exit code on normal exit. None when killed by a signal (incl. cancel kill).
        code: Option<i32>,
    },
    /// Repo that had not started when cancel was requested (never ran).
    Skipped {
        repo: RepoId,
    },
    Error {
        repo: RepoId,
        message: String,
    },
}

/// An execution target.
pub type Target = (RepoId, PathBuf);

/// Execution dispatcher. Tracks running children by process group to enable cancel.
pub struct Runner {
    /// pgids of running children (each child leads its own group). cancel does `kill(-pgid)`.
    pgids: Arc<Mutex<Vec<i32>>>,
    /// Cancel-request flag; the dispatcher checks it to stop spawning the rest.
    cancel: Arc<AtomicBool>,
}

impl Runner {
    pub fn new() -> Self {
        Self {
            pgids: Arc::new(Mutex::new(Vec::new())),
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Run `command` across all `targets` concurrently; events flow to the returned
    /// Receiver. `concurrency` gates the number of simultaneous spawns.
    pub fn start(
        &self,
        command: String,
        targets: Vec<Target>,
        concurrency: usize,
    ) -> Receiver<RunnerEvent> {
        let (tx, rx) = mpsc::channel();
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let pgids = Arc::clone(&self.pgids);
        pgids.lock().unwrap().clear();
        let cancel = Arc::clone(&self.cancel);
        cancel.store(false, Ordering::SeqCst);

        let permits = concurrency.max(1);
        let (permit_tx, permit_rx) = mpsc::channel::<()>();
        for _ in 0..permits {
            let _ = permit_tx.send(());
        }

        thread::spawn(move || {
            for (repo, path) in targets {
                // Already cancelled: don't start, emit Skipped (always settle pending).
                if cancel.load(Ordering::SeqCst) {
                    let _ = tx.send(RunnerEvent::Skipped { repo });
                    continue;
                }
                // Wait for a permit (exit if the channel was dropped).
                if permit_rx.recv().is_err() {
                    break;
                }
                // Re-check: cancel may have fired while waiting for the permit.
                if cancel.load(Ordering::SeqCst) {
                    let _ = permit_tx.send(());
                    let _ = tx.send(RunnerEvent::Skipped { repo });
                    continue;
                }
                let tx = tx.clone();
                let permit_tx = permit_tx.clone();
                let pgids = Arc::clone(&pgids);
                let shell = shell.clone();
                let command = command.clone();
                thread::spawn(move || {
                    run_one(repo, path, shell, command, tx, pgids);
                    let _ = permit_tx.send(());
                });
            }
        });

        rx
    }

    /// Request cancel: stop further spawns and SIGTERM running children by process group.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        let pgids = self.pgids.lock().unwrap();
        for &pgid in pgids.iter() {
            let _ = kill(Pid::from_raw(-pgid), Signal::SIGTERM);
        }
    }
}

/// Launch the command in one repo and stream stdout/stderr from separate threads (reader A).
fn run_one(
    repo: RepoId,
    path: PathBuf,
    shell: String,
    command: String,
    tx: Sender<RunnerEvent>,
    pgids: Arc<Mutex<Vec<i32>>>,
) {
    let _ = tx.send(RunnerEvent::Started {
        repo,
        command: command.clone(),
    });

    let mut cmd = Command::new(&shell);
    cmd.arg("-c")
        .arg(&command)
        .current_dir(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Disable pager / prompts so it doesn't hang without a TTY.
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        // Lead its own process group so cancel can reach grandchildren.
        .process_group(0);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(RunnerEvent::Error {
                repo,
                message: e.to_string(),
            });
            return;
        }
    };

    // With process_group(0), pgid == child pid.
    let pgid = child.id() as i32;
    pgids.lock().unwrap().push(pgid);

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = spawn_reader(repo, Stream::Stdout, stdout, tx.clone());
    let stderr_reader = spawn_reader(repo, Stream::Stderr, stderr, tx.clone());

    let code = child.wait().ok().and_then(|s| s.code());

    let _ = stdout_reader.join();
    let _ = stderr_reader.join();

    pgids.lock().unwrap().retain(|&p| p != pgid);

    let _ = tx.send(RunnerEvent::Finished { repo, code });
}

/// Spawn a thread that reads the pipe line by line and emits Line events.
fn spawn_reader<R>(
    repo: RepoId,
    stream: Stream,
    handle: Option<R>,
    tx: Sender<RunnerEvent>,
) -> thread::JoinHandle<()>
where
    R: std::io::Read + Send + 'static,
{
    thread::spawn(move || {
        let Some(handle) = handle else {
            return;
        };
        let reader = BufReader::new(handle);
        for line in reader.lines() {
            match line {
                Ok(text) => {
                    if tx.send(RunnerEvent::Line { repo, stream, text }).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_command_and_streams_output() {
        let runner = Runner::new();
        let target = (0usize, std::env::temp_dir());
        let rx = runner.start("echo hello && echo oops 1>&2".to_string(), vec![target], 4);

        let mut started = false;
        let mut stdout_lines = Vec::new();
        let mut stderr_lines = Vec::new();
        let mut code = None;
        while let Ok(ev) = rx.recv() {
            match ev {
                RunnerEvent::Started { .. } => started = true,
                RunnerEvent::Line {
                    stream: Stream::Stdout,
                    text,
                    ..
                } => stdout_lines.push(text),
                RunnerEvent::Line {
                    stream: Stream::Stderr,
                    text,
                    ..
                } => stderr_lines.push(text),
                RunnerEvent::Finished { code: c, .. } => {
                    code = c;
                    break;
                }
                RunnerEvent::Skipped { .. } => {}
                RunnerEvent::Error { message, .. } => panic!("spawn error: {message}"),
            }
        }

        assert!(started);
        assert_eq!(stdout_lines, vec!["hello"]);
        assert_eq!(stderr_lines, vec!["oops"]);
        assert_eq!(code, Some(0));
    }

    #[test]
    fn cancel_terminates_all_running_and_skips_pending() {
        let runner = Runner::new();
        let tmp = std::env::temp_dir();
        // 6 targets at concurrency 2: at cancel time ~2 running, ~4 not started.
        let targets: Vec<Target> = (0..6).map(|i| (i, tmp.clone())).collect();
        let rx = runner.start("sleep 30".to_string(), targets, 2);

        std::thread::sleep(std::time::Duration::from_millis(200));
        runner.cancel();

        // All 6 repos receive exactly one terminal event (Finished/Skipped/Error).
        let mut terminal = std::collections::HashSet::new();
        let mut skipped = 0;
        let deadline = std::time::Duration::from_secs(3);
        while terminal.len() < 6 {
            match rx.recv_timeout(deadline) {
                Ok(RunnerEvent::Finished { repo, .. }) | Ok(RunnerEvent::Error { repo, .. }) => {
                    terminal.insert(repo);
                }
                Ok(RunnerEvent::Skipped { repo }) => {
                    terminal.insert(repo);
                    skipped += 1;
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        assert_eq!(terminal.len(), 6, "every repo must reach a terminal state");
        assert!(skipped > 0, "pending repos should be skipped, not started");
    }
}
