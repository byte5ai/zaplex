use std::{io::Cursor, time::Duration};

use base64::Engine;
use cpal::{
    traits::{DeviceTrait, HostTrait},
    Sample, StreamConfig,
};
use futures::channel::oneshot;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use thiserror::Error;

use warpui::event::KeyState;
use warpui::{platform::MicrophoneAccessState, Entity, ModelContext, SingletonEntity};

const DEFAULT_CHUNK_SIZE: u32 = 512;
// We only support mono for now.
const NUM_CHANNELS: u16 = 1;
// Voice input is typically sampled at 16000Hz (and required by Wispr)
const TARGET_SAMPLE_RATE: f32 = 16000.0;
const STREAM_TIMEOUT: Duration = Duration::from_secs(60 * 6);

fn make_resampler(
    sample_rate: f64,
    chunk_size: usize,
) -> Result<SincFixedIn<f32>, rubato::ResamplerConstructionError> {
    SincFixedIn::new(
        TARGET_SAMPLE_RATE as f64 / sample_rate,
        2.0,
        SincInterpolationParameters {
            interpolation: SincInterpolationType::Linear,
            window: WindowFunction::Hann,
            sinc_len: chunk_size,
            f_cutoff: 0.95,
            oversampling_factor: 1,
        },
        chunk_size,
        NUM_CHANNELS as usize,
    )
}

struct AudioAccumulator {
    resampler: SincFixedIn<f32>,
    pending: Vec<f32>,
    output: Vec<f32>,
}

impl AudioAccumulator {
    fn new(resampler: SincFixedIn<f32>) -> Self {
        Self {
            resampler,
            pending: Vec::new(),
            output: Vec::new(),
        }
    }

    fn push(&mut self, frame: Vec<f32>) -> anyhow::Result<()> {
        self.pending.extend(frame);
        self.process_complete_chunks()
    }

    fn process_complete_chunks(&mut self) -> anyhow::Result<()> {
        loop {
            let input_frames = self.resampler.input_frames_next();
            if self.pending.len() < input_frames {
                return Ok(());
            }
            let chunk = self.pending.drain(..input_frames).collect::<Vec<_>>();
            self.output
                .extend(self.resampler.process(&[chunk], None)?[0].iter().copied());
        }
    }

    fn finish(mut self) -> anyhow::Result<Vec<f32>> {
        self.process_complete_chunks()?;
        if !self.pending.is_empty() {
            let pending = std::mem::take(&mut self.pending);
            self.output.extend(
                self.resampler.process_partial(Some(&[pending]), None)?[0]
                    .iter()
                    .copied(),
            );
        }
        Ok(self.output)
    }
}

async fn run_audio_pipeline(
    audio_frame_rx: async_channel::Receiver<Vec<f32>>,
    resampler: SincFixedIn<f32>,
) -> anyhow::Result<String> {
    encode_wav(collect_resampled_audio(audio_frame_rx, resampler).await?)
}

async fn collect_resampled_audio(
    audio_frame_rx: async_channel::Receiver<Vec<f32>>,
    resampler: SincFixedIn<f32>,
) -> anyhow::Result<Vec<f32>> {
    let mut accumulator = AudioAccumulator::new(resampler);
    while let Ok(frame) = audio_frame_rx.recv().await {
        accumulator.push(frame)?;
    }
    accumulator.finish()
}

fn encode_wav(resampled: Vec<f32>) -> anyhow::Result<String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_SAMPLE_RATE as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut wav_cursor = Cursor::new(Vec::with_capacity(resampled.len() * 2));
    let mut wav_writer = hound::WavWriter::new(&mut wav_cursor, spec)?;
    for sample in resampled {
        wav_writer.write_sample(sample.to_sample::<i16>())?;
    }
    wav_writer.finalize()?;

    Ok(base64::engine::general_purpose::STANDARD.encode(wav_cursor.into_inner()))
}

