//! This module contains all code relevant to Agent Predict within Zaplex.
//!
//! Agent Predict attempts to predict the next action the user will take in Zaplex.

pub(crate) mod generate_ai_input_suggestions;
pub(crate) mod generate_am_query_suggestions;
pub mod next_command_model;
// Zaplex(Wave 3-2): `predict_am_queries` API module physically deleted — original `ServerApi::predict_am_queries`
// and all 0 external consumers already removed; FeatureFlag::PredictAMQueries / `predict_am_queries_future_handle`
// in terminal/input.rs kept only as control switch/handle placeholder, no longer needs this module.
pub mod prompt_suggestions;
