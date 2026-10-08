//! Windows cache serialization; no network or game process is involved.
use rtx_fg_manager::win;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

#[test]
fn download_waits_for_cache_owner_and_resumes_after_release() {
    let dir = tempfile::tempdir().unwrap();
    let owner = win::game_lock(dir.path()).unwrap();
    let path = dir.path().to_owned();
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let _guard = win::cache_lock_wait(&path, &AtomicBool::new(false), || {
            tx.send("waiting").unwrap();
        })
        .unwrap();
        tx.send("acquired").unwrap();
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), "waiting");
    assert!(rx.recv_timeout(Duration::from_millis(250)).is_err());
    drop(owner);
    assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), "acquired");
    worker.join().unwrap();
    assert!(win::game_lock(dir.path()).is_ok());
}

#[test]
fn cancelled_wait_exits_without_taking_or_releasing_another_owners_lock() {
    let dir = tempfile::tempdir().unwrap();
    let owner = win::game_lock(dir.path()).unwrap();
    let path = dir.path().to_owned();
    let cancellation = Arc::new(AtomicBool::new(false));
    let flag = cancellation.clone();
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let error = match win::cache_lock_wait(&path, &flag, || {
            tx.send("waiting").unwrap();
        }) {
            Ok(_) => panic!("cancelled worker acquired the held lock"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("取消"));
        // Ordinary game mutations still fail immediately on contention.
        assert!(win::game_lock(&path).is_err());
        tx.send("cancelled").unwrap();
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap(), "waiting");
    cancellation.store(true, Ordering::Relaxed);
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(3)).unwrap(),
        "cancelled"
    );
    worker.join().unwrap();
    drop(owner);
}

#[test]
fn already_cancelled_cache_task_never_acquires_lock() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        win::cache_lock_wait(dir.path(), &AtomicBool::new(true), || panic!(
            "unexpected wait"
        ))
        .is_err()
    );
    assert!(win::game_lock(dir.path()).is_ok());
}
