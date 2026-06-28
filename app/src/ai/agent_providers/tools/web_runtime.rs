//! Local execution logic for BYOP `webfetch` and `websearch` tools.
//!
//! These two BYOP tools do not use the protobuf executor (warp_multi_agent_api has no
//! corresponding variant). Instead, `chat_stream.rs::handle_byop_web_tool_intercept` calls
//! this module directly before `parse_incoming_tool_call`, composing results into a pair
//! of `(ToolCall carrier, ToolCallResult)` messages that are pushed back into the stream.
//!
//! ## Alignment with opencode
//!
//! - `webfetch` mirrors `packages/opencode/src/tool/webfetch.ts`:
//!   * User-Agent defaults to Chrome; 403 + `cf-mitigated: challenge` → retry once with `Zaplex` UA
//!   * `Accept` header negotiated by format parameter q priority
//!   * Content-Length pre-check + actual byte double-check, 5 MB limit
//!   * Timeout defaults to 30s, max 120s
//!   * Image mime automatically base64-encoded → output.attachments
//! - `websearch` mirrors `packages/opencode/src/tool/{websearch,mcp-exa}.ts`:
//!   * Anonymous by default: `https://mcp.exa.ai/mcp`; if `EXA_API_KEY` env var exists, append to querystring
//!   * 25s timeout
//!   * SSE response → `result.content[0].text`

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, CONTENT_LENGTH, CONTENT_TYPE, USER_AGENT};
use reqwest::redirect::Policy;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
use std::time::Duration;

use super::exa;

// ---------------------------------------------------------------------------
// Constants (aligned with opencode webfetch.ts:8-10)
// ---------------------------------------------------------------------------

pub const MAX_RESPONSE_SIZE: usize = 5 * 1024 * 1024; // 5 MB
pub const DEFAULT_FETCH_TIMEOUT_SECS: u64 = 30;
pub const MAX_FETCH_TIMEOUT_SECS: u64 = 120;
pub const SEARCH_TIMEOUT_SECS: u64 = 25;

pub const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";
pub const FALLBACK_UA: &str = "Zaplex";

// ---------------------------------------------------------------------------
// webfetch
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum FetchFormat {
    #[default]
    Markdown,
    Text,
    Html,
}

impl FetchFormat {
    fn accept_header(&self) -> &'static str {
        match self {
            Self::Markdown => {
                "text/markdown;q=1.0, text/x-markdown;q=0.9, text/plain;q=0.8, \
                 text/html;q=0.7, */*;q=0.1"
            }
            Self::Text => "text/plain;q=1.0, text/markdown;q=0.9, text/html;q=0.8, */*;q=0.1",
            Self::Html => {
                "text/html;q=1.0, application/xhtml+xml;q=0.9, text/plain;q=0.8, \
                 text/markdown;q=0.7, */*;q=0.1"
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FetchArgs {
    pub url: String,
    #[serde(default)]
    pub format: Option<FetchFormat>,
    /// Unit: seconds. `None` → 30s; max 120s, excess is clamped.
    #[serde(default)]
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FetchAttachment {
    pub mime: String,
    /// `data:<mime>;base64,<...>` format (aligned with opencode).
    pub url: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FetchOutput {
    pub url: String,
    pub status: u16,
    pub content_type: String,
    pub format: String,
    pub output: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<FetchAttachment>,
}

/// Returns `true` if the IP belongs to ranges that webfetch should not access
/// (private, loopback, link-local, etc.).
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_ipv4(v4),
        IpAddr::V6(v6) => {
            // IPv4-mapped IPv6 (::ffff:x.x.x.x) is handled by IPv4 rules.
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_blocked_ipv4(mapped);
            }
            v6.is_loopback()               // ::1
                || v6.is_unspecified()      // ::
                || v6.is_multicast()        // ff00::/8
                || is_ipv6_unique_local(v6) // fc00::/7
                || is_ipv6_link_local(v6)   // fe80::/10
                || is_ipv6_documentation(v6) // Documentation example addresses 2001:db8::/32
        }
    }
}

fn is_blocked_ipv4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    v4.is_loopback()          // 127.0.0.0/8
        || v4.is_private()    // 10/8, 172.16/12, 192.168/16
        || v4.is_link_local() // 169.254.0.0/16
        || v4.is_multicast()  // 224.0.0.0/4
        || o[0] == 0          // 0.0.0.0/8, “this host” range
        || v4.is_broadcast()  // 255.255.255.255
        || (Ipv4Addr::new(100, 64, 0, 0) <= v4 && v4 <= Ipv4Addr::new(100, 127, 255, 255))
            // CGNAT 100.64/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 2)   // TEST-NET-1 192.0.2.0/24
        || (o[0] == 198 && o[1] == 51 && o[2] == 100) // TEST-NET-2 198.51.100.0/24
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)  // TEST-NET-3 203.0.113.0/24
        || (o[0] == 198 && (o[1] & 0xfe) == 18)       // Performance test addresses 198.18.0.0/15
        || o[0] >= 240 // Reserved addresses 240.0.0.0/4
}

