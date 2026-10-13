//! Quitting terminates running script groups synchronously. Lives in its own test binary because
//! `terminate_all_groups` acts on every subprocess of the process.

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use kebloop_services::automation::{
    AutomationRunner, ExecuteRequest, running_group_count, terminate_all_groups,
};

fn gone(pid: i32) -> bool {
    !std::path::Path::new(&format!("/proc/{pid}")).exists()
        || fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.contains(") Z"))
}

#[test]
fn terminating_all_groups_kills_running_scripts_and_their_children() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("grandchild.pid");
    let script = format!("sleep 120 & echo $! > '{}'; wait", pid_file.display());
    let mut request = ExecuteRequest::new("/bin/sh", vec!["-c".into(), script]);
    request.timeout = Duration::from_secs(60);
    let runner = AutomationRunner::new();
    let worker = {
        let runner = runner.clone();
        thread::spawn(move || runner.execute(&request))
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let grandchild = loop {
        if let Some(pid) = fs::read_to_string(&pid_file)
            .ok()
            .and_then(|text| text.trim().parse::<i32>().ok())
        {
            break pid;
        }
        assert!(Instant::now() < deadline, "script did not start");
        thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(running_group_count(), 1);

    let started = Instant::now();
    runner.cancel();
    terminate_all_groups(Duration::from_millis(500));
    assert!(started.elapsed() < Duration::from_secs(3));
    let deadline = Instant::now() + Duration::from_secs(3);
    while !gone(grandchild) {
        assert!(
            Instant::now() < deadline,
            "grandchild {grandchild} survived"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert!(worker.join().unwrap().is_err());
    assert_eq!(running_group_count(), 0);
}
