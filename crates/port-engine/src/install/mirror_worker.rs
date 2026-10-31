//! Bridge synchronous Portfile callbacks to a shared Tokio probe worker.
use super::mirror_times::MirrorTimings;
use anyhow::{Context, Result, ensure};
use futures_util::{StreamExt, stream};
use std::sync::{Arc, mpsc};

enum Request {
    Start(Vec<String>),
    Wait(String, mpsc::Sender<Result<()>>),
    Shutdown,
}
struct Inner {
    sender: mpsc::Sender<Request>,
    thread: Option<std::thread::JoinHandle<()>>,
    timings: MirrorTimings,
}
impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.sender.send(Request::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[derive(Clone)]
pub struct MirrorWorker(Arc<Inner>);
impl MirrorWorker {
    pub fn new(timings: MirrorTimings, jobs: usize) -> Result<Self> {
        ensure!(
            (1..=128).contains(&jobs),
            "probe jobs must be between 1 and 128"
        );
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let (sender, receiver) = mpsc::channel();
        let cache = timings.clone();
        let thread = std::thread::Builder::new()
            .name("port-mirror-probes".into())
            .spawn(move || {
                let permits = Arc::new(tokio::sync::Semaphore::new(jobs));
                while let Ok(request) = receiver.recv() {
                    match request {
                        Request::Start(urls) => {
                            let cache = cache.clone();
                            let permits = permits.clone();
                            runtime.spawn(async move {
                                stream::iter(urls.into_iter().map(|url| {
                                    let cache = cache.clone();
                                    let permits = permits.clone();
                                    async move {
                                        let _permit =
                                            permits.acquire().await.expect("live probe semaphore");
                                        // Input was validated before queuing; transport failures
                                        // become cached penalties within probe_urls.
                                        let _ = cache.probe_urls(&[url], 1).await;
                                    }
                                }))
                                .buffer_unordered(jobs)
                                .collect::<Vec<_>>()
                                .await;
                            });
                        }
                        Request::Wait(url, reply) => {
                            let cache = cache.clone();
                            runtime.spawn(async move {
                                let _ = reply.send(cache.wait_for_url(&url).await);
                            });
                        }
                        Request::Shutdown => break,
                    }
                }
                // Cancel async work without blocking teardown on system DNS threads.
                runtime.shutdown_background();
            })
            .context("cannot start mirror worker")?;
        Ok(Self(Arc::new(Inner {
            sender,
            thread: Some(thread),
            timings,
        })))
    }
    pub fn timings(&self) -> MirrorTimings {
        self.0.timings.clone()
    }
    pub fn start(&self, urls: Vec<String>) -> Result<()> {
        for url in &urls {
            super::mirror_times::parts(url)?;
        }
        self.0
            .sender
            .send(Request::Start(urls))
            .context("mirror worker stopped")
    }
    /// Blocks only the calling Portfile worker; network tasks run independently.
    pub fn wait(&self, url: &str) -> Result<()> {
        super::mirror_times::parts(url)?;
        let (sender, receiver) = mpsc::channel();
        self.0
            .sender
            .send(Request::Wait(url.into(), sender))
            .context("mirror worker stopped")?;
        receiver
            .recv()
            .context("mirror worker stopped while waiting")?
    }
}
