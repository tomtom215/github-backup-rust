// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Process-wide shutdown of git subprocesses.
//!
//! `request_shutdown` is a one-way, process-wide flag, so this test lives in
//! its own integration-test binary (its own process) where it cannot affect
//! any other test.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use github_backup_core::git::{request_shutdown, shutdown_requested, CloneOptions, GitRunner};
use github_backup_core::{CoreError, ProcessGitRunner};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_request_stops_a_running_git_and_refuses_new_ones() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("pid");
    let script = dir.path().join("fake-git");
    // Records its own pid, then sleeps far longer than the test will wait.
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho $$ > '{}'\nexec sleep 60\n",
            pid_file.display()
        ),
    )
    .expect("write script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let runner = ProcessGitRunner::with_program(&script);
    let dest = dir.path().join("repo.git");
    let opts = CloneOptions {
        stall_timeout_secs: 120, // far longer than the test: only shutdown can end it
        ..CloneOptions::unauthenticated()
    };

    let running = {
        let runner = runner.clone();
        let dest = dest.clone();
        let opts = opts.clone();
        tokio::spawn(async move {
            runner
                .mirror_clone("https://example.invalid/r.git", &dest, &opts)
                .await
        })
    };

    // Wait until the child is really running.
    let started = Instant::now();
    while !pid_file.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "child never started"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .expect("pid file")
        .trim()
        .parse()
        .expect("pid");

    assert!(!shutdown_requested());
    let requested_at = Instant::now();
    request_shutdown();
    assert!(shutdown_requested());

    let result = running.await.expect("task must not panic");
    assert!(
        matches!(result, Err(CoreError::Interrupted)),
        "expected Interrupted, got {result:?}"
    );
    assert!(
        requested_at.elapsed() < Duration::from_secs(5),
        "shutdown took {:?}; it must stop git within a few poll intervals",
        requested_at.elapsed()
    );

    // The child process (and its group) must really be gone.
    let gone = std::path::Path::new(&format!("/proc/{pid}")).exists();
    if cfg!(target_os = "linux") {
        // Allow the kernel a moment to reap.
        let mut waited = 0;
        while std::path::Path::new(&format!("/proc/{pid}")).exists() && waited < 50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            waited += 1;
        }
        assert!(
            !std::path::Path::new(&format!("/proc/{pid}")).exists(),
            "git child {pid} survived the shutdown request (was alive at check: {gone})"
        );
    }

    // New git work is refused immediately once shutdown has been requested.
    let refused = runner
        .mirror_clone("https://example.invalid/r.git", &dest, &opts)
        .await;
    assert!(
        matches!(refused, Err(CoreError::Interrupted)),
        "{refused:?}"
    );
}
