use std::{
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
};

use async_channel::Sender;
use async_fs::OpenOptions;
use futures::{
    channel::oneshot,
    future::{BoxFuture, Shared},
    AsyncWriteExt as _, FutureExt as _,
};
use warpui::r#async::executor::{Background, BackgroundTask};

pub mod manager;

#[cfg(test)]
pub(crate) type AfterReceiveHook = Arc<dyn Fn(&str) + Send + Sync>;
#[cfg(not(test))]
pub(crate) type AfterReceiveHook = ();

enum LogCommand {
    Begin,
    Line(String),
    Finish(oneshot::Sender<()>),
}

/// Serializes every generation that writes to one log path.
pub(crate) struct LogWorker {
    log_tx: Sender<LogCommand>,
    _logging_task: BackgroundTask,
}

impl LogWorker {
    pub(crate) fn new(
        log_path: PathBuf,
        executor: Arc<Background>,
        after_receive: Option<AfterReceiveHook>,
    ) -> Arc<Self> {
        let (log_tx, log_rx) = async_channel::unbounded::<LogCommand>();

        if let Some(directory) = log_path.parent() {
            let _ = std::fs::create_dir_all(directory);
        }

        #[cfg(not(test))]
        let _ = after_receive;
        let logging_task = executor.spawn(async move {
            let mut log_file: Option<async_fs::File> = None;
            while let Ok(command) = log_rx.recv().await {
                match command {
                    LogCommand::Begin => {
                        if let Some(mut previous_file) = log_file.take() {
                            let _ = previous_file.flush().await;
                        }
                        log_file = match OpenOptions::new()
                            .write(true)
                            .create(true)
                            .truncate(true)
                            .open(&log_path)
                            .await
                        {
                            Ok(log_file) => Some(log_file),
                            Err(error) => {
                                log::warn!(
                                    "Could not open file for logging: {:?}. {:?}",
                                    &log_path,
                                    error
                                );
                                None
                            }
                        };
                    }
                    LogCommand::Line(log_line) => {
                        #[cfg(test)]
                        if let Some(after_receive) = &after_receive {
                            after_receive(&log_line);
                        }
                        let Some(log_file) = &mut log_file else {
                            continue;
                        };
                        let _ = log_file
                            .write_all(
                                format!(
                                    "{} | {}\n",
                                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                                    log_line
                                )
                                .as_bytes(),
                            )
                            .await;
                        // Flush after each line to ensure logs are visible immediately.
                        let _ = log_file.flush().await;
                    }
                    LogCommand::Finish(completion) => {
                        if let Some(mut finished_file) = log_file.take() {
                            let _ = finished_file.flush().await;
                        }
                        let _ = completion.send(());
                    }
                }
            }

            if let Some(mut log_file) = log_file {
                let _ = log_file.flush().await;
            }
        });

        Arc::new(Self {
            log_tx,
            _logging_task: logging_task,
        })
    }
}

struct WriterState {
    finish_tx: Option<oneshot::Sender<()>>,
}

/// Shared state for one [`SimpleLogger`] generation.
pub(crate) struct LogFileWriter {
    worker: Arc<LogWorker>,
    state: Mutex<WriterState>,
    completion: Shared<BoxFuture<'static, ()>>,
}

impl LogFileWriter {
    fn new(worker: Arc<LogWorker>) -> Arc<Self> {
        let (finish_tx, finish_rx) = oneshot::channel();
        let completion = finish_rx.map(|_| ()).boxed().shared();
        let _ = worker.log_tx.try_send(LogCommand::Begin);
        Arc::new(Self {
            worker,
            state: Mutex::new(WriterState {
                finish_tx: Some(finish_tx),
            }),
            completion,
        })
    }

    fn log(&self, message: String) {
        let state = self.state.lock().unwrap();
        if state.finish_tx.is_some() {
            let _ = self.worker.log_tx.try_send(LogCommand::Line(message));
        }
    }

    fn finish(&self) {
        let Some(completion) = self.state.lock().unwrap().finish_tx.take() else {
            return;
        };
        let _ = self.worker.log_tx.try_send(LogCommand::Finish(completion));
    }

    /// Returns true after this generation stopped accepting new lines.
    pub(crate) fn is_closed(&self) -> bool {
        self.state.lock().unwrap().finish_tx.is_none()
    }
}

impl Drop for LogFileWriter {
    fn drop(&mut self) {
        self.finish();
    }
}

/// A simple file-based logger for server stderr output.
/// Writes timestamped log entries to a file asynchronously.
#[derive(Clone)]
pub struct SimpleLogger {
    writer: Arc<LogFileWriter>,
}

impl SimpleLogger {
    pub(crate) fn new_generation(worker: Arc<LogWorker>) -> Self {
        Self {
            writer: LogFileWriter::new(worker),
        }
    }

    /// Log a message to the file.
    pub fn log(&self, message: String) {
        self.writer.log(message);
    }

    /// Stop accepting messages and enqueue a final flush for this generation.
    pub fn close(&self) {
        self.writer.finish();
    }

    /// Wait until every accepted line in this generation has been flushed.
    pub async fn wait_closed(&self) {
        self.writer.completion.clone().await;
    }

    /// Returns a weak reference to the shared writer, used by [`manager::LogManager`]
    /// to track liveness without preventing shutdown.
    pub(crate) fn downgrade(&self) -> Weak<LogFileWriter> {
        Arc::downgrade(&self.writer)
    }
}