pub struct VoiceInput {
    state: VoiceInputState,
    next_session_id: u64,
    current_session_id: Option<VoiceSessionId>,
    pub should_suppress_new_feature_popup: bool,
    voice_session_start: Option<instant::Instant>,
}

#[derive(Default)]
pub enum VoiceInputState {
    #[default]
    Idle,

    Listening {
        session_id: VoiceSessionId,
        stream: cpal::Stream,
        audio_frame_tx: async_channel::Sender<Vec<f32>>,
        enabled_from: VoiceInputToggledFrom,
        result_tx: Option<oneshot::Sender<VoiceSessionResult>>,
    },

    Transcribing {
        session_id: VoiceSessionId,
        result_tx: Option<oneshot::Sender<VoiceSessionResult>>,
        session_duration_ms: Option<u64>,
    },
}

pub type VoiceSessionId = u64;

#[derive(Debug, Clone)]
pub enum VoiceInputToggledFrom {
    Button,
    Key { state: KeyState },
}

/// Result of a voice recording session.
#[derive(Debug)]
pub enum VoiceSessionResult {
    /// Recording completed successfully with audio data.
    Audio {
        session_id: VoiceSessionId,
        wav_base64: String,
        session_duration_ms: u64,
    },
    /// Recording was aborted without producing audio.
    Aborted {
        session_id: VoiceSessionId,
        session_duration_ms: Option<u64>,
    },
}

impl VoiceSessionResult {
    pub fn session_id(&self) -> VoiceSessionId {
        match self {
            VoiceSessionResult::Audio { session_id, .. }
            | VoiceSessionResult::Aborted { session_id, .. } => *session_id,
        }
    }
}

/// Represents an active voice recording session.
///
/// The caller owns this session and can await the result directly.
/// Dropping the session will prevent the caller from receiving the result,
/// but does not itself stop or abort the underlying recording.
pub struct VoiceSession {
    session_id: VoiceSessionId,
    result_rx: oneshot::Receiver<VoiceSessionResult>,
}

impl VoiceSession {
    pub fn id(&self) -> VoiceSessionId {
        self.session_id
    }

    /// Awaits the result of the voice recording session.
    ///
    /// Returns `VoiceSessionResult::Audio` if recording completed successfully,
    /// or `VoiceSessionResult::Aborted` if the recording was cancelled.
    pub async fn await_result(self) -> VoiceSessionResult {
        match self.result_rx.await {
            Ok(result) => result,
            // Channel closed without sending - treat as aborted
            Err(_) => VoiceSessionResult::Aborted {
                session_id: self.session_id,
                session_duration_ms: None,
            },
        }
    }
}

