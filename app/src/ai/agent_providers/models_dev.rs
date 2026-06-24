//! models.dev data source integration.
//!
//! When the user opens the Providers settings page, the background asynchronously fetches
//! `https://models.dev/api.json` and caches it to `${cache_dir}/models-dev.json`. On the next
//! startup, the cache is read directly. If the cache hit is within the TTL (default 24h),
//! no request is made; otherwise, fetch on expiry/missing.
//!
//! Data structure aligns with opencode's `provider/models.ts`: top level is
//! `{ <provider_id>: Provider }`, where Provider contains `models: { <model_id>: Model }`.
//! We only care about a few fields needed for UI "quick selection":
//! - provider: id / name / api / env (indicates which env var is needed)
//! - model:    id / name / limit.context / limit.output / reasoning / tool_call
//!
//! Unlisted fields use `serde(default)` + `#[allow(dead_code)]` for tolerance.
//!
//! Design tradeoff: **synchronous cache read, asynchronous network fetch**. Read side
//! is for UI and must be fast; fetch side spawns in background, fails silently with only
//! logging. If cache cannot be read, return empty data, and UI displays
//! "models.dev not yet fetched, please check network".

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::sync::RwLock;
use std::time::{Duration, SystemTime};

use http_client::Client;
use serde::{Deserialize, Serialize};

const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const CACHE_FILENAME: &str = "models-dev.json";
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// models.dev top-level data — provider_id → Provider.
pub type Catalog = BTreeMap<String, Provider>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Provider {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Upstream API base URL, e.g., `https://api.deepseek.com/v1`.
    #[serde(default)]
    pub api: Option<String>,
    /// Environment variable names typically required by this provider, e.g., `["DEEPSEEK_API_KEY"]`.
    #[serde(default)]
    pub env: Vec<String>,
    /// Available models, keyed by model id.
    #[serde(default)]
    pub models: BTreeMap<String, Model>,
    /// Documentation URL (present for some providers).
    #[serde(default)]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Model {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default = "default_true")]
    pub tool_call: bool,
    /// Whether file attachments are supported (attachment field complements modalities:
    /// modalities describe native multimodality; attachment covers PDF / generic file attachment protocol).
    #[serde(default)]
    pub attachment: bool,
    /// Input / output modalities, typical values: `text` / `image` / `audio` / `video` / `pdf`.
    #[serde(default)]
    pub modalities: ModelModalities,
    /// Context window upper limit.
    #[serde(default)]
    pub limit: ModelLimit,
    /// Status tags: "alpha" / "beta" / "deprecated".
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelModalities {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
}

impl ModelModalities {
    pub fn supports_input(&self, modality: &str) -> bool {
        self.input.iter().any(|m| m.eq_ignore_ascii_case(modality))
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelLimit {
    #[serde(default)]
    pub context: u32,
    #[serde(default)]
    pub output: u32,
}

// ── In-process singleton cache ───────────────────────────────────────────

#[derive(Debug, Default)]
struct State {
    /// Loaded catalog. `None` indicates never successfully loaded.
    catalog: Option<Catalog>,
    /// Cache last modification time (used to determine expiration).
    loaded_at: Option<SystemTime>,
}

fn state() -> &'static RwLock<State> {
    static S: OnceLock<RwLock<State>> = OnceLock::new();
    S.get_or_init(|| RwLock::new(State::default()))
}

fn cache_path() -> PathBuf {
    let mut p = warp_core::paths::cache_dir();
    p.push(CACHE_FILENAME);
    p
}

/// Read a copy of the loaded catalog (no lock waiting — direct clone).
/// Returns `None` if no data; UI should display "fetching" / retry button.
pub fn cached() -> Option<Catalog> {
    state().read().ok().and_then(|s| s.catalog.clone())
}

/// A capability snapshot of a model extracted from models.dev, used for BYOP UI / chat_stream attachment type decisions.
#[derive(Debug, Clone, Default)]
pub struct ModelCaps {
    pub vision: bool,
    pub pdf: bool,
    pub audio: bool,
    pub attachment: bool,
}

impl ModelCaps {
    pub fn from_model(m: &Model) -> Self {
        Self {
            vision: m.modalities.supports_input("image"),
            pdf: m.modalities.supports_input("pdf") || m.attachment,
            audio: m.modalities.supports_input("audio"),
            attachment: m.attachment,
        }
    }
}

/// Look up a model by model_id in the loaded catalog and return the capabilities
/// declared for that model on models.dev.
///
/// First attempts exact match using `provider_id` as the catalog provider key;
/// on miss, degrades to "scan entire catalog for first model.id hit".
/// This allows both exact matching (when user-supplied provider.id matches models.dev)
/// and user-defined provider ids (e.g., "openrouter" or "siliconflow" — aggregator
/// providers that forward upstream models with different ids than models.dev upstream providers).
pub fn lookup_caps(provider_id: &str, model_id: &str) -> Option<ModelCaps> {
    let s = state().read().ok()?;
    let catalog = s.catalog.as_ref()?;
    if let Some(p) = catalog.get(provider_id) {
        if let Some(m) = p.models.get(model_id) {
            return Some(ModelCaps::from_model(m));
        }
    }
    for p in catalog.values() {
        if let Some(m) = p.models.get(model_id) {
            return Some(ModelCaps::from_model(m));
        }
    }
    None
}

/// Load disk cache into memory (synchronous, non-blocking; called only on process startup
/// or first UI need). Returns false if disk cache doesn't exist or fails to parse;
/// caller should trigger a network fetch.
pub fn load_from_disk() -> bool {
    let path = cache_path();
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let mtime = std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok());
    match serde_json::from_slice::<Catalog>(&bytes) {
        Ok(catalog) => {
            if let Ok(mut s) = state().write() {
                s.catalog = Some(catalog);
                s.loaded_at = mtime;
            }
            true
        }
        Err(e) => {
            log::warn!("[models.dev] Failed to parse disk cache ({path:?}): {e}");
            false
        }
    }
}

/// Whether cache is stale — doesn't exist or exceeds TTL.
pub fn is_stale() -> bool {
    let s = match state().read() {
        Ok(s) => s,
        Err(_) => return true,
    };
    match s.loaded_at {
        Some(t) => SystemTime::now()
            .duration_since(t)
            .map(|d| d > CACHE_TTL)
            .unwrap_or(true),
        None => true,
    }
}

/// Asynchronously fetch models.dev and write to disk and memory cache.
/// Failures only log, do not propagate upward (UI caller decides display based on whether `cached()` is `Some`).
pub async fn fetch_and_cache(client: Client) -> Result<(), String> {
    let resp = client
        .get(MODELS_DEV_URL)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    let catalog: Catalog =
        serde_json::from_slice(&bytes).map_err(|e| format!("JSON parse failed: {e}"))?;

    // Write to disk — failure is not fatal, only log.
    let path = cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, &bytes) {
        log::warn!("[models.dev] Failed to write disk cache ({path:?}): {e}");
    }

    if let Ok(mut s) = state().write() {
        s.catalog = Some(catalog);
        s.loaded_at = Some(SystemTime::now());
    }
    Ok(())
}