fn is_ipv6_unique_local(v6: Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xfe00) == 0xfc00
}

fn is_ipv6_link_local(v6: Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xffc0) == 0xfe80
}

fn is_ipv6_documentation(v6: Ipv6Addr) -> bool {
    v6.segments()[0] == 0x2001 && v6.segments()[1] == 0x0db8
}

/// Validates SSRF safety of a URL: rejects private/internal IP ranges after DNS resolution.
fn validate_url_not_internal(url_str: &str) -> Result<()> {
    let parsed = url::Url::parse(url_str).context("invalid URL")?;
    let host = parsed.host_str().context("URL has no host")?;

    // If the host is already an IP literal, check it directly.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_blocked_ip(ip) {
            bail!("URL targets a blocked IP address range");
        }
    }

    // Additionally resolve the hostname to detect DNS results pointing to internal IPs early.
    // Port 0 is used here because only the address itself is needed.
    if let Ok(addrs) = std::net::ToSocketAddrs::to_socket_addrs(&(host, 0)) {
        for addr in addrs {
            if is_blocked_ip(addr.ip()) {
                bail!("URL resolves to a blocked IP address range");
            }
        }
    }

    Ok(())
}

/// DNS resolver filters out blocked internal IPs during resolution to avoid TOCTOU gaps
/// between pre-validation and connection.
///
/// Only available for non-WASM targets: reqwest's `dns` module and
/// `ClientBuilder::dns_resolver` are not exposed to WebAssembly.
#[cfg(not(target_arch = "wasm32"))]
struct SsrfSafeResolver;