/// Error returned when starting voice input fails.
#[derive(Debug, Error)]
pub enum StartListeningError {
    /// Voice input is already running.
    #[error("Voice input is already running")]
    AlreadyRunning,
    /// Microphone access was denied or restricted.
    #[error("Microphone access denied")]
    AccessDenied,
    /// Other error (e.g., no input device, failed to create stream).
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl VoiceInput {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            state: VoiceInputState::Idle,
            next_session_id: 0,
            current_session_id: None,
            should_suppress_new_feature_popup: false,
            voice_session_start: None,
        }
    }

    pub fn is_listening(&self) -> bool {
        matches!(self.state, VoiceInputState::Listening { .. })
    }

    pub fn is_transcribing(&self) -> bool {
        matches!(self.state, VoiceInputState::Transcribing { .. })
    }

    /// Returns true if voice is currently recording or transcribing.
    pub fn is_active(&self) -> bool {
        self.is_listening() || self.is_transcribing()
    }

    pub fn state(&self) -> &VoiceInputState {
        &self.state
    }

    pub fn is_current_session(&self, session_id: VoiceSessionId) -> bool {
        self.current_session_id == Some(session_id)
    }

    /// Starts listening for voice input and returns a session that will receive the result.
    ///
    /// The returned `VoiceSession` can be awaited to receive the audio data when recording
    /// stops. Dropping the session will abort the recording.
    pub fn start_listening(
        &mut self,
        ctx: &mut ModelContext<Self>,
        source: VoiceInputToggledFrom,
    ) -> Result<VoiceSession, StartListeningError> {
        if self.is_active() {
            log::debug!("Voice input is already active, not starting again");
            return Err(StartListeningError::AlreadyRunning);
        }

        log::debug!("Enabling voice input");
        let (audio_frame_tx, audio_frame_rx) = async_channel::unbounded();

        let host = cpal::default_host();
        let Some(input_device) = host.default_input_device() else {
            return Err(anyhow::anyhow!("No default input device found").into());
        };

        let config = input_device.default_input_config().map_err(|e| {
            log::error!("Failed to get default input config: {e}");
            StartListeningError::Other(anyhow::anyhow!("Failed to get default input config: {}", e))
        })?;

        // Kind of annoying that we need to check this here, but cpal will actually still create an audio
        // stream of empty frames even if the user denies access on MacOS.
        if matches!(
            ctx.microphone_access_state(),
            MicrophoneAccessState::Denied | MicrophoneAccessState::Restricted
        ) {
            return Err(StartListeningError::AccessDenied);
        }

        // Try to use our default chunk size, but clamped to the supported range.
        let buffer_size = match config.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => DEFAULT_CHUNK_SIZE.clamp(*min, *max),
            cpal::SupportedBufferSize::Unknown => DEFAULT_CHUNK_SIZE,
        };
        let sample_rate = config.sample_rate() as f64;
        let num_channels = config.channels();
        let stream_config: StreamConfig = config.into();

        // Set the buffer size to a fixed size so it's easier to resample.
        let stream_config = StreamConfig {
            buffer_size: cpal::BufferSize::Fixed(buffer_size),
            ..stream_config
        };

        log::debug!("Stream config: {stream_config:?}");

        // Set up the resampler to resample the audio to 16000Hz, which is typical for voice input.
        let resampler = make_resampler(sample_rate, buffer_size as usize).map_err(|e| {
            StartListeningError::Other(anyhow::anyhow!("Resampler construction failed: {e}"))
        })?;

        let callback_audio_frame_tx = audio_frame_tx.clone();
        let stream = input_device
            .build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let is_empty = data.iter().all(|&x| x == 0.0);
                    log::debug!("Sending audio frame to resampling thread. is_empty: {is_empty}");

                    // Average the channels into mono at this point.
                    let mono_samples: Vec<f32> = data
                        .chunks_exact(num_channels as usize)
                        .map(|frame| frame.iter().sum::<f32>() / num_channels as f32)
                        .collect();

                    // This is blocking, but we aren't on the main thread.
                    let _ = warpui::r#async::block_on(callback_audio_frame_tx.send(mono_samples));
                },
                |err| {
                    log::error!("Error in voice input stream: {err}");
                },
                Some(STREAM_TIMEOUT),
            )
            .map_err(|e| {
                StartListeningError::Other(anyhow::anyhow!("Failed to build input stream: {e}"))
            })?;
        cpal::traits::StreamTrait::play(&stream).map_err(|e| {
            StartListeningError::Other(anyhow::anyhow!("Failed to play stream: {e}"))
        })?;

        log::debug!("Starting voice input stream with chunk size {buffer_size}");

        self.next_session_id = self.next_session_id.checked_add(1).ok_or_else(|| {
            StartListeningError::Other(anyhow::anyhow!("Voice session ID space exhausted"))
        })?;
        let session_id = self.next_session_id;
        self.current_session_id = Some(session_id);

        // Track voice session start time
        self.voice_session_start = Some(instant::Instant::now());

        // Create channel for returning result to caller
        let (result_tx, result_rx) = oneshot::channel();

        self.state = VoiceInputState::Listening {
            session_id,
            audio_frame_tx,
            enabled_from: source,
            result_tx: Some(result_tx),
            // We need to keep the stream around to keep the audio flowing.
            stream,
        };

        ctx.spawn(
            run_audio_pipeline(audio_frame_rx, resampler),
            move |me, wav_result, _ctx| me.complete_audio_session(session_id, wav_result),
        );

        Ok(VoiceSession {
            session_id,
            result_rx,
        })
    }

    pub fn start_time(&self) -> Option<instant::Instant> {
        self.voice_session_start
    }

    pub fn set_transcribing_active(&mut self, session_id: VoiceSessionId, active: bool) {
        if !self.is_current_session(session_id) {
            return;
        }
        if active {
            self.state = VoiceInputState::Transcribing {
                session_id,
                result_tx: None,
                session_duration_ms: None,
            };
        } else {
            self.current_session_id = None;
            self.state = VoiceInputState::Idle;
        }
    }

    /// Stops listening and triggers WAV conversion. The result will be sent through
    /// the VoiceSession returned from start_listening.
    pub fn stop_listening(&mut self, ctx: &mut ModelContext<Self>) -> Result<(), anyhow::Error> {
        if let VoiceInputState::Listening { stream, .. } = &mut self.state {
            cpal::traits::StreamTrait::pause(stream)?;

            // Calculate session duration before conversion
            let session_duration_ms = self
                .voice_session_start
                .take()
                .map(|start| start.elapsed().as_millis() as u64)
                .unwrap_or(0);

            log::debug!("Disabling voice input and converting to WAV");

            let old_state = std::mem::take(&mut self.state);
            let VoiceInputState::Listening {
                session_id,
                stream,
                audio_frame_tx,
                result_tx,
                ..
            } = old_state
            else {
                unreachable!("voice state changed while stopping");
            };
            drop(stream);
            drop(audio_frame_tx);
            self.state = VoiceInputState::Transcribing {
                session_id,
                result_tx,
                session_duration_ms: Some(session_duration_ms),
            };
            ctx.notify();
        } else {
            log::debug!("Not currently listening for voice input");
        }
        Ok(())
    }

    /// Stops listening without forwarding audio for processing.
    /// The VoiceSession will receive VoiceSessionResult::Aborted.
    pub fn abort_listening(&mut self) {
        log::debug!("Aborting voice input");

        // Calculate session duration before aborting
        let session_duration_ms = self
            .voice_session_start
            .take()
            .map(|start| start.elapsed().as_millis() as u64);

        // Take ownership and send abort result through channel.
        let old_state = std::mem::take(&mut self.state);
        let (session_id, result_tx, duration) = match old_state {
            VoiceInputState::Listening {
                session_id,
                result_tx,
                ..
            } => (Some(session_id), result_tx, session_duration_ms),
            VoiceInputState::Transcribing {
                session_id,
                result_tx,
                session_duration_ms: conversion_duration,
            } => (Some(session_id), result_tx, conversion_duration),
            VoiceInputState::Idle => (None, None, session_duration_ms),
        };
        self.current_session_id = None;
        if let (Some(session_id), Some(tx)) = (session_id, result_tx) {
            let _ = tx.send(VoiceSessionResult::Aborted {
                session_id,
                session_duration_ms: duration,
            });
        }

        // Reset to Idle state
        self.state = VoiceInputState::Idle;
    }

    fn complete_audio_session(
        &mut self,
        session_id: VoiceSessionId,
        wav_result: anyhow::Result<String>,
    ) {
        if !self.is_current_session(session_id) {
            return;
        }
        let (active_session_id, result_tx, session_duration_ms) =
            match std::mem::take(&mut self.state) {
                VoiceInputState::Transcribing {
                    session_id,
                    result_tx,
                    session_duration_ms,
                } => (session_id, result_tx, session_duration_ms),
                state => {
                    self.state = state;
                    return;
                }
            };
        if active_session_id != session_id {
            self.state = VoiceInputState::Transcribing {
                session_id: active_session_id,
                result_tx,
                session_duration_ms,
            };
            return;
        }

        if let Some(tx) = result_tx {
            let duration = session_duration_ms.unwrap_or(0);
            let result = match wav_result {
                Ok(wav_base64) => VoiceSessionResult::Audio {
                    session_id,
                    wav_base64,
                    session_duration_ms: duration,
                },
                Err(error) => {
                    log::error!("Failed to convert to WAV: {error}");
                    VoiceSessionResult::Aborted {
                        session_id,
                        session_duration_ms,
                    }
                }
            };
            let _ = tx.send(result);
        }
        self.state = VoiceInputState::Idle;
    }
}

