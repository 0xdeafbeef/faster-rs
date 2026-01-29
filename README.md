[![License](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/faster-rs/faster-rs)
[![Cargo](https://img.shields.io/badge/crates.io-v0.11.0-orange)](https://crates.io/crates/faster-rs)
[![Build Status](https://dev.azure.com/faster-rs/faster-rs/_apis/build/status/faster-rs.faster-rs?branchName=master)](https://dev.azure.com/faster-rs/faster-rs/_build/latest?definitionId=1&branchName=master)

# Experimental FASTER wrapper for Rust

```toml
[dependencies]
faster-rs = "0.11.0"
```

Includes experimental C interface for FASTER. It is a generic implementation of FASTER that allows arbitrary Key-Value pairs to be stored. This wrapper is only focusing on Linux support.

Install Dependencies (Ubuntu):
```
$ add-apt-repository -y ppa:ubuntu-toolchain-r/test
$ apt update
$ apt install -y g++-7 libaio-dev uuid-dev libtbb-dev
```

*Make sure you clone the submodules as well*, this is best done by cloning with `git clone --recurse-submodules`.

## The interface
This wrapper exposes a simple raw-bytes API that mirrors the original FASTER design. Keys and values are passed as `AsRef<[u8]>`, so callers control serialization and schema evolution.


The `Read` and `Upsert` operations require a monotonic serial number to form the sequence of operations that will be persisted by FASTER. `Read` operations require a serial number so that at a CPR checkpoint boundary, FASTER guarantees that the reads before that point have accessed no data updates after the checkpoint. If persistence is not important, the serial number can safely be set to `1` for all operations (as is done in the examples above).

More information about Checkpointing and Recovery is provided below the following examples.

## A basic example

The following example shows the creation of a FASTER Key-Value Store and basic operations on `u64` values encoded as bytes.

Try it out by running `cargo run --example basic`.

```rust,no_run
extern crate faster_rs;

use faster_rs::{status, FasterKv};
use std::convert::TryInto;
fn enc_u64(value: u64) -> [u8; 8] {
    value.to_le_bytes()
}

fn dec_u64(bytes: &[u8]) -> u64 {
    let array: [u8; 8] = bytes.try_into().unwrap();
    u64::from_le_bytes(array)
}

fn main() {
    // Create a Key-Value Store
    let store = FasterKv::default();
    let key0: u64 = 1;
    let value0: u64 = 1000;

    // Upsert
    for i in 0..1000 {
        let key = enc_u64(key0 + i);
        let value = enc_u64(value0 + i);
        let upsert = store.upsert(&key, &value, i);
        assert!(upsert == status::OK || upsert == status::PENDING);
    }

    assert!(store.size() > 0);

    // Read
    for i in 0..1000 {
        let key = enc_u64(key0 + i);
        let (read, recv) = store.read(&key, i);
        assert!(read == status::OK || read == status::PENDING);
        let val = recv.recv().unwrap();
        let value = dec_u64(&val);
        assert_eq!(value, value0 + i);
        println!("Key: {}, Value: {}", key0 + i, value);
    }

    // Clear used storage
    match store.clean_storage() {
        Ok(()) => {}
        Err(_err) => panic!("Unable to clear FASTER directory"),
    }
}
```

## Using custom keys
Keys are arbitrary byte sequences. If you have a custom type, serialize it to bytes before calling `upsert` or `read`.

The following example shows custom bytes being used as a key. Try it out by running `cargo run --example custom_keys`.

```rust,no-run
extern crate faster_rs;
use faster_rs::{status, FasterKv};
use std::convert::TryInto;
fn main() {
    // Create a Key-Value Store
    let store = FasterKv::default();
    let key = b"hello-world".to_vec();
    let value = vec![1u8, 2, 3, 4];

    // Upsert
    let upsert = store.upsert(&key, &value, 1);
    assert!(upsert == status::OK || upsert == status::PENDING);

    assert!(store.size() > 0);

    let (read, recv) = store.read(&key, 1);
    assert!(read == status::OK || read == status::PENDING);
    let val = recv.recv().unwrap();
    println!("Key: {:?}, Value: {:?}", key, val);

    // Clear used storage
    match store.clean_storage() {
        Ok(()) => {}
        Err(_err) => panic!("Unable to clear FASTER directory"),
    }
}
```


## Using custom values
Values are arbitrary byte sequences. If you have a custom type, serialize it to bytes before calling `upsert`, and deserialize the returned bytes from `read`.

The following example shows custom bytes being used as a value. Try it out by running `cargo run --example custom_values`.

```rust,no_run
extern crate faster_rs;
use faster_rs::{status, FasterKv};
use std::convert::TryInto;
fn main() {
    // Create a Key-Value Store
    let store = FasterKv::default();
    let key = b"primary-key".to_vec();
    let value = b"hello-world".to_vec();

    // Upsert
    let upsert = store.upsert(&key, &value, 1);
    assert!(upsert == status::OK || upsert == status::PENDING);

    assert!(store.size() > 0);

    let (read, recv) = store.read(&key, 1);
    assert!(read == status::OK || read == status::PENDING);
    let val = recv.recv().unwrap();
    println!("Key: {:?}, Value: {:?}", key, val);

    // Clear used storage
    match store.clean_storage() {
        Ok(()) => {}
        Err(_err) => panic!("Unable to clear FASTER directory"),
    }
}
```

## Checkpoint and Recovery
FASTER's fault tolerance is provided by [Concurrent Prefix Recovery](https://www.microsoft.com/en-us/research/uploads/prod/2019/01/cpr-sigmod19.pdf) (CPR). It provides the following semantics:
 > If operation X is persisted, then all operations before X in the input operation sequence are persisted as well (and none after).

Persisting operations is done using the `checkpoint()` function. It is also important to periodically call the `refresh()` function as it is the mechanism threads use to report forward progress to the system.

Individual sessions (threads accessing FASTER) will persist a different number of operations. The most recently persisted serial number is returned by the `continue_session()` function and allows reasoning about which operations were (not) persisted. It is also the operation sequence number from which the thread should continue to provide operations after recovery. 

A good demonstration of checkpointing/recovery can be found in `examples/sum_store_single.rs`. Try it out for yourself!
```bash
$ cargo run --example sum_store_single -- populate
$ cargo run --example sum_store_single -- recover <checkpoint-token>
```

## Benchmarking
It is possible to benchmark both the C-wrapper and the Rust-wrapper of FASTER. To build and run the C-benchmark follow Microsoft's instructions [here](https://github.com/Microsoft/FASTER/tree/master/cc) and then run the binary `benchmark-c`. It takes the same parameters and input format as the original benchmark.

### Running the Rust benchmark
The benchmark is written as a separate crate in the `benchmark` directory. Inside the directory run `cargo run --release -- help` to see the available options.

The benchmark consists of two subcommands `cargo run --release -- [process-ycsb|run]`:
* `process-ycsb` will take the output of the supplied YCSB file and produce an output file containing only the 8-byte key in the format expected by the Rust & C benchmarks
* `run` will actually execute the benchmark using the supplied load and run keys. The workload and number of threads can be customised.

The benchmark is very similar to the original C++ implementation so it's best to follow their instructions for setting up YCSB.
