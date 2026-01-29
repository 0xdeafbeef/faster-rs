extern crate faster_rs;

use faster_rs::{status, FasterKv};
fn enc_u64(value: u64) -> [u8; 8] {
    value.to_le_bytes()
}

#[test]
fn faster_check() {
    let store = FasterKv::default();
    let key = enc_u64(1);
    let value = enc_u64(1337);

    let upsert = store.upsert(&key, &value, 1);
    assert!(upsert == status::OK || upsert == status::PENDING);

    assert!(store.size() > 0);
}

#[test]
fn faster_read_inserted_value() {
    let store = FasterKv::default();
    let key = enc_u64(1);
    let value = enc_u64(1337);

    let upsert = store.upsert(&key, &value, 1);
    assert!(upsert == status::OK || upsert == status::PENDING);

    let (res, recv) = store.read(&key, 1);
    assert!(res == status::OK);
    assert!(recv.recv().unwrap() == value);
}

#[test]
fn faster_read_missing_value_recv_error() {
    let store = FasterKv::default();
    let key = enc_u64(1);

    let (res, recv) = store.read(&key, 1);
    assert!(res == status::NOT_FOUND);
    assert!(recv.recv().is_err());
}

#[test]
fn faster_read_drop_waiter_without_recv() {
    let store = FasterKv::default();
    let key = enc_u64(1);

    let (_res, waiter) = store.read(&key, 1);
    drop(waiter);
}

#[test]
fn faster_delete_inserted_value() {
    let store = FasterKv::default();
    let key = enc_u64(1);
    let value = enc_u64(1337);

    let upsert = store.upsert(&key, &value, 1);
    assert!(upsert == status::OK || upsert == status::PENDING);

    let (res, recv) = store.read(&key, 1);
    assert!(res == status::OK);
    assert!(recv.recv().unwrap() == value);

    let delete = store.delete(&key, 1);
    assert!(delete == status::OK || delete == status::PENDING);

    let (res, recv) = store.read(&key, 1);
    assert!(res == status::NOT_FOUND);
    assert!(recv.recv().is_err());
}
