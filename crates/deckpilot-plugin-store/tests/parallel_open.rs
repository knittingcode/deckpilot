//! The first connections of a process are opened on many threads at once,
//! as test threads (and the agent's start-up) do. None of them may miss
//! SQLCipher's one-time initialization (`sqlcipherCodecAttach: sqlcipher
//! not initialized`, then `PRAGMA key` fails). Its own test binary, so that
//! these really are the process's first connections.

#![cfg(feature = "sqlcipher")]

use std::sync::{Arc, Barrier};

use deckpilot_plugin_store::{ManagerOptions, PluginManager, StoreKey};

#[test]
fn the_first_opens_of_a_process_may_run_in_parallel() {
    const THREADS: usize = 8;
    let start = Arc::new(Barrier::new(THREADS));
    let threads: Vec<_> = (0..THREADS)
        .map(|_| {
            let start = start.clone();
            std::thread::spawn(move || {
                let dir = tempfile::tempdir().unwrap();
                let key = StoreKey::from_bytes([0x5a; 32]);
                start.wait();
                PluginManager::open(dir.path(), &key, ManagerOptions::default()).map(drop)
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap().unwrap();
    }
}
