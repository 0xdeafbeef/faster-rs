use faster_rs::{FasterKvConfig, HlogCompactionConfig};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn temp_storage_dir() -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock drift")
        .as_nanos();
    let pid = std::process::id();
    dir.push(format!("faster-rs-compaction-start-session-race-{nanos}-{pid}"));
    fs::create_dir_all(&dir).expect("create storage dir");
    dir
}

fn main() {
    // FASTER C++ uses a fixed 2^25 log page size; keep all sizes aligned.
    const LOG_PAGE_SIZE_BYTES: u64 = 1 << 25;

    let dir = temp_storage_dir();
    let path = dir.to_str().expect("utf-8 path").to_owned();

    // Keep the size threshold tiny so we can reach it quickly during the non-REST window.
    let log_size = LOG_PAGE_SIZE_BYTES * 8;
    let hlog_size_budget = log_size * 2;
    let trigger_pct = 0.01;
    let size_threshold = (hlog_size_budget as f64 * trigger_pct) as u64;

    let store = FasterKvConfig::builder()
        .table_size(1 << 15)
        .log_size(log_size)
        .storage_path(path)
        .log_mutable_fraction(0.5)
        .hlog_compaction(
            HlogCompactionConfig::builder()
                .check_interval(Duration::from_millis(1))
                .trigger_pct(trigger_pct)
                .compact_pct(0.1)
                .max_compacted_size(LOG_PAGE_SIZE_BYTES)
                .hlog_size_budget(hlog_size_budget)
                .num_threads(2)
                .build(),
        )
        .build()
        .expect("open store");

    let store = Arc::new(store);
    eprintln!("store opened");

    let stall_started = Arc::new(AtomicBool::new(false));
    let writer_started = Arc::new(AtomicBool::new(false));
    let checkpoint_started = Arc::new(AtomicBool::new(false));
    let checkpoint_started_for_main = Arc::clone(&checkpoint_started);

    // Thread that holds an active session but does not refresh for a while. This keeps a
    // checkpoint in a non-REST phase long enough for auto-compaction to attempt to start a
    // session (the historical bug).
    let store_for_stall = Arc::clone(&store);
    let stall_started_for_thread = Arc::clone(&stall_started);
    let checkpoint_started_for_stall = Arc::clone(&checkpoint_started);
    let stall = thread::spawn(move || {
        eprintln!("stall thread: starting session");
        store_for_stall.start_session();
        eprintln!("stall thread: session started");
        stall_started_for_thread.store(true, Ordering::Release);

        while !checkpoint_started_for_stall.load(Ordering::Acquire) {
            thread::yield_now();
        }
        eprintln!("stall thread: checkpoint started; stalling refresh");

        thread::sleep(Duration::from_millis(300));
        eprintln!("stall thread: resuming refresh");

        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(300) {
            store_for_stall.refresh();
            store_for_stall.complete_pending(false);
            thread::yield_now();
        }
        store_for_stall.stop_session();
        eprintln!("stall thread: session stopped");
    });

    // Writer thread: waits for checkpoint to start, then grows the log past the compaction
    // threshold.
    let store_for_writer = Arc::clone(&store);
    let writer_started_for_thread = Arc::clone(&writer_started);
    let checkpoint_started_for_writer = Arc::clone(&checkpoint_started);
    let writer = thread::spawn(move || {
        eprintln!("writer thread: starting session");
        store_for_writer.start_session();
        eprintln!("writer thread: session started");
        writer_started_for_thread.store(true, Ordering::Release);

        while !checkpoint_started_for_writer.load(Ordering::Acquire) {
            thread::yield_now();
        }
        eprintln!("writer thread: checkpoint started; writing");

        // Give the checkpoint a moment to transition the store to a non-REST phase before we grow
        // the log enough to trigger auto-compaction.
        thread::sleep(Duration::from_millis(20));

        let value = vec![0u8; 4 * 1024];
        let mut serial = 0u64;
        let mut last_refresh = Instant::now();

        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(200) {
            serial += 1;
            let mut key = [0u8; 32];
            key[..8].copy_from_slice(&serial.to_le_bytes());
            let _ = store_for_writer.upsert(&key, &value, serial);
            store_for_writer.complete_pending(false);
            store_for_writer.refresh_if_due(&mut last_refresh, Duration::from_millis(5));
        }

        // Help special phases finish.
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(200) {
            store_for_writer.complete_pending(false);
            store_for_writer.refresh();
            thread::yield_now();
        }

        store_for_writer.stop_session();
        eprintln!("writer thread: session stopped");
    });

    while !stall_started.load(Ordering::Acquire) || !writer_started.load(Ordering::Acquire) {
        thread::yield_now();
    }
    eprintln!("worker sessions started");

    // Start a checkpoint in a separate thread. The checkpoint call blocks until completion, so
    // we must allow other sessions to make progress (writer thread) while holding one session
    // without refresh (stall thread) to keep the store in a non-REST phase.
    let store_for_checkpoint = Arc::clone(&store);
    let checkpoint = thread::spawn(move || {
        checkpoint_started_for_main.store(true, Ordering::Release);
        eprintln!("checkpoint thread: starting checkpoint_index");
        let _ = store_for_checkpoint.checkpoint_index();
        eprintln!("checkpoint thread: checkpoint_index finished");
    });

    // Ensure the log grows past the compaction threshold.
    let start = Instant::now();
    while store.size() < size_threshold && start.elapsed() < Duration::from_secs(2) {
        thread::yield_now();
    }
    if store.size() < size_threshold {
        eprintln!(
            "failed to reach size threshold; size={} threshold={size_threshold}",
            store.size()
        );
        std::process::exit(1);
    }

    // Wait for auto-compaction to be scheduled. On buggy versions, the compaction thread then
    // calls StartSession() during the non-REST phase and aborts the process.
    let start = Instant::now();
    while !store.auto_compaction_scheduled() && start.elapsed() < Duration::from_secs(2) {
        thread::yield_now();
    }
    if !store.auto_compaction_scheduled() {
        eprintln!("auto compaction was not scheduled");
        std::process::exit(2);
    }

    // Give the historical crash a chance to manifest deterministically.
    thread::sleep(Duration::from_millis(200));

    let _ = stall.join();
    let _ = writer.join();
    let _ = checkpoint.join();

    // Avoid hanging on destructor if a special phase is still in progress.
    std::process::exit(0);
}