impl Entity for VoiceInput {
    type Event = ();
}

impl SingletonEntity for VoiceInput {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn samples(count: usize) -> Vec<f32> {
        (0..count)
            .map(|index| ((index as f32) * 0.013).sin() * 0.5)
            .collect()
    }

    fn reference_output(input: &[f32], chunk_size: usize) -> Vec<f32> {
        let mut resampler = make_resampler(48_000.0, chunk_size).unwrap();
        let complete_len = input.len() / chunk_size * chunk_size;
        let mut output = Vec::new();
        for chunk in input[..complete_len].chunks_exact(chunk_size) {
            output.extend(
                resampler.process(&[chunk], None).unwrap()[0]
                    .iter()
                    .copied(),
            );
        }
        if complete_len < input.len() {
            output.extend(
                resampler
                    .process_partial(Some(&[&input[complete_len..]]), None)
                    .unwrap()[0]
                    .iter()
                    .copied(),
            );
        }
        output
    }

    #[test]
    fn accumulator_is_invariant_to_callback_sizes() {
        let input = samples(1_504);
        let mut accumulator = AudioAccumulator::new(make_resampler(48_000.0, 512).unwrap());
        accumulator.push(input[..480].to_vec()).unwrap();
        accumulator.push(input[480..].to_vec()).unwrap();

        assert_eq!(accumulator.finish().unwrap(), reference_output(&input, 512));
    }

