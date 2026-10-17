//! Per-model pricing table for deriving cost from token counts.
//!
//! **Approximation, by design.** LLM prices drift; this table is centralized and
//! overridable. It contains only bundled list prices, never values fetched from a
//! remote endpoint at runtime. Unknown models are unpriced rather than represented
//! as a real zero cost. Rates are USD per 1,000,000 tokens.

use serde::{Deserialize, Serialize};

/// Price of one model, USD per 1M tokens, with distinct cache rates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    /// Cost to *write* the prompt cache (Claude 5m ephemeral rate). 0 where N/A.
    pub cache_write: f64,
    /// Cost to *read* from the prompt cache.
    pub cache_read: f64,
}

/// The higher rate a request pays once its prompt is longer than a threshold.
///
/// Vendors bill such a request at this rate for **all** of its tokens, not
/// only for the part above the threshold. The prompt is everything the request
/// read: uncached input, cache writes and cache reads.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LongContextPrice {
    /// Prompts longer than this many tokens pay `price`.
    above_prompt_tokens: u64,
    price: ModelPrice,
}

/// Where a local price estimate came from. The cockpit deliberately keeps this
/// separate from actual account billing: transcript tokens plus a static list
/// price are an estimate, not an invoice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PricingSource {
    /// Bundled API list price (standard tier), verified on 2026-10-16 against
    /// the vendor's public pricing documentation. It is intentionally static:
    /// Zaplex never invents or fetches a runtime price for an unknown model.
    ///
    /// Anthropic: <https://platform.claude.com/docs/en/about-claude/pricing>
    /// OpenAI: <https://developers.openai.com/api/docs/pricing> and the model
    /// pages under <https://developers.openai.com/api/docs/models>
    BundledListPrice,
    /// A caller-supplied static table. It is still an estimate, but Zaplex does
    /// not claim a vendor source for it.
    CustomStatic,
}

impl PricingSource {
    /// Date the bundled list prices were last verified against their published
    /// vendor sources. Custom static prices have no vendor verification date.
    pub const fn as_of(self) -> Option<&'static str> {
        match self {
            PricingSource::BundledListPrice => Some("2026-10-16"),
            PricingSource::CustomStatic => None,
        }
    }
}

/// A local calculation from one transcript turn and a static price table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CostEstimate {
    pub usd: f64,
    pub source: PricingSource,
}

impl CostEstimate {
    pub fn is_estimate(self) -> bool {
        true
    }
}

/// How an entry's key is compared with a transcript's (lowercased) model id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyMatch {
    /// The key may occur anywhere in the id: caller-supplied tables.
    Substring,
    /// The id is the key itself, optionally pinned to a dated snapshot
    /// (`-20251001`, `-2025-08-07`). Any other suffix names a different model
    /// (`-5` for a newer minor version, `-mini` for a cheaper variant), which
    /// must stay unpriced rather than borrow this entry's rate.
    ModelId,
}

#[derive(Clone, Debug)]
struct PriceEntry {
    key: String,
    matching: KeyMatch,
    price: ModelPrice,
    long_context: Option<LongContextPrice>,
    source: PricingSource,
}

impl PriceEntry {
    fn matches(&self, model: &str) -> bool {
        match self.matching {
            KeyMatch::Substring => model.contains(self.key.as_str()),
            KeyMatch::ModelId => model
                .strip_prefix(self.key.as_str())
                .is_some_and(|suffix| suffix.is_empty() || is_dated_snapshot(suffix)),
        }
    }
}

/// Whether `suffix` pins a model id to a dated snapshot: `-20251001`
/// (Anthropic) or `-2025-08-07` (OpenAI).
fn is_dated_snapshot(suffix: &str) -> bool {
    let Some(date) = suffix.strip_prefix('-') else {
        return false;
    };
    let bytes = date.as_bytes();
    match bytes.len() {
        8 => bytes.iter().all(u8::is_ascii_digit),
        10 => bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        }),
        _ => false,
    }
}

