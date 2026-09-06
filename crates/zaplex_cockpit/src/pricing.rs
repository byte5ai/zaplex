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

/// Where a local price estimate came from. The cockpit deliberately keeps this
/// separate from actual account billing: transcript tokens plus a static list
/// price are an estimate, not an invoice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PricingSource {
    /// Bundled API list price, verified on 2026-07-31 from the vendor's public
    /// pricing page. It is intentionally static: Zaplex never invents or fetches
    /// a runtime price for an unknown model.
    ///
    /// OpenAI: <https://developers.openai.com/api/docs/models>
    /// Anthropic: <https://docs.anthropic.com/en/docs/about-claude/pricing>
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
            PricingSource::BundledListPrice => Some("2026-07-31"),
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

/// A model-name-substring → price table, matched case-insensitively, first match
/// wins (order entries specific → general).
#[derive(Clone, Debug)]
pub struct PricingTable {
    entries: Vec<(String, ModelPrice, PricingSource)>,
}

impl PricingTable {
    /// Build a table from `(substring, price)` pairs (already ordered specific→general).
    pub fn new(entries: Vec<(String, ModelPrice)>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|(key, price)| (key.to_ascii_lowercase(), price, PricingSource::CustomStatic))
                .collect(),
        }
    }

    fn bundled(entries: Vec<(String, ModelPrice)>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|(key, price)| {
                    (
                        key.to_ascii_lowercase(),
                        price,
                        PricingSource::BundledListPrice,
                    )
                })
                .collect(),
        }
    }

    /// Look up the price whose key is a case-insensitive substring of `model`.
    pub fn price_for(&self, model: &str) -> Option<ModelPrice> {
        let m = model.to_ascii_lowercase();
        self.entries
            .iter()
            .find(|(key, _, _)| m.contains(key.as_str()))
            .map(|(_, price, _)| *price)
    }

    /// Estimate one turn's API-list-price cost. Codex `output` already includes
    /// its reasoning-token breakdown. `None` means the model is unpriced, never
    /// that it costs zero.
    pub fn estimate_for(
        &self,
        model: &str,
        input: u64,
        output: u64,
        cache_create: u64,
        cache_read: u64,
    ) -> Option<CostEstimate> {
        let m = model.to_ascii_lowercase();
        let (_, p, source) = self
            .entries
            .iter()
            .find(|(key, _, _)| m.contains(key.as_str()))?;
        let usd = (input as f64 * p.input
            + output as f64 * p.output
            + cache_create as f64 * p.cache_write
            + cache_read as f64 * p.cache_read)
            / 1_000_000.0;
        Some(CostEstimate {
            usd,
            source: *source,
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
    /// Seeded from current Anthropic + OpenAI list prices (standard tier). Keys are
    /// lowercase substrings matched against the transcript `model` field.
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
        // Exact supported model IDs (plus their dated transcript suffixes) only.
        // A broad family key would make a new model appear priced using an older
        // sibling's rate, which is less honest than reporting it as unpriced.
        Self::bundled(vec![
            // --- Anthropic (Claude) ---
            ("claude-opus-4-8".into(), m(5.0, 25.0, 6.25, 0.50)),
            ("claude-opus-4-7".into(), m(5.0, 25.0, 6.25, 0.50)),
            ("claude-opus-4-6".into(), m(5.0, 25.0, 6.25, 0.50)),
            ("claude-opus-4-5".into(), m(5.0, 25.0, 6.25, 0.50)),
            ("claude-sonnet-4-6".into(), m(3.0, 15.0, 3.75, 0.30)),
            ("claude-haiku-4-5".into(), m(1.0, 5.0, 1.25, 0.10)),
            // --- OpenAI (Codex / GPT-5 family) ---
            // Codex rollouts currently report cache reads but no cache writes.
            // Keep the official 1.25x write rates nonetheless so a future
            // explicit write counter cannot silently understate cost.
            ("gpt-5.6-sol".into(), m(5.0, 30.0, 6.25, 0.50)),
            ("gpt-5.6-terra".into(), m(2.50, 15.0, 3.125, 0.25)),
            ("gpt-5.6-luna".into(), m(1.0, 6.0, 1.25, 0.10)),
            ("gpt-5.1-codex".into(), m(1.25, 10.0, 1.25, 0.125)),
            ("gpt-5-codex".into(), m(1.25, 10.0, 1.25, 0.125)),
            // GPT-5.5 is intentionally unpriced until this table can represent
            // its >272K-input 2x-input / 1.5x-output long-context surcharge.
        ])
    }
}

#[cfg(test)]
#[path = "pricing_tests.rs"]
mod tests;
