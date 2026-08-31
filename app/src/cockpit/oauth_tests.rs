use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex as StdMutex};
use std::thread;

#[test]
fn concurrent_refreshes_send_one_request_per_account_within_ttl() {
    let account = PathBuf::from("/test/claude-account");
    let cache = OauthCache::default();
    let requests = Arc::new(AtomicUsize::new(0));
    let gate = Arc::new((StdMutex::new(false), Condvar::new()));
    let (started_tx, started_rx) = mpsc::channel();

    let fetch = {
        let requests = requests.clone();
        let gate = gate.clone();
        move |_dir: PathBuf| {
            let requests = requests.clone();
            let gate = gate.clone();
            let started_tx = started_tx.clone();
            async move {
                requests.fetch_add(1, Ordering::SeqCst);
                started_tx.send(()).unwrap();
                let (lock, wake) = &*gate;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = wake.wait(released).unwrap();
                }
                None
            }
        }
    };

    let first = {
        let account = account.clone();
        let cache = cache.clone();
        let fetch = fetch.clone();
        thread::spawn(move || {
            futures::executor::block_on(refresh_cache_with(vec![account], cache, fetch))
        })
    };
    started_rx.recv().unwrap();
    let (second_invoked_tx, second_invoked_rx) = mpsc::channel();
    let second = {
        let account = account.clone();
        let cache = cache.clone();
        let fetch = fetch.clone();
        thread::spawn(move || {
            second_invoked_tx.send(()).unwrap();
            futures::executor::block_on(refresh_cache_with(vec![account], cache, fetch))
        })
    };
    second_invoked_rx.recv().unwrap();

    {
        let (lock, wake) = &*gate;
        *lock.lock().unwrap() = true;
        wake.notify_all();
    }

    let first_cache = first.join().unwrap();
    let second_cache = second.join().unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(first_cache.len(), 1);
    assert_eq!(second_cache.len(), 1);
    assert!(first_cache[&account].usage.is_none());
    assert!(second_cache[&account].usage.is_none());
}
