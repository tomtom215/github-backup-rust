// SPDX-License-Identifier: MIT
// Copyright 2026 Tom F

//! Cancelling a run stops its git subprocesses — and only its own.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use github_backup_core::git::{CloneOptions, GitRunner};
use github_backup_core::{CancelFlag, CoreError, ProcessGitRunner};

fn write_script(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write script");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_stops_a_running_git_refuses_new_ones_and_spares_other_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("pid");
    let script = dir.path().join("fake-git");
    // Records its own pid, then sleeps far longer than the test will wait.
    write_script(
        &script,
        &format!(
            "#!/bin/sh\necho $$ > '{}'\nexec sleep 60\n",
            pid_file.display()
        ),
    );

    let runner = ProcessGitRunner::with_program(&script);
    let dest = dir.path().join("repo.git");
    let cancel = CancelFlag::new();
    let opts = CloneOptions {
        stall_timeout_secs: 120, // far longer than the test: only cancelling can end it
        cancel: cancel.clone(),
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

    let cancelled_at = Instant::now();
    cancel.cancel();

    let result = running.await.expect("task must not panic");
    assert!(
        matches!(result, Err(CoreError::Interrupted)),
        "expected Interrupted, got {result:?}"
    );
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(5),
        "cancelling took {:?}; it must stop git within a few poll intervals",
        cancelled_at.elapsed()
    );

    // The child process (and its group) must really be gone.
    let proc = format!("/proc/{pid}");
    if cfg!(target_os = "linux") {
        let mut waited = 0;
        while Path::new(&proc).exists() && waited < 50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            waited += 1;
        }
        assert!(
            !Path::new(&proc).exists(),
            "git child {pid} survived cancellation"
        );
    }

    // New git work with the cancelled flag is refused immediately.
    let refused = runner
        .mirror_clone("https://example.invalid/r.git", &dest, &opts)
        .await;
    assert!(
        matches!(refused, Err(CoreError::Interrupted)),
        "{refused:?}"
    );

    // A different run (its own flag) is unaffected: this is what lets the TUI
    // start a second backup after cancelling the first.
    let ok_script = dir.path().join("ok-git");
    write_script(&ok_script, "#!/bin/sh\nexit 0\n");
    let other = ProcessGitRunner::with_program(&ok_script);
    let other_dest = dir.path().join("other.git");
    std::fs::create_dir_all(&other_dest).expect("existing dest means an update, not a clone");
    other
        .mirror_clone(
            "https://example.invalid/o.git",
            &other_dest,
            &CloneOptions::unauthenticated(),
        )
        .await
        .expect("an independent run must not see the other run's cancellation");
}
