//! How a plugin's long work reports to the file operation it runs in.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::fs::engine::Worker;

/// What a scan or a removal tells the job it runs in: pause and cancel points, and progress.
pub trait Progress: Sync {
    fn checkpoint(&self) -> io::Result<()>;
    fn current(&self, name: &str);
    /// A new stage: how many files and bytes it will go through; counting starts again.
    fn plan(&self, files: u64, bytes: u64);
    fn bytes(&self, n: u64);
    fn file_done(&self);
}

/// The progress of a file operation, stopped early when the window that asked for it was closed.
pub struct OnWorker<'a> {
    pub worker: &'a Worker,
    pub cancel: &'a AtomicBool,
}

impl Progress for OnWorker<'_> {
    fn checkpoint(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        self.worker.shared.checkpoint()
    }

    fn current(&self, name: &str) {
        self.worker.shared.set_current(name);
    }

    fn plan(&self, files: u64, bytes: u64) {
        let shared = &self.worker.shared;
        shared.files_done.store(0, Ordering::Relaxed);
        shared.bytes_done.store(0, Ordering::Relaxed);
        shared.files_total.store(files, Ordering::Relaxed);
        shared.bytes_total.store(bytes, Ordering::Relaxed);
    }

    fn bytes(&self, n: u64) {
        self.worker.add_bytes(n);
    }

    fn file_done(&self) {
        self.worker.file_done();
    }
}
