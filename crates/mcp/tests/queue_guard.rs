//! A job thread must not drain the command queue.

#![expect(clippy::expect_used, reason = "a test aborts when a fixture fails")]
#![expect(clippy::unwrap_used, reason = "a test aborts when a fixture fails")]

#[test]
fn a_worker_cannot_take_the_command_queue() {
    let before = genos_mcp::command::queue_violations();
    let handle = std::thread::Builder::new()
        .name("genos-job-0".into())
        .spawn(|| genos_mcp::command::take().expect_err("job thread"))
        .unwrap();
    let err = handle.join().unwrap();
    assert_eq!(err, "jobs.queue_violation");
    assert_eq!(genos_mcp::command::queue_violations(), before + 1);
    assert!(genos_mcp::command::take().is_ok());
    let waited = std::thread::Builder::new()
        .name("genos-job-1".into())
        .spawn(|| {
            genos_mcp::command::take_wait(std::time::Duration::from_millis(1)).expect_err("wait")
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(waited, "jobs.queue_violation");
}