    #[test]
    fn worker_drains_queued_frames_before_finishing() {
        let input = samples(1_504);
        let (frame_tx, frame_rx) = async_channel::unbounded();
        frame_tx.try_send(input[..480].to_vec()).unwrap();
        frame_tx.try_send(input[480..].to_vec()).unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = warpui::r#async::block_on(collect_resampled_audio(
                frame_rx,
                make_resampler(48_000.0, 512).unwrap(),
            ));
            done_tx.send(result).unwrap();
        });

        assert!(matches!(done_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
        drop(frame_tx);
        let output = done_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        worker.join().unwrap();

        assert_eq!(output, reference_output(&input, 512));
    }

    #[test]
    fn cancelled_session_completion_does_not_replace_newer_session_state() {
        let (result_tx, result_rx) = oneshot::channel();
        let mut voice = VoiceInput {
            state: VoiceInputState::Transcribing {
                session_id: 1,
                result_tx: Some(result_tx),
                session_duration_ms: Some(10),
            },
            next_session_id: 1,
            current_session_id: Some(1),
            should_suppress_new_feature_popup: false,
            voice_session_start: None,
        };

        voice.abort_listening();
        let aborted = warpui::r#async::block_on(result_rx).unwrap();
        assert_eq!(aborted.session_id(), 1);

        voice.next_session_id = 2;
        voice.current_session_id = Some(2);
        voice.state = VoiceInputState::Transcribing {
            session_id: 2,
            result_tx: None,
            session_duration_ms: None,
        };

        voice.complete_audio_session(1, Ok(String::new()));

        assert!(matches!(
            voice.state,
            VoiceInputState::Transcribing { session_id: 2, .. }
        ));
        assert_eq!(voice.current_session_id, Some(2));
    }
}
