use super::*;
use http_client::{ProxyConfig, ProxyMode};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex as StdMutex, Once};
use std::thread;

static INSTALL_CRYPTO_PROVIDER: Once = Once::new();

fn ensure_crypto_provider() {
    INSTALL_CRYPTO_PROVIDER.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

#[test]
fn cockpit_oauth_client_accepts_every_global_proxy_mode() {
    ensure_crypto_provider();
    let configs = [
        ProxyConfig {
            mode: ProxyMode::System,
            ..Default::default()
        },
        ProxyConfig {
            mode: ProxyMode::Custom,
            url: "http://proxy.example:8080".to_string(),
            username: "alice".to_string(),
            password: "secret".to_string(),
            no_proxy: "localhost,.internal".to_string(),
        },
        ProxyConfig {
            mode: ProxyMode::Off,
            ..Default::default()
        },
    ];

    for config in configs {
        build_oauth_client_with_proxy(config)
            .expect("cockpit OAuth client should accept the configured proxy mode");
    }
}

#[test]
fn endpoint_contract_treats_low_utilization_as_percent() {
    let usage = parse_response(
        r#"{
            "five_hour": { "utilization": 1.0 },
            "seven_day": { "utilization": 42.0 }
        }"#,
    )
    .expect("versioned endpoint response parses");

    assert_eq!(usage.five_hour.fraction, 0.01);
    assert_eq!(usage.seven_day.fraction, 0.42);
}

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

    let snapshot_cache = cache.clone();
    let (snapshot_tx, snapshot_rx) = mpsc::channel();
    let snapshot = thread::spawn(move || {
        let snapshot = futures::executor::block_on(snapshot_cache.snapshot());
        snapshot_tx.send(snapshot).unwrap();
    });
    let cache_was_responsive = snapshot_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .is_ok();

    {
        let (lock, wake) = &*gate;
        *lock.lock().unwrap() = true;
        wake.notify_all();
    }

    let first_cache = first.join().unwrap();
    let second_cache = second.join().unwrap();
    snapshot.join().unwrap();
    assert!(
        cache_was_responsive,
        "refresh must not hold the cache mutex"
    );
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(first_cache.len(), 1);
    assert_eq!(second_cache.len(), 1);
    assert!(first_cache[&account].usage.is_none());
    assert!(second_cache[&account].usage.is_none());
}
