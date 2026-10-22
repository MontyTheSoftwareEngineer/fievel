use std::{
    io,
    sync::mpsc::{sync_channel, SyncSender, TrySendError},
    thread::{self, JoinHandle},
};

pub struct Locator {
    sender: Option<SyncSender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl Locator {
    pub fn new() -> io::Result<Self> {
        let (sender, receiver) = sync_channel(1);
        let worker = thread::Builder::new()
            .name("fievel-locator".into())
            .spawn(move || {
                while receiver.recv().is_ok() {
                    if let Err(error) = crate::hints::locate_cursor() {
                        eprintln!("fievel: cursor locator unavailable: {error}");
                    }
                }
            })?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    pub fn trigger(&self) -> io::Result<()> {
        let Some(sender) = &self.sender else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "cursor locator is shutting down",
            ));
        };
        match sender.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) => Ok(()),
            Err(TrySendError::Disconnected(())) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "cursor locator worker stopped",
            )),
        }
    }
}

impl Drop for Locator {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                eprintln!("fievel: cursor locator worker panicked");
            }
        }
    }
}
