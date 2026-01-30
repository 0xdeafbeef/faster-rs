use faster_rs::FasterKvConfig;

#[test]
fn metrics_are_accessible_and_sane() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap().to_owned();

    const LOG_PAGE_SIZE_BYTES: u64 = 1 << 25;

    let store = FasterKvConfig::builder()
        .table_size(1 << 15)
        .log_size(LOG_PAGE_SIZE_BYTES * 8)
        .storage_path(path)
        .log_mutable_fraction(0.5)
        .build()
        .unwrap();

    store.start_session();
    let key = 1u64.to_le_bytes();
    let value = 2u64.to_le_bytes();
    let _ = store.upsert(&key, &value, 1);
    store.complete_pending(true);
    store.refresh();

    assert!(store.num_active_sessions() > 0);

    let begin = store.hlog_begin_address();
    let tail = store.hlog_tail_address();
    assert!(begin <= tail);

    let head = store.hlog_head_address();
    assert!(head >= begin);
    assert!(head <= tail);

    let safe_head = store.hlog_safe_head_address();
    assert!(safe_head >= begin);
    assert!(safe_head <= tail);

    let read_only = store.hlog_read_only_address();
    assert!(read_only >= begin);
    assert!(read_only <= tail);

    let safe_read_only = store.hlog_safe_read_only_address();
    assert!(safe_read_only >= begin);
    assert!(safe_read_only <= tail);

    let flushed_until = store.hlog_flushed_until_address();
    assert!(flushed_until >= begin);
    assert!(flushed_until <= tail);

    assert!(!store.hlog_max_size_reached());

    store.stop_session();
}
