extern crate faster_rs;

use faster_rs::*;
use std::convert::TryInto;
use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const TABLE_SIZE: u64 = 1 << 15;
const LOG_SIZE: u64 = 17179869184;
const NUM_OPS: u64 = 1 << 25;
const NUM_UNIQUE_KEYS: u64 = 1 << 22;
const REFRESH_INTERVAL: u64 = 1 << 8;
const COMPLETE_PENDING_INTERVAL: u64 = 1 << 12;
const CHECKPOINT_INTERVAL: u64 = 1 << 22;

const STORAGE_DIR: &str = "sum_store_concurrent_storage";

// More or less a copy of the multi-threaded sum_store populate/recover example from FASTER

fn enc_u64(value: u64) -> [u8; 8] {
    value.to_le_bytes()
}

fn dec_u64(bytes: &[u8]) -> u64 {
    let array: [u8; 8] = bytes.try_into().unwrap();
    u64::from_le_bytes(array)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() > 2 {
        let operation = &args[1].to_string();
        let num_threads = args[2]
            .parse()
            .expect("Must specify number of threads as an integer");

        if operation == "populate" {
            println!("This may take a while, and make sure you have disk space");
            populate(num_threads);
        } else if operation == "recover" {
            if args.len() > 3 {
                let token = &args[3];
                recover(token.to_string());
            } else {
                println!("Second argument required is checkpoint token to recover");
            }
        }
    } else {
        println!("Populate: args 1. populate, 2. #threads");
        println!(
            "Recover: args 1. recover, 2. #threads, 3. checkpoint token"
        );
    }
}

fn populate(num_threads: usize) {
    if let Ok(store) = FasterKvConfig::builder()
        .table_size(TABLE_SIZE)
        .log_size(LOG_SIZE)
        .storage_path(STORAGE_DIR.to_owned())
        .build()
    {
        let store = Arc::new(store);
        let mut threads = vec![];
        let num_active_threads = Arc::new(AtomicUsize::new(0));
        for thread_id in 0..num_threads {
            let store = Arc::clone(&store);
            let num_active_threads = Arc::clone(&num_active_threads);
            threads.push(std::thread::spawn(move || {
                // Populate Store
                let _session = store.start_session();
                num_active_threads.fetch_add(1, Ordering::SeqCst);

                for i in 0..NUM_OPS {
                    let idx = i;
                    let key = enc_u64(idx % NUM_UNIQUE_KEYS);
                    let value = enc_u64(idx);
                    store.upsert(&key, &value, idx);

                    if idx.is_multiple_of(CHECKPOINT_INTERVAL)
                        && num_active_threads.load(Ordering::SeqCst) == num_threads
                    {
                        let check = store.checkpoint().unwrap();
                        println!("Calling checkpoint with token {}", check.token);
                    }

                    if idx.is_multiple_of(COMPLETE_PENDING_INTERVAL) {
                        store.complete_pending(false);
                    } else if idx.is_multiple_of(REFRESH_INTERVAL) {
                        store.refresh();
                    }
                }

                store.complete_pending(true);
                store.stop_session();
                println!("Thread {} finished populating", thread_id);
            }));
        }
        for t in threads {
            t.join().expect("Something went wrong in a thread");
        }
        println!("Threads finished populating");
        println!("Store size: {}", store.size());
        println!("Verifying values");

        store.start_session();
        let mut read_results = Vec::with_capacity(NUM_UNIQUE_KEYS as usize);
        read_results.resize_with(NUM_UNIQUE_KEYS as usize, || None);
        for idx in 0..NUM_UNIQUE_KEYS {
            let key = enc_u64(idx);
            let (_, receiver) = store.read(&key, idx);
            read_results[idx as usize] = Some(receiver);
        }
        store.complete_pending(true);
        store.stop_session();

        for idx in 0..NUM_UNIQUE_KEYS {
            let recv = read_results[idx as usize].take().unwrap();
            match recv.recv() {
                Ok(val) => {
                    let _ = dec_u64(&val);
                }
                Err(_) => {
                    println!("Error reading {}", idx);
                }
            }
        }
    } else {
        println!("Failed to create FasterKV store");
    }
}

fn recover(token: String) {
    println!("Attempting to recover");
    if let Ok(store) = FasterKvConfig::builder()
        .table_size(TABLE_SIZE)
        .log_size(LOG_SIZE)
        .storage_path(STORAGE_DIR.to_owned())
        .build()
    {
        match store.recover(token.clone(), token.clone()) {
            Ok(rec) => {
                println!("Recover version: {}", rec.version);
                println!("Recover status: {}", rec.status);
                println!("Recovered sessions: {:?}", rec.session_ids);
                
                for id in rec.session_ids {
                    store.continue_session(id);
                    store.stop_session();
                }

                store.start_session();
                let mut read_results = Vec::with_capacity(NUM_UNIQUE_KEYS as usize);
                read_results.resize_with(NUM_UNIQUE_KEYS as usize, || None);
                for idx in 0..NUM_UNIQUE_KEYS {
                    let key = enc_u64(idx);
                    let (_, receiver) = store.read(&key, idx);
                    read_results[idx as usize] = Some(receiver);
                }
                store.complete_pending(true);
                store.stop_session();

                println!("Verifying recovered values!");
                let mut incorrect = 0;
                for i in 0..NUM_OPS {
                    let idx = i;
                    let key = enc_u64(idx % NUM_UNIQUE_KEYS);
                    let (status, recv) = store.read(&key, idx);
                    if let Ok(val) = recv.recv() {
                        let _ = dec_u64(&val);
                    } else {
                        println!("Failure to read with status: {}, and key: {}", status, idx);
                        incorrect += 1;
                    }
                }
                println!("{} incorrect recoveries", incorrect);
            }
            Err(_) => println!("Recover operation failed"),
        }
    } else {
        println!("Failed to create recover store");
    }
}