/// A model → price table, matched case-insensitively, first match wins. The
/// bundled table names exact model ids; a caller-supplied table matches its
/// keys as substrings (order those entries specific → general).
#[derive(Clone, Debug)]
pub struct PricingTable {
    entries: Vec<PriceEntry>,
}

impl PricingTable {
    /// Build a table from `(substring, price)` pairs (already ordered specific→general).
    pub fn new(entries: Vec<(String, ModelPrice)>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|(key, price)| PriceEntry {
                    key: key.to_ascii_lowercase(),
                    matching: KeyMatch::Substring,
                    price,
                    long_context: None,
                    source: PricingSource::CustomStatic,
                })
                .collect(),
        }
    }

    fn bundled(entries: Vec<(&str, ModelPrice, Option<LongContextPrice>)>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|(key, price, long_context)| PriceEntry {
                    key: key.to_ascii_lowercase(),
                    matching: KeyMatch::ModelId,
                    price,
                    long_context,
                    source: PricingSource::BundledListPrice,
                })
                .collect(),
        }
    }

    fn entry_for(&self, model: &str) -> Option<&PriceEntry> {
        let model = model.to_ascii_lowercase();
        self.entries.iter().find(|entry| entry.matches(&model))
    }

    /// The standard (short-context) price of the entry that matches `model`.
    pub fn price_for(&self, model: &str) -> Option<ModelPrice> {
        self.entry_for(model).map(|entry| entry.price)
    }

    /// Estimate one turn's API-list-price cost. Codex `output` already includes
    /// its reasoning-token breakdown. A turn whose prompt exceeds the model's
    /// long-context threshold is priced entirely at the long-context rate.
    /// `None` means the model is unpriced, never that it costs zero.
    pub fn estimate_for(
        &self,
        model: &str,
        input: u64,
        output: u64,
        cache_create: u64,
        cache_read: u64,
    ) -> Option<CostEstimate> {
        let entry = self.entry_for(model)?;
        let prompt = input
            .saturating_add(cache_create)
            .saturating_add(cache_read);
        let p = match entry.long_context {
            Some(long) if prompt > long.above_prompt_tokens => long.price,
            Some(_) | None => entry.price,
        };
        let usd = (input as f64 * p.input
            + output as f64 * p.output
            + cache_create as f64 * p.cache_write
            + cache_read as f64 * p.cache_read)
            / 1_000_000.0;
        Some(CostEstimate {
            usd,
            source: entry.source,
        })
    }

    /// The numeric part of [`Self::estimate_for`]. `None` is an unpriced model.
    pub fn cost_for(
        &self,
        model: &str,
        input: u64,
        output: u64,
        cache_create: u64,
        cache_read: u64,
    ) -> Option<f64> {
        self.estimate_for(model, input, output, cache_create, cache_read)
            .map(|estimate| estimate.usd)
    }
}

