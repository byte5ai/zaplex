//! Token estimation — aligned with opencode `packages/opencode/src/util/token.ts`.
//!
//! ```ts
//! const CHARS_PER_TOKEN = 4
//! export function estimate(input: string) {
//!   return Math.max(0, Math.round((input || "").length / CHARS_PER_TOKEN))
//! }
//! ```
//!
//! Use `chars().count()` instead of `len()` to avoid UTF-8 multi-byte characters skewing estimates.
//! In opencode's JS, `.length` is 1 for BMP characters and matches chars().count() in most cases;
//! for emoji outside BMP, JS is 2 (UTF-16 surrogate pair) while Rust chars().count() is 1 —
//! this small difference has no practical impact on head/tail splitting.
use super::consts::CHARS_PER_TOKEN;

/// Equivalent to `Math.round(len / 4)`. Returns 0 for empty string.
pub fn estimate(input: &str) -> usize {
    let n = input.chars().count();
    // Math.round was "round to even" (banker's rounding) before, but in JS behaves as standard rounding.
    // Here (n + 2) / 4 is equivalent to round(n / 4) for positive integers.
    (n + CHARS_PER_TOKEN / 2) / CHARS_PER_TOKEN
}

/// Estimate after JSON serialization — aligned with opencode `compaction.ts:241`:
/// `Token.estimate(JSON.stringify(msgs))`
pub fn estimate_json<T: serde::Serialize>(value: &T) -> usize {
    serde_json::to_string(value)
        .map(|s| estimate(&s))
        .unwrap_or(0)
}
