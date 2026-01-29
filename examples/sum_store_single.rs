extern crate faster_rs;

use faster_rs::*;
use std::convert::TryInto;
use std::env;

const TABLE_SIZE: u64 = 1 << 15;
const LOG_SIZE: u64 = 1024 * 1024 * 1024;
const NUM_OPS: u64 = 1 << 25;
const NUM_UNIQUE_KEYS: u64 = 1 << 23;
const REFRESH_INTERVAL: u64 = 1 << 8;
const COMPLETE_PENDING_INTERVAL: u64 = 1 << 12;
const CHECKPOINT_INTERVAL: u64 = 1 << 20;

const STORAGE_DIR: &str = "sum_store_single_storage";

// More or less a copy of the single-threaded sum_store populate/recover example from FASTER

fn enc_u64(value: u64) -> [u8; 8] {
    value.to_le_bytes()
}

fn dec_u64(bytes: &[u8]) -> u64 {
    let array: [u8; 8] = bytes.try_into().unwrap();
    u64::from_le_bytes(array)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 {
        let operation = &args[1].to_string();

        if operation == "populate" {
            println!("This may take a while, and make sure you have disk space");
            populate();
        } else if operation == "recover" {
            if args.len() > 2 {
                let token = &args[2];
                recover(token.to_string());
            } else {
                println!("Second argument required is token checkpoint to recover");
            }
        }
    } else {
        println!("Populate: args 1. populate");
        println!("Recover: args 1. recover, 2. checkpoint token");
    }
}

fn populate() {
    if let Ok(store) = FasterKvConfig::builder()
        .table_size(TABLE_SIZE)
        .log_size(LOG_SIZE)
        .storage_path(STORAGE_DIR.to_owned())
        .pre_allocate_log(true)
        .build()
    {
        // Populate Store
        let session = store.start_session();
        println!("Starting Session {}", session);

        for i in 0..NUM_OPS {
            let idx = i;
            let key = enc_u64(idx % NUM_UNIQUE_KEYS);
            let value = enc_u64(idx);
            store.upsert(&key, &value, idx);

            if idx.is_multiple_of(CHECKPOINT_INTERVAL) {
                let check = store.checkpoint().unwrap();
                println!("Calling checkpoint with token {}", check.token);
            }

            if idx.is_multiple_of(COMPLETE_PENDING_INTERVAL) {
                store.complete_pending(false);
            } else if idx.is_multiple_of(REFRESH_INTERVAL) {
                store.refresh();
            }
        }

        println!("Dumping distribution");
        store.dump_distribution();
        println!("Stopping Session {}", session);
        store.complete_pending(true);
        store.stop_session();
        println!("Store size: {}", store.size());
    } else {
        println!("Failed to create FasterKV store");
    }
}

fn recover(token: String) {
    println!("Attempting to recover");
    if let Ok(recover_store) = FasterKvConfig::builder()
        .table_size(TABLE_SIZE)
        .log_size(LOG_SIZE)
        .storage_path(STORAGE_DIR.to_owned())
        .pre_allocate_log(true)
        .build()
    {
        match recover_store.recover(token.clone(), token.clone()) {
            Ok(rec) => {
                println!("Recover version: {}", rec.version);
                println!("Recover status: {}", rec.status);
                println!("Recovered sessions: {:?}", rec.session_ids);
                let persisted_count =
                    recover_store.continue_session(rec.session_ids.first().cloned().unwrap());
                println!("Session persisted until: {}", persisted_count);

                let mut expected_results = vec![0; NUM_UNIQUE_KEYS as usize];
                for i in 0..(persisted_count + 1) {
                    let elem = expected_results
                        .get_mut((i % NUM_UNIQUE_KEYS) as usize)
                        .unwrap();
                    *elem = i;
                }

                println!("Verifying recovered values!");
                let mut incorrect = 0;
                for i in 0..NUM_OPS {
                    let idx = i;
                    let key = enc_u64(idx % NUM_UNIQUE_KEYS);
                    let (status, recv) = recover_store.read(&key, idx);
                    if let Ok(val) = recv.recv() {
                        let expected = *expected_results
                            .get((idx % NUM_UNIQUE_KEYS) as usize)
                            .unwrap();
                        if expected != dec_u64(&val) {
                            println!(
                                "Error recovering {}, expected {}, got {}",
                                idx,
                                expected,
                                dec_u64(&val)
                            );
                            incorrect += 1;
                        }
                    } else {
                        println!("Failure to read with status: {}, and key: {}", status, idx);
                    }
                }
                println!("{} incorrect recoveries", incorrect);
                recover_store.stop_session();
            }
            Err(_) => println!("Recover operation failed"),
        }
    } else {
        println!("Failed to create recover store");
    }
}