impl Default for PricingTable {
    /// Seeded from current Anthropic + OpenAI list prices (standard tier, global
    /// processing: no data-residency, fast-mode or batch modifiers). Keys are the
    /// exact model ids transcripts record in their `model` field.
    ///
    /// TODO(pricing): refresh on model launches; Codex transcripts and
    /// `~/.codex/models_cache.json` carry no price fields.
    fn default() -> Self {
        let m = |input, output, cache_write, cache_read| ModelPrice {
            input,
            output,
            cache_write,
            cache_read,
        };
        let above = |above_prompt_tokens, price| {
            Some(LongContextPrice {
                above_prompt_tokens,
                price,
            })
        };
        // Exact supported model IDs (plus their dated snapshot suffixes) only.
        // A broad family key would make a new model appear priced using an older
        // sibling's rate, which is less honest than reporting it as unpriced.
        Self::bundled(vec![
            // --- Anthropic (Claude) ---
            // Source: https://platform.claude.com/docs/en/about-claude/pricing
            // (model pricing table), verified 2026-10-16. Cache writes use the
            // 5-minute rate. Claude 4.6 and later bill the full 1M context at
            // these rates, except Haiku 5.5: a request whose prompt (including
            // cache reads and writes) is over 100,000 tokens pays its higher
            // prices.
            ("claude-fable-5-1", m(10.0, 50.0, 12.50, 0.25), None),
            ("claude-mythos-5-1", m(10.0, 50.0, 12.50, 0.25), None),
            ("claude-fable-5", m(10.0, 50.0, 12.50, 1.0), None),
            ("claude-mythos-5", m(10.0, 50.0, 12.50, 1.0), None),
            ("claude-opus-5-5", m(4.0, 20.0, 5.0, 0.20), None),
            ("claude-opus-5", m(5.0, 25.0, 6.25, 0.50), None),
            ("claude-opus-4-8", m(5.0, 25.0, 6.25, 0.50), None),
            ("claude-opus-4-7", m(5.0, 25.0, 6.25, 0.50), None),
            ("claude-opus-4-6", m(5.0, 25.0, 6.25, 0.50), None),
            ("claude-opus-4-5", m(5.0, 25.0, 6.25, 0.50), None),
            ("claude-sonnet-5-5", m(2.0, 10.0, 2.50, 0.10), None),
            ("claude-sonnet-5", m(2.0, 10.0, 2.50, 0.20), None),
            ("claude-sonnet-4-6", m(3.0, 15.0, 3.75, 0.30), None),
            ("claude-sonnet-4-5", m(3.0, 15.0, 3.75, 0.30), None),
            (
                "claude-haiku-5-5",
                m(0.10, 0.50, 0.125, 0.01),
                above(100_000, m(0.50, 2.50, 0.625, 0.05)),
            ),
            ("claude-haiku-4-5", m(1.0, 5.0, 1.25, 0.10), None),
            // --- OpenAI (Codex) ---
            // Sources: https://developers.openai.com/api/docs/pricing (Standard
            // tab, short- and long-context columns) and the model pages under
            // https://developers.openai.com/api/docs/models/<id>, verified
            // 2026-10-16. A request with more than 272K input tokens pays the
            // long-context column for all of its tokens; 272K is read as 272,000,
            // the input share of the earlier 400,000-token windows (128,000 of
            // them output). Codex rollouts report cache reads but no cache
            // writes; the write rates are listed so a future write counter
            // cannot silently understate cost.
            (
                "gpt-6-astra",
                m(10.0, 50.0, 12.50, 1.0),
                above(272_000, m(20.0, 75.0, 25.0, 2.0)),
            ),
            (
                "gpt-6.1-sol",
                m(2.0, 10.0, 2.50, 0.10),
                above(272_000, m(4.0, 15.0, 5.0, 0.20)),
            ),
            (
                "gpt-6-sol",
                m(2.0, 10.0, 2.50, 0.20),
                above(272_000, m(4.0, 15.0, 5.0, 0.40)),
            ),
            (
                "gpt-6-luna",
                m(0.10, 0.50, 0.125, 0.01),
                above(272_000, m(0.20, 0.75, 0.25, 0.02)),
            ),
            // GPT-5.6 Sol's list price is promotional, "available at least
            // through November 21, 2026" per its model page.
            (
                "gpt-5.6-sol",
                m(4.0, 20.0, 5.0, 0.40),
                above(272_000, m(8.0, 30.0, 10.0, 0.80)),
            ),
            (
                "gpt-5.6-terra",
                m(2.0, 12.0, 2.50, 0.20),
                above(272_000, m(4.0, 18.0, 5.0, 0.40)),
            ),
            (
                "gpt-5.6-luna",
                m(0.20, 1.20, 0.25, 0.02),
                above(272_000, m(0.40, 1.80, 0.50, 0.04)),
            ),
            // Deprecated but still listed with these rates. Their pages list no
            // cache-write price, so the input rate stands in for one (Codex
            // rollouts record no cache writes for it to apply to).
            ("gpt-5.1-codex", m(1.25, 10.0, 1.25, 0.125), None),
            ("gpt-5-codex", m(1.25, 10.0, 1.25, 0.125), None),
            // GPT-5.5 stays unpriced: OpenAI lists no cache-write price for it
            // and words its >272K surcharge as applying "for the full session",
            // which a per-request estimate cannot reproduce honestly.
        ])
    }
}

#[cfg(test)]
#[path = "pricing_tests.rs"]
mod tests;
