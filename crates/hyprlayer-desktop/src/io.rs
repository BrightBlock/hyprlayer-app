//! The tokio runtime on its own thread. GPUI's executor drives the UI; every socket, timer and
//! subprocess runs here and reports back over a channel or a oneshot.

use std::future::Future;

use gpui::Global;
use tokio::runtime::Handle;
use tokio::sync::oneshot;

#[derive(Clone)]
pub struct Io {
    handle: Handle,
}

impl Global for Io {}

impl Io {
    pub fn start() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("hyprlayer-io".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                let _ = tx.send(rt.handle().clone());
                rt.block_on(std::future::pending::<()>());
            })
            .expect("spawn the io thread");
        Self {
            handle: rx.recv().expect("the io thread reports its handle"),
        }
    }

    pub fn handle(&self) -> &Handle {
        &self.handle
    }

    /// Runs `fut` on the io thread; the receiver resolves on any executor.
    pub fn spawn<T: Send + 'static>(
        &self,
        fut: impl Future<Output = T> + Send + 'static,
    ) -> oneshot::Receiver<T> {
        let (tx, rx) = oneshot::channel();
        self.handle.spawn(async move {
            let _ = tx.send(fut.await);
        });
        rx
    }
}
