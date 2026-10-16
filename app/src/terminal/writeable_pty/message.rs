use crate::terminal::model::session::InBandCommandCancelledEvent;
use crate::terminal::SizeInfo;
use async_channel::Sender;
use std::borrow::Cow;
use std::fmt;

/// Messages that may be sent to the `EventLoop`.
#[derive(Debug)]
pub enum Message {
    /// Data that should be written to the PTY.
    Input(Cow<'static, [u8]>),

    /// An in-band command for a PTY whose event loop may still reject input
    /// (a daemon-hosted session).
    InBandCommand(InBandCommandInput),

    /// Indicates that the `EventLoop` should be shut down.
    Shutdown,

    /// Indicates that the child process has exited.
    ///
    /// Only used on Windows, as we need to pass this information to the
    /// event loop via the channel (and cannot use the child event token).
    #[cfg_attr(not(windows), allow(dead_code))]
    ChildExited,

    /// Instruction to resize the PTY.
    Resize(SizeInfo),
}

/// The bytes of an in-band command and how to settle it. The event loop marks
/// the active block as the command's in-band block only when it hands the
/// bytes to the PTY; if it rejects them, it cancels the command instead, so
/// neither a marked block nor a waiting executor is left behind.
pub struct InBandCommandInput {
    pub bytes: Cow<'static, [u8]>,
    command_id: String,
    mark_block: Box<dyn Fn() + Send + Sync + 'static>,
    cancel_tx: Sender<InBandCommandCancelledEvent>,
}

impl InBandCommandInput {
    pub fn new(
        bytes: Cow<'static, [u8]>,
        command_id: String,
        mark_block: Box<dyn Fn() + Send + Sync + 'static>,
        cancel_tx: Sender<InBandCommandCancelledEvent>,
    ) -> Self {
        Self {
            bytes,
            command_id,
            mark_block,
            cancel_tx,
        }
    }

    /// Settles a command whose bytes were handed to the PTY: marks the active
    /// block before the shell's echo of the command can be parsed.
    pub fn delivered(self) {
        (self.mark_block)();
    }

    /// Settles a command whose bytes will not reach the PTY, so its executor
    /// stops waiting for output.
    pub fn rejected(self) {
        if let Err(err) = self.cancel_tx.try_send(InBandCommandCancelledEvent {
            command_id: self.command_id,
        }) {
            log::warn!("Failed to cancel a rejected in-band command: {err}");
        }
    }
}

impl fmt::Debug for InBandCommandInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InBandCommandInput")
            .field("command_id", &self.command_id)
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}
