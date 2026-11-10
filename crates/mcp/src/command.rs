#![cfg_attr(
    test,
    expect(clippy::expect_used, reason = "a test aborts when a fixture fails")
)]
#![expect(
    clippy::missing_errors_doc,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::must_use_candidate,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![expect(
    clippy::redundant_closure_for_method_calls,
    reason = "TODO(lint-2026-10-14): pedantic backlog. docs/LINT_RULES.md"
)]
#![cfg_attr(
    test,
    expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")
)]

//! Named commands a client sends and the frame thread runs.
//!
//! A client (an MCP tool call, `POST /cmd/<name>`, or a script thread) queues a
//! command and blocks. The frame thread takes the queue at a fixed point in its
//! frame, runs each command, and replies, at once or after some frames (a wait,
//! a capture). The catalogue of commands is registered by whoever runs them, so
//! this crate stays free of engine types.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::json::Value;

/// One command the frame thread can run, as `tools/list` describes it.
#[derive(Clone, Debug)]
pub struct CommandSpec {
    pub name: String,
    pub description: String,
    /// JSON schema of the arguments object.
    pub schema: Value,
}

/// A queued command. Reply with [`reply`] and its id.
#[derive(Clone, Debug)]
pub struct Pending {
    pub id: u64,
    pub name: String,
    pub args: Value,
}

struct Queue {
    next: u64,
    waiting: Vec<Pending>,
    done: Vec<(u64, Result<Value, String>)>,
    specs: Vec<CommandSpec>,
    /// A frame thread has taken the queue at least once.
    served: bool,
}

static QUEUE: Mutex<Queue> = Mutex::new(Queue {
    next: 1,
    waiting: Vec::new(),
    done: Vec::new(),
    specs: Vec::new(),
    served: false,
});
static WAKE: Condvar = Condvar::new();

fn lock() -> std::sync::MutexGuard<'static, Queue> {
    QUEUE.lock().unwrap_or_else(|err| err.into_inner())
}

/// Publish the commands this process runs. MCP lists them as tools.
pub fn register(specs: Vec<CommandSpec>) {
    lock().specs = specs;
}

/// The registered commands.
pub fn specs() -> Vec<CommandSpec> {
    lock().specs.clone()
}

/// True when `name` is a registered command.
pub fn known(name: &str) -> bool {
    lock().specs.iter().any(|spec| spec.name == name)
}

/// Queue a command and wait for its reply. A command that waits on frames needs a
/// `timeout` that covers them.
pub fn call(name: &str, args: Value, timeout: Duration) -> Result<Value, String> {
    let id = {
        let mut queue = lock();
        if !queue.specs.iter().any(|spec| spec.name == name) {
            return Err(format!("unknown command {name}"));
        }
        let id = queue.next;
        queue.next += 1;
        queue.waiting.push(Pending {
            id,
            name: name.to_string(),
            args,
        });
        // Wake a host blocked in `take_wait`.
        WAKE.notify_all();
        id
    };
    let deadline = Instant::now() + timeout;
    let mut queue = lock();
    loop {
        if let Some(at) = queue.done.iter().position(|(done, _)| *done == id) {
            return queue.done.swap_remove(at).1;
        }
        let now = Instant::now();
        if now >= deadline {
            queue.waiting.retain(|pending| pending.id != id);
            return Err(if queue.served {
                format!("{name} did not finish in {:.1} s", timeout.as_secs_f32())
            } else {
                "no frame is running commands".to_string()
            });
        }
        let (guard, _) = WAKE
            .wait_timeout(queue, deadline.saturating_duration_since(now))
            .unwrap_or_else(|err| err.into_inner());
        queue = guard;
    }
}

static QUEUE_VIOLATIONS: AtomicU32 = AtomicU32::new(0);

/// Times a `genos-job-*` thread called [`take`] or [`take_wait`].
#[must_use]
pub fn queue_violations() -> u32 {
    QUEUE_VIOLATIONS.load(Ordering::Relaxed)
}

fn reject_job_thread() -> Result<(), &'static str> {
    let job = std::thread::current()
        .name()
        .is_some_and(|name| name.starts_with("genos-job-"));
    if job {
        QUEUE_VIOLATIONS.fetch_add(1, Ordering::Relaxed);
        Err("jobs.queue_violation")
    } else {
        Ok(())
    }
}

/// Commands queued since the last take, oldest first.
///
/// A `genos-job-*` thread is refused. The frame thread drains the queue, and
/// a job that waited here would stall that thread.
///
/// # Errors
///
/// `jobs.queue_violation` when the caller is a `genos-job-*` thread.
pub fn take() -> Result<Vec<Pending>, &'static str> {
    reject_job_thread()?;
    let mut queue = lock();
    queue.served = true;
    Ok(std::mem::take(&mut queue.waiting))
}

/// Like [`take`], but a host with no frame loop (a headless debug binary)
/// blocks here until a command arrives or `timeout` passes.
pub fn take_wait(timeout: Duration) -> Result<Vec<Pending>, &'static str> {
    reject_job_thread()?;
    let deadline = Instant::now() + timeout;
    let mut queue = lock();
    queue.served = true;
    loop {
        let now = Instant::now();
        if !queue.waiting.is_empty() || now >= deadline {
            return Ok(std::mem::take(&mut queue.waiting));
        }
        let (guard, _) = WAKE
            .wait_timeout(queue, deadline.saturating_duration_since(now))
            .unwrap_or_else(|err| err.into_inner());
        queue = guard;
    }
}

/// Answer command `id`. A caller that gave up is ignored.
pub fn reply(id: u64, result: Result<Value, String>) {
    let mut queue = lock();
    queue.done.push((id, result));
    // Answers nobody collects (the caller timed out) must not pile up.
    if queue.done.len() > 256 {
        drop(queue.done.remove(0));
    }
    WAKE.notify_all();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test waits for a fixture. docs/LINT_RULES.md"
    )]
    fn a_command_waits_for_the_frame_reply() {
        register(vec![CommandSpec {
            name: "echo".into(),
            description: String::new(),
            schema: json::object([]),
        }]);
        let frame = std::thread::spawn(|| loop {
            let pending = take().expect("frame thread");
            if let Some(cmd) = pending.into_iter().next() {
                reply(cmd.id, Ok(cmd.args));
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        });
        let out = call("echo", json::int(7), Duration::from_secs(5)).expect("reply");
        assert_eq!(out, json::int(7));
        frame.join().unwrap();
        assert!(call("nope", json::int(1), Duration::from_millis(10)).is_err());

        // A host with no frame loop blocks in `take_wait` instead of polling.
        assert!(take_wait(Duration::from_millis(1))
            .expect("frame thread")
            .is_empty());
        let host = std::thread::spawn(|| {
            let pending = take_wait(Duration::from_secs(5)).expect("frame thread");
            let cmd = pending.into_iter().next().expect("woken with a command");
            reply(cmd.id, Ok(cmd.args));
        });
        let out = call("echo", json::int(8), Duration::from_secs(5)).expect("reply");
        assert_eq!(out, json::int(8));
        host.join().unwrap();
    }
}
