extern crate faster_rs;

use faster_rs::FasterKv;
use std::convert::TryInto;
use std::sync::Arc;
use std::thread;

fn enc_u64(value: u64) -> [u8; 8] {
    value.to_le_bytes()
}

fn dec_u64(bytes: &[u8]) -> u64 {
    let array: [u8; 8] = bytes.try_into().unwrap();
    u64::from_le_bytes(array)
}

#[test]
fn multi_threaded_test() {
    let store = Arc::new(FasterKv::default());
    let ops = 1 << 15;

    let initial_value = enc_u64(100);
    store.start_session();

    for key in 0..ops {
        store.upsert(&enc_u64(key), &initial_value, key);
    }

    let num_threads = 16;
    let mut threads = vec![];
    for _ in 0..num_threads {
        let store = Arc::clone(&store);
        threads.push(thread::spawn(move || {
            // Register FASTER thread
            let _session = store.start_session();

            for key in 0..ops {
                let (_res, recv) = store.read(&enc_u64(key), key);
                let value = recv.recv().unwrap();
                assert_eq!(dec_u64(&value), 100);
            }

            // Make sure everything is completed
            store.complete_pending(true);

            // Unregister Thread
            store.stop_session();
        }))
    }

    for t in threads {
        t.join().unwrap();
    }

    for key in 0..ops {
        let (_res, recv) = store.read(&enc_u64(key), ops + key);
        let value = recv.recv().unwrap();
        assert_eq!(dec_u64(&value), 100);
    }
    store.complete_pending(true);
    store.stop_session();
}
