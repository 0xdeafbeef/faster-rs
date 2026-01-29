extern crate faster_rs;

use faster_rs::{status, FasterKv, FasterKvBuilder};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::convert::TryInto;
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;

const TABLE_SIZE: u64 = 1 << 26;
const LOG_PAGE: u64 = 32 * 1024 * 1024;
// FASTER requires at least 2 non-head pages and 4 immutable pages.
const LOG_SIZE: u64 = LOG_PAGE * 6;
const LOG_MUTABLE_FRACTION: f64 = 0.34;
const INSERTS: u64 = 300_000;
const DELETES: u64 = 200_000;
const READS_PER_THREAD: usize = 200_000;
const THREADS: usize = 4;
const SEED: u64 = 0x5eed_5eed_5eed_5eed;
const VALUE_SIZE: usize = 4096;

fn enc_u64(value: u64) -> [u8; 8] {
    value.to_le_bytes()
}

fn dec_u64(bytes: &[u8]) -> u64 {
    let array: [u8; 8] = bytes.try_into().unwrap();
    u64::from_le_bytes(array)
}

fn make_value(key: u64) -> Vec<u8> {
    let mut value = vec![0u8; VALUE_SIZE];
    value[..8].copy_from_slice(&enc_u64(key));
    value
}

fn read_key(store: &FasterKv, key: u64, serial: &mut u64) -> Vec<u8> {
    let mut tries = 0;
    loop {
        let (status, recv) = store.read(&enc_u64(key), *serial);
        *serial += 1;
        match status {
            status::OK => {
                return recv.recv().unwrap_or_else(|error| {
                    panic!(
                        "fckup faster read ok returned error={error:?} key={key}",
                        error = error,
                        key = key
                    )
                });
            }
            status::PENDING => {
                store.complete_pending(true);
                return recv.recv().unwrap_or_else(|error| {
                    panic!(
                        "fckup faster read pending completion returned error={error:?} key={key}",
                        error = error,
                        key = key
                    )
                });
            }
            status::NOT_FOUND => {
                panic!("fckup faster read NOT_FOUND key={key}", key = key);
            }
            status::OUT_OF_MEMORY => {
                tries += 1;
                if tries > 8 {
                    panic!(
                        "fckup faster read OUT_OF_MEMORY retry limit key={key}",
                        key = key
                    );
                }
                if !store.grow_index() {
                    thread::yield_now();
                }
                continue;
            }
            other => {
                panic!(
                    "fckup faster read status={other} key={key}",
                    other = other,
                    key = key
                );
            }
        }
    }
}

#[test]
#[ignore]
fn repro_disk_read_after_flush_random_reads() {
    let tmp_dir = TempDir::new().unwrap();
    let dir = tmp_dir.path().to_str().unwrap();

    let mut builder = FasterKvBuilder::new(TABLE_SIZE, LOG_SIZE);
    builder
        .with_disk(dir)
        .with_log_mutable_fraction(LOG_MUTABLE_FRACTION)
        .set_pre_allocate_log(true);
    let store = Arc::new(builder.build().unwrap());

    let mut serial = 0u64;
    store.start_session();

    for key in 0..INSERTS {
        let value = make_value(key);
        loop {
            let status = store.upsert(&enc_u64(key), &value, serial);
            serial += 1;
            match status {
                status::OK => break,
                status::PENDING => store.complete_pending(true),
                status::OUT_OF_MEMORY => {
                    if !store.grow_index() {
                        thread::yield_now();
                    }
                }
                other => panic!(
                    "fckup faster read upsert status={other} key={key}",
                    other = other,
                    key = key
                ),
            }
        }
    }

    for key in 0..DELETES {
        loop {
            let status = store.delete(&enc_u64(key), serial);
            serial += 1;
            match status {
                status::OK => break,
                status::PENDING => store.complete_pending(true),
                status::OUT_OF_MEMORY => {
                    if !store.grow_index() {
                        thread::yield_now();
                    }
                }
                other => panic!(
                    "fckup faster read delete status={other} key={key}",
                    other = other,
                    key = key
                ),
            }
        }
    }

    store.complete_pending(true);
    let live_keys: Vec<u64> = (DELETES..INSERTS).collect();

    let mut threads = Vec::with_capacity(THREADS);
    for thread_index in 0..THREADS {
        let store = Arc::clone(&store);
        let live_keys = live_keys.clone();
        threads.push(thread::spawn(move || {
            store.start_session();
            let mut serial = 0u64;
            let mut rng = StdRng::seed_from_u64(SEED ^ thread_index as u64);
            for _ in 0..READS_PER_THREAD {
                let index = rng.random_range(0..live_keys.len());
                let key = live_keys[index];
                let value = read_key(&store, key, &mut serial);
                assert!(
                    value.len() >= 8,
                    "fckup faster read wrong value size key={key}",
                    key = key
                );
                assert_eq!(
                    dec_u64(&value[..8]),
                    key,
                    "fckup faster read wrong value key={key}",
                    key = key
                );
            }

            store.complete_pending(true);
            store.stop_session();
        }));
    }

    for thread in threads {
        thread.join().unwrap();
    }

    store.complete_pending(true);
    store.stop_session();
}
