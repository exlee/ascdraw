use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use crate::app::{AppEvent, BackgroundEvent};

pub struct BackgroundWorker {
    sender: Sender<Task>,
    thread: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct BackgroundSender(Sender<Task>);

enum Task {
    #[cfg(debug_assertions)]
    DebugOutput(String),
    WriteAutosave {
        window_id: WindowId,
        path: PathBuf,
        contents: String,
    },
    Flush(Sender<()>),
    Stop,
}

impl BackgroundWorker {
    pub fn start(proxy: EventLoopProxy<AppEvent>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("ascdraw-background".into())
            .spawn(move || {
                while let Ok(task) = receiver.recv() {
                    match task {
                        #[cfg(debug_assertions)]
                        Task::DebugOutput(message) => println!("{message}"),
                        Task::WriteAutosave {
                            window_id,
                            path,
                            contents,
                        } => {
                            let result = path
                                .parent()
                                .filter(|parent| !parent.as_os_str().is_empty())
                                .map(std::fs::create_dir_all)
                                .transpose()
                                .with_context(|| {
                                    format!("failed to create parent for {}", path.display())
                                })
                                .and_then(|_| {
                                    std::fs::write(&path, contents).with_context(|| {
                                        format!("failed to write {}", path.display())
                                    })
                                })
                                .map_err(|error| format!("{error:#}"));
                            let _ = proxy.send_event(AppEvent::Background(
                                BackgroundEvent::AutosaveFinished { window_id, result },
                            ));
                        }
                        Task::Flush(done) => {
                            let _ = done.send(());
                        }
                        Task::Stop => break,
                    }
                }
            })
            .expect("failed to start background worker");
        Self {
            sender,
            thread: Some(thread),
        }
    }

    pub fn sender(&self) -> BackgroundSender {
        BackgroundSender(self.sender.clone())
    }

    pub fn flush(&self) {
        let (done, received) = mpsc::channel();
        if self.sender.send(Task::Flush(done)).is_ok() {
            let _ = received.recv();
        }
    }
}

impl Drop for BackgroundWorker {
    fn drop(&mut self) {
        let _ = self.sender.send(Task::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl BackgroundSender {
    #[cfg(debug_assertions)]
    pub fn debug_output(&self, message: String) {
        let _ = self.0.send(Task::DebugOutput(message));
    }

    pub fn flush(&self) {
        let (done, received) = mpsc::channel();
        if self.0.send(Task::Flush(done)).is_ok() {
            let _ = received.recv();
        }
    }

    pub fn write_autosave(
        &self,
        window_id: WindowId,
        path: PathBuf,
        contents: String,
    ) -> Result<()> {
        self.0
            .send(Task::WriteAutosave {
                window_id,
                path,
                contents,
            })
            .map_err(|_| anyhow::anyhow!("background worker stopped"))
    }
}
