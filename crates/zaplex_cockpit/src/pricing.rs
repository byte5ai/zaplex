//! Per-model pricing table for deriving cost from token counts.
//!
//! **Approximation, by design.** LLM prices drift; this table is centralized and
//! overridable, and unknown models fall back to a *logged* zero (never silently
//! mispriced). Rates are USD per 1,000,000 tokens. Verify/refresh against current
//! Anthropic + OpenAI pricing when models change (see the Increment 1 design doc §5).

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

/// A model-name-substring → price table, matched case-insensitively, first match
/// wins (order entries specific → general).
#[derive(Clone, Debug)]
pub struct PricingTable {
    entries: Vec<(String, ModelPrice)>,
}

impl PricingTable {
    /// Build a table from `(substring, price)` pairs (already ordered specific→general).
    pub fn new(entries: Vec<(String, ModelPrice)>) -> Self {
        Self { entries }
    }

    /// Look up the price whose key is a case-insensitive substring of `model`.
    pub fn price_for(&self, model: &str) -> Option<ModelPrice> {
        let m = model.to_ascii_lowercase();
        self.entries
            .iter()
            .find(|(key, _)| m.contains(key.as_str()))
            .map(|(_, price)| *price)
    }

    /// Cost in USD for one turn's tokens. Reasoning tokens (Codex) bill as output.
    /// Unknown models cost 0 and are logged at debug level (never silently mispriced).
    pub fn cost_for(
        &self,
        model: &str,
        input: u64,
        output: u64,
        cache_create: u64,
        cache_read: u64,
        reasoning: u64,
    ) -> f64 {
        let Some(p) = self.price_for(model) else {
            log::debug!("zaplex_cockpit: no pricing for model {model:?}; costing as $0");
            return 0.0;
        };
        (input as f64 * p.input
            + (output + reasoning) as f64 * p.output
            + cache_create as f64 * p.cache_write
            + cache_read as f64 * p.cache_read)
            / 1_000_000.0
    }
}

impl Default for PricingTable {
    /// Seeded from current Anthropic + OpenAI list prices (standard tier). Keys are
    /// lowercase substrings matched against the transcript `model` field.
    ///
    /// TODO(pricing): refresh on model launches; verify OpenAI/Codex rates (their
    /// transcripts + `~/.codex/models_cache.json` carry no price fields).
    fn default() -> Self {
        let m = |input, output, cache_write, cache_read| ModelPrice {
            input,
            output,
            cache_write,
            cache_read,
        };
        // Order matters: more specific keys first.
        Self::new(vec![
            // --- Anthropic (Claude) ---
            ("opus".into(), m(15.0, 75.0, 18.75, 1.50)),
            ("sonnet".into(), m(3.0, 15.0, 3.75, 0.30)),
            ("haiku".into(), m(1.0, 5.0, 1.25, 0.10)),
            // --- OpenAI (Codex / GPT-5 family) ---
            // Codex transcripts report `cached_input_tokens` (→ cache_read); no
            // separate cache-write concept, so cache_write mirrors input.
            ("gpt-5-codex".into(), m(1.25, 10.0, 1.25, 0.125)),
            ("codex".into(), m(1.25, 10.0, 1.25, 0.125)),
            ("gpt-5".into(), m(1.25, 10.0, 1.25, 0.125)),
            ("o4".into(), m(1.10, 4.40, 1.10, 0.275)),
            ("o3".into(), m(2.0, 8.0, 2.0, 0.50)),
        ])
    }
}

#[cfg(test)]
#[path = "pricing_tests.rs"]
mod tests;