#[cfg(not(target_arch = "wasm32"))]
impl reqwest::dns::Resolve for SsrfSafeResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            use std::net::ToSocketAddrs;
            let lookup_host = host.clone();
            let addrs: Vec<std::net::SocketAddr> = tokio::task::spawn_blocking(
                move || -> std::io::Result<Vec<std::net::SocketAddr>> {
                    Ok((lookup_host.as_str(), 0)
                        .to_socket_addrs()?
                        .filter(|addr| !is_blocked_ip(addr.ip()))
                        .collect())
                },
            )
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
            if addrs.is_empty() {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("DNS for '{host}' resolved to blocked IPs (SSRF protection)"),
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Maximum number of redirects, consistent with reqwest default.
const MAX_REDIRECT_HOPS: usize = 10;

/// Builds an SSRF-protected reqwest client:
/// - Custom DNS resolver to block connections to internal IPs
/// - Custom redirect policy to enforce HTTPS, validate each hop, and limit total hops
///   (`Policy::custom` does not inherit reqwest's default hop limits)
pub fn build_ssrf_safe_client() -> Result<reqwest::Client> {
    let policy = Policy::custom(|attempt| {
        // `Policy::custom` does not inherit reqwest's default loop/hop-count protection;
        // explicit limit is required.
        if attempt.previous().len() >= MAX_REDIRECT_HOPS {
            return attempt.stop();
        }
        let url = attempt.url();
        // Redirect targets must maintain HTTPS to prevent HTTPS → HTTP downgrade.
        if url.scheme() != "https" {
            return attempt.stop();
        }
        // Perform an additional validation layer outside the DNS resolver to immediately
        // block internal addresses in IP literal form.
        if validate_url_not_internal(url.as_str()).is_err() {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    let builder = reqwest::Client::builder()
        .redirect(policy)
        .pool_idle_timeout(Duration::from_secs(30));
    // Only connect SSRF-safe DNS resolver for non-WASM targets;
    // WebAssembly does not expose the reqwest DNS module.
    #[cfg(not(target_arch = "wasm32"))]
    let builder = builder.dns_resolver(Arc::new(SsrfSafeResolver));
    builder.build().context("build SSRF-safe reqwest client")
}

/// Entry point: execute a single webfetch and return structured output
/// (to be passed to upstream LLM by caller via `serde_json::to_value`).
pub async fn run_webfetch(client: &reqwest::Client, args: FetchArgs) -> Result<FetchOutput> {
    if !args.url.starts_with("https://") {
        bail!("URL must use HTTPS");
    }
    validate_url_not_internal(&args.url)?;
    let format = args.format.clone().unwrap_or_default();
    let timeout_secs = args
        .timeout
        .unwrap_or(DEFAULT_FETCH_TIMEOUT_SECS)
        .min(MAX_FETCH_TIMEOUT_SECS);
    let timeout = Duration::from_secs(timeout_secs);

    let accept = format.accept_header();
    let resp = match send_fetch(client, &args.url, accept, CHROME_UA, timeout).await {
        Ok(r) => r,
        Err(e) => return Err(e),
    };

    // Cloudflare challenge: Chrome UA first round 403 + cf-mitigated: challenge → retry once with different UA.
    let resp = if resp.status() == StatusCode::FORBIDDEN
        && resp
            .headers()
            .get("cf-mitigated")
            .and_then(|v| v.to_str().ok())
            == Some("challenge")
    {
        log::info!("[webfetch] cloudflare challenge detected → retry with fallback UA");
        send_fetch(client, &args.url, accept, FALLBACK_UA, timeout).await?
    } else {
        resp
    };

    response_to_fetch_output(resp, &args.url, &format).await
}

/// Shared Response → FetchOutput conversion logic.
///
/// Called by both `run_webfetch` and test helper functions to avoid duplicating
/// status checks, size limits, image encoding, and JSON prettification logic.
async fn response_to_fetch_output(
    resp: reqwest::Response,
    url: &str,
    format: &FetchFormat,
) -> Result<FetchOutput> {
    let status = resp.status();
    if !status.is_success() {
        bail!("HTTP {} fetching {url}", status.as_u16());
    }

    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let mime = content_type
        .split(';')
        .next()
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();

    // Content-Length pre-check
    if let Some(len_str) = resp
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
    {
        if let Ok(len) = len_str.parse::<usize>() {
            if len > MAX_RESPONSE_SIZE {
                bail!(
                    "Response too large (Content-Length {len} > {MAX_RESPONSE_SIZE} bytes limit)"
                );
            }
        }
    }

    let bytes = resp.bytes().await.context("read response body")?;
    if bytes.len() > MAX_RESPONSE_SIZE {
        bail!(
            "Response too large ({} bytes > {} bytes limit)",
            bytes.len(),
            MAX_RESPONSE_SIZE
        );
    }

    // Image → base64 attachment
    if is_image_mime(&mime) {
        let encoded = BASE64.encode(&bytes);
        let data_url = format!("data:{mime};base64,{encoded}");
        return Ok(FetchOutput {
            url: url.to_owned(),
            status: status.as_u16(),
            content_type,
            format: format!("{format:?}").to_ascii_lowercase(),
            output: "Image fetched successfully".to_owned(),
            attachments: vec![FetchAttachment {
                mime,
                url: data_url,
            }],
        });
    }

    let body_str = String::from_utf8_lossy(&bytes).into_owned();
    let is_html = mime == "text/html" || mime == "application/xhtml+xml";

    let output = match format {
        FetchFormat::Markdown if is_html => html_to_markdown(&body_str),
        FetchFormat::Text if is_html => extract_text_from_html(&body_str),
        FetchFormat::Html => body_str,
        // markdown / text but mime is not html → pass through (already text type)
        _ => body_str,
    };

    Ok(FetchOutput {
        url: url.to_owned(),
        status: status.as_u16(),
        content_type,
        format: format!("{format:?}").to_ascii_lowercase(),
        output: maybe_format_json(&output, &mime),
        attachments: vec![],
    })
}

async fn send_fetch(
    client: &reqwest::Client,
    url: &str,
    accept: &str,
    ua: &str,
    timeout: Duration,
) -> Result<reqwest::Response> {
    client
        .get(url)
        .header(USER_AGENT, ua)
        .header(ACCEPT, accept)
        .header(ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .timeout(timeout)
        .send()
        .await
        .with_context(|| format!("HTTP GET {url}"))
}

fn is_image_mime(mime: &str) -> bool {
    mime.starts_with("image/")
}

/// If mime is application/json and content is valid JSON, prettify as a ```json``` code block
/// (aligned with zed fetch_tool.rs JSON handling).
fn maybe_format_json(content: &str, mime: &str) -> String {
    if mime != "application/json" {
        return content.to_owned();
    }
    match serde_json::from_str::<Value>(content) {
        Ok(v) => match serde_json::to_string_pretty(&v) {
            Ok(pretty) => format!("```json\n{pretty}\n```"),
            Err(_) => content.to_owned(),
        },
        Err(_) => content.to_owned(),
    }
}

fn html_to_markdown(html: &str) -> String {
    // htmd default configuration is aligned with common Turndown output style
    // (atx headings, fenced code blocks, etc.).
    // Pre-strip script / style / noscript / iframe content (htmd defaults to keeping
    // text within these tags as plain text, which pollutes markdown output).
    let pre = strip_unsafe_blocks(html);
    match std::panic::catch_unwind(|| htmd::convert(&pre)) {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            log::warn!("[webfetch] htmd convert error: {e}, falling back to text extraction");
            naive_html_strip(&pre)
        }
        Err(_) => {
            log::warn!("[webfetch] htmd panicked, falling back to text extraction");
            naive_html_strip(&pre)
        }
    }
}

/// Strip entire `<script>...</script>` / `<style>...</style>` / `<noscript>...</noscript>` /
/// `<iframe>...</iframe>` blocks (case-insensitive, attributes allowed).
fn strip_unsafe_blocks(html: &str) -> String {
    let mut out = html.to_owned();
    for tag in &["script", "style", "noscript", "iframe", "object", "embed"] {
        out = strip_tag_block(&out, tag);
    }
    out
}

fn strip_tag_block(html: &str, tag: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some(rel_open) = lower[cursor..].find(&open) {
        let abs_open = cursor + rel_open;
        // Must be followed by `>` or whitespace (to avoid consuming <scriptlike>)
        let after = abs_open + open.len();
        match html.as_bytes().get(after) {
            Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'/') => {}
            _ => {
                out.push_str(&html[cursor..=abs_open]);
                cursor = abs_open + 1;
                continue;
            }
        }
        out.push_str(&html[cursor..abs_open]);
        // Find closing tag
        match lower[after..].find(&close) {
            Some(rel_close) => {
                cursor = after + rel_close + close.len();
            }
            None => {
                // No closing tag → discard entire block
                cursor = html.len();
                break;
            }
        }
    }
    out.push_str(&html[cursor..]);
    out
}

/// Minimal HTML→plain text fallback: regex-strip all tags. Used only when htmd fails.
fn naive_html_strip(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// HTML → plain text: first convert to markdown using htmd, then strip markdown markup.
///
/// Simplified approach to avoid introducing html5ever DOM traversal dependencies
/// (`markup5ever_rcdom`). htmd internally filters out invisible tags like script/style/noscript,
/// so plain text output is sufficient for text mode.
fn extract_text_from_html(html: &str) -> String {
    let md = html_to_markdown(html);
    strip_markdown(&md)
}

fn strip_markdown(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut last_blank = false;
    for raw_line in md.lines() {
        let mut line = raw_line.trim().to_owned();
        // Strip heading prefixes # ## ###
        while line.starts_with('#') {
            line.remove(0);
        }
        let line = line.trim_start();
        // Strip list / quote / horizontal line prefixes
        let line = line.trim_start_matches(['-', '*', '>', '+']).trim_start();
        // ![alt](url) → remove entire pattern
        let line = strip_pattern(line, "![", ")");
        // [text](url) → keep text
        let line = unwrap_links(&line);
        // `code` / **bold** / *em* / _em_ — conservatively remove ` * _
        let cleaned: String = line
            .chars()
            .filter(|c| !matches!(c, '`' | '*' | '_'))
            .collect();
        let trimmed = cleaned.trim();
        if trimmed.is_empty() {
            if !last_blank && !out.is_empty() {
                out.push('\n');
                last_blank = true;
            }
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(trimmed);
        last_blank = false;
    }
    out
}

fn strip_pattern(s: &str, start: &str, end: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(start) {
        out.push_str(&rest[..i]);
        let after = &rest[i + start.len()..];
        match after.find(end) {
            Some(j) => rest = &after[j + end.len()..],
            None => {
                // No closing delimiter; keep remainder
                rest = after;
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `[text](url)` → `text`.
fn unwrap_links(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            // Find ]( then )
            if let Some(close_text) = s[i + 1..].find("](") {
                let text_end = i + 1 + close_text;
                if let Some(close_url) = s[text_end + 2..].find(')') {
                    let url_end = text_end + 2 + close_url;
                    out.push_str(&s[i + 1..text_end]);
                    i = url_end + 1;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// websearch
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct SearchToolArgs {
    pub query: String,
    #[serde(rename = "numResults", default)]
    pub num_results: Option<u32>,
    #[serde(default)]
    pub livecrawl: Option<String>,
    #[serde(rename = "type", default)]
    pub search_type: Option<String>,
    #[serde(rename = "contextMaxCharacters", default)]
    pub context_max_characters: Option<u32>,
}

impl SearchToolArgs {
    pub fn into_exa_args(self) -> exa::SearchArgs {
        let mut a = exa::SearchArgs::with_defaults(self.query);
        if let Some(n) = self.num_results {
            a.num_results = n;
        }
        if let Some(s) = self.livecrawl {
            a.livecrawl = s;
        }
        if let Some(t) = self.search_type {
            a.search_type = t;
        }
        if let Some(c) = self.context_max_characters {
            a.context_max_characters = Some(c);
        }
        a
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SearchOutput {
    pub query: String,
    /// Human-readable / LLM-optimized context string returned by Exa.
    pub results: String,
}

const EMPTY_FALLBACK: &str = "No search results found. Please try a different query.";

/// Entry point: execute a single Exa websearch.
///
/// `endpoint_override`: for testing; defaults to `exa::endpoint_url(api_key)`.
/// `api_key`: `None` → anonymous; `Some(...)` → append to querystring.
pub async fn run_websearch(
    client: &reqwest::Client,
    args: SearchToolArgs,
    api_key: Option<&str>,
    endpoint_override: Option<&str>,
) -> Result<SearchOutput> {
    let query = args.query.clone();
    let exa_args = args.into_exa_args();
    let body = exa::build_request_body(exa::SEARCH_TOOL_NAME, &exa_args);

    let url = endpoint_override
        .map(|s| s.to_owned())
        .unwrap_or_else(|| exa::endpoint_url(api_key));

    let resp = client
        .post(&url)
        .header(ACCEPT, "application/json, text/event-stream")
        .header(CONTENT_TYPE, "application/json")
        .timeout(Duration::from_secs(SEARCH_TIMEOUT_SECS))
        .json(&body)
        .send()
        .await
        .with_context(|| format!("Exa POST {url}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        bail!("Exa returned HTTP {} ({})", status.as_u16(), body_text);
    }
    let body_text = resp.text().await.context("read Exa SSE body")?;

    let parsed = exa::parse_sse_body(&body_text)?;
    let results = parsed.unwrap_or_else(|| EMPTY_FALLBACK.to_owned());
    Ok(SearchOutput { query, results })
}

/// Serializes structured results from webfetch / websearch to JSON Value
/// (string presented to upstream LLM).
///
/// All BYOP locally-intercepted tool results must include the `"_byop_intercepted":true` sentinel;
/// otherwise the controller (`controller.rs:2693+`) will not trigger auto-resume and the model
/// will hang waiting for results. See `chat_stream::dispatch_byop_web_tool` and the controller's
/// `needs_byop_local_resume` check.
pub fn fetch_output_to_json(out: &FetchOutput) -> Value {
    let mut v = serde_json::to_value(out).unwrap_or_else(|_| json!({"status": "serialize_error"}));
    if let Some(obj) = v.as_object_mut() {
        obj.insert("_byop_intercepted".to_owned(), Value::Bool(true));
    }
    v
}
pub fn search_output_to_json(out: &SearchOutput) -> Value {
    let mut v = serde_json::to_value(out).unwrap_or_else(|_| json!({"status": "serialize_error"}));
    if let Some(obj) = v.as_object_mut() {
        obj.insert("_byop_intercepted".to_owned(), Value::Bool(true));
    }
    v
}
pub fn error_to_json(tool: &str, e: &anyhow::Error) -> Value {
    json!({
        "_byop_intercepted": true,
        "status": "error",
        "tool": tool,
        "message": format!("{e:#}"),
    })
}

#[cfg(test)]
#[path = "webfetch_tests.rs"]
mod webfetch_tests;
#[cfg(test)]
#[path = "websearch_tests.rs"]
mod websearch_tests;
