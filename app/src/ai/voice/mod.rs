//! This module contains all code relevant to Voice within Zap.
//!
//! Voice is used for voice input within Zap.

// Zap Wave 6-1: `pub(crate) mod transcribe` to be deleted physically together with `ServerApi::transcribe`.
// The atomic module `transcribe/api/{request,response}` is wire type only for the deleted cloud
// `/ai/transcribe` endpoint. Local voice uses `voice/transcriber.rs::Transcriber` trait + `TranscribeError`.
