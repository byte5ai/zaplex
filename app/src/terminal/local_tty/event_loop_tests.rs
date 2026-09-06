use std::{
    borrow::Cow,
    collections::VecDeque,
    io::{self, Write},
};

use super::{write_queued, State};

enum WriteStep {
    Partial(usize),
    WouldBlock,
    Remaining,
}

struct ScriptedWriter {
    steps: VecDeque<WriteStep>,
    bytes: Vec<u8>,
}

impl ScriptedWriter {
    fn new(steps: impl IntoIterator<Item = WriteStep>) -> Self {
        Self {
            steps: steps.into_iter().collect(),
            bytes: Vec::new(),
        }
    }
}

impl Write for ScriptedWriter {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        match self.steps.pop_front().unwrap_or(WriteStep::Remaining) {
            WriteStep::Partial(limit) => {
                let written = limit.min(input.len());
                self.bytes.extend_from_slice(&input[..written]);
                Ok(written)
            }
            WriteStep::WouldBlock => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            WriteStep::Remaining => {
                self.bytes.extend_from_slice(input);
                Ok(input.len())
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn terminal_responses_survive_partial_and_blocked_writes_in_queue_order() {
    let mut state = State::default();
    state.write_list.push_back(Cow::Borrowed(b"user-input:"));
    state.queue_terminal_response_sequences(b"response-one:".to_vec());
    state.queue_terminal_response_sequences(Vec::new());
    state.queue_terminal_response_sequences(b"response-two".to_vec());

    let mut writer = ScriptedWriter::new([
        WriteStep::Partial(3),
        WriteStep::WouldBlock,
        WriteStep::Partial(2),
        WriteStep::WouldBlock,
        WriteStep::Remaining,
    ]);

    let mut can_write = true;
    write_queued(&mut state, &mut writer, &mut can_write).unwrap();
    assert!(!can_write);
    assert!(state.needs_write());

    can_write = true;
    write_queued(&mut state, &mut writer, &mut can_write).unwrap();
    assert!(!can_write);
    assert!(state.needs_write());

    can_write = true;
    write_queued(&mut state, &mut writer, &mut can_write).unwrap();

    assert_eq!(writer.bytes, b"user-input:response-one:response-two");
    assert!(!state.needs_write());
}
