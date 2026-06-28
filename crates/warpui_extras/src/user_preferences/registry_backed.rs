use std::io;
use std::sync::Mutex;

/// Store user preferences in the Windows Registry.
/// Modeled after https://github.com/neovide/neovide/blob/main/src/windows_utils.rs .
use super::UserPreferences;
use windows_registry::{Key, CURRENT_USER};
use windows_result::HRESULT;

pub struct RegistryBackedPreferences {
    app_key_path: String,
    /// Caches `HKCU\Software\Zap\<channel>` registry Key handle.
    ///
    /// Zap startup sequentially calls `read_value` on ~100 settings.
    /// Each `CURRENT_USER.create(...)` call is a ~3ms synchronous system call,
    /// totaling 300ms+ (dominating the cold-startup `READ_USER_DEFAULTS_AND_INITIALIZE_SETTINGS` phase).
    /// The first successfully opened Key is cached here; subsequent reads reuse it,
    /// eliminating N-1 system calls.
    ///
    /// Uses `Mutex<Option<Key>>` instead of `OnceLock` because `windows_registry::Key`
    /// does not implement `Clone`, requiring a mutable lock for `replace`/`take`.
    /// Also, `read_value` interface is `&self`, so `RefCell` cannot be used (requires `Sync`).
    cached_key: Mutex<Option<Key>>,
}

static ZAPLEX_REGISTRY_BASE_PATH: &str = "Software\\Zap\\";
pub const KEY_NOT_FOUND_ERR: HRESULT = HRESULT::from_win32(0x80070002);

impl RegistryBackedPreferences {
    /// Construct a separate registry path for each channel (stable, dev, local, etc.)
    pub fn new(app_name: &str) -> Self {
        let app_key_path = ZAPLEX_REGISTRY_BASE_PATH.to_owned() + app_name;
        // Warm up the Key at startup so the first setting read also avoids synchronous system calls.
        // Warmup failure is not an error: `with_warp_registry` will retry when needed.
        let initial_key = CURRENT_USER
            .create(app_key_path.clone())
            .inspect_err(|e| {
                log::warn!("warp registry key prewarm failed (will retry on first access): {e:#}");
            })
            .ok();
        Self {
            app_key_path,
            cached_key: Mutex::new(initial_key),
        }
    }

    /// Operates on the cached Zap registry Key via callback. First call invokes
    /// `CURRENT_USER.create(...)`; subsequent calls reuse the cached Key.
    /// If the Key lock is poisoned (previous panic), falls back to a one-time create
    /// without caching — behavior degrades but does not panic further.
    fn with_warp_registry<R>(
        &self,
        f: impl FnOnce(&Key) -> Result<R, super::Error>,
    ) -> Result<R, super::Error> {
        let mut guard = match self.cached_key.lock() {
            Ok(g) => g,
            // Mutex poisoned: take one-time create path without caching,
            // behavior equivalent to original.
            Err(_) => {
                let key = CURRENT_USER
                    .create(self.app_key_path.clone())
                    .map_err(|e| {
                        log::error!("unable to access Zap app key in Windows Registry: {e:#}");
                        super::Error::IoError(io::Error::from(e))
                    })?;
                return f(&key);
            }
        };

        if guard.is_none() {
            let key = CURRENT_USER
                .create(self.app_key_path.clone())
                .map_err(|e| {
                    log::error!("unable to access Zap app key in Windows Registry: {e:#}");
                    super::Error::IoError(io::Error::from(e))
                })?;
            *guard = Some(key);
        }

        // guard must be Some at this point; unwrap is safe.
        f(guard.as_ref().expect("cached_key must be Some after init"))
    }
}

impl UserPreferences for RegistryBackedPreferences {
    fn read_value(&self, name: &str) -> Result<Option<String>, super::Error> {
        self.with_warp_registry(|key| Ok(key.get_string(name).ok()))
    }

    fn write_value(&self, key: &str, value: String) -> Result<(), super::Error> {
        self.with_warp_registry(|reg| {
            reg.set_string(key, value.as_str())
                .map_err(|e| super::Error::from(io::Error::from(e)))
        })
    }

    fn remove_value(&self, key: &str) -> Result<(), super::Error> {
        self.with_warp_registry(|reg| match reg.remove_value(key) {
            Ok(_) => Ok(()),
            // If the key doesn't exist, then treat removal of that nonexistent key as a success.
            Err(e) if e.code() == KEY_NOT_FOUND_ERR => Ok(()),
            Err(e) => Err(super::Error::from(io::Error::from(e))),
        })
    }
}