// ── Chip row collapse/expand state (process-level, avoids widget rebuild loss) ─

static CHIPS_EXPANDED: AtomicBool = AtomicBool::new(false);

pub fn chips_expanded() -> bool {
    CHIPS_EXPANDED.load(Ordering::Relaxed)
}

pub fn toggle_chips_expanded() {
    CHIPS_EXPANDED.fetch_xor(true, Ordering::Relaxed);
}

// ── Search filter for quick chip row addition ────────────────────────────

fn search_state() -> &'static RwLock<String> {
    static S: OnceLock<RwLock<String>> = OnceLock::new();
    S.get_or_init(|| RwLock::new(String::new()))
}

pub fn search_query() -> String {
    search_state()
        .read()
        .ok()
        .map(|s| s.clone())
        .unwrap_or_default()
}

pub fn set_search_query(q: String) {
    if let Ok(mut s) = search_state().write() {
        *s = q;
    }
}

/// Filter catalog by current search query, case-insensitive substring matching on
/// provider.name and provider.id. Empty query returns all entries in order.
/// Returns owned Vec so UI can take/iter.
pub fn filter_catalog(catalog: &Catalog, query: &str) -> Vec<(String, Provider)> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return catalog
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
    }
    catalog
        .iter()
        .filter(|(id, p)| id.to_lowercase().contains(&q) || p.name.to_lowercase().contains(&q))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Convert a models.dev Model to a local settings AgentProviderModel.
///
/// By default, writes catalog-inferred image/pdf/audio into fields (so on first user
/// sync / quick-add, model capabilities are directly visible in toml without needing
/// to expand details). On subsequent syncs, callers only fill `None` slots with new values;
/// `Some(_)` is treated as explicit user override to skip.
pub fn into_agent_provider_model(model: &Model) -> crate::settings::AgentProviderModel {
    let caps = ModelCaps::from_model(model);
    crate::settings::AgentProviderModel {
        name: if model.name.is_empty() {
            model.id.clone()
        } else {
            model.name.clone()
        },
        id: model.id.clone(),
        context_window: model.limit.context,
        max_output_tokens: model.limit.output,
        reasoning: model.reasoning,
        tool_call: model.tool_call,
        image: Some(caps.vision),
        pdf: Some(caps.pdf),
        audio: Some(caps.audio),
    }
}
