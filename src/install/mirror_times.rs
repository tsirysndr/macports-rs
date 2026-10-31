//! Shared mirror timing cache and MacPorts URL ordering policy.
use anyhow::{Context as _, Result, ensure};
use molt::glob::Glob;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

#[derive(Clone, Default)]
pub struct MirrorTimings(Arc<Mutex<Cache>>, Arc<tokio::sync::Notify>);
#[derive(Default)]
struct Cache {
    blacklist: Vec<Glob>,
    preferred: Vec<Glob>,
    times: BTreeMap<String, (f64, SystemTime)>,
    pending: BTreeSet<String>,
}
impl MirrorTimings {
    /// Parse configured Tcl lists without evaluating their contents.
    pub fn from_variables(variables: &BTreeMap<String, String>) -> Result<Self> {
        let patterns = |name: &str| {
            crate::tcl_list::parse(variables.get(name).map(String::as_str).unwrap_or(""))
                .with_context(|| format!("invalid {name} list"))
        };
        Ok(Self::new(
            &patterns("host_blacklist")?,
            &patterns("preferred_hosts")?,
        ))
    }
    pub fn new(blacklist: &[String], preferred: &[String]) -> Self {
        Self(
            Arc::new(Mutex::new(Cache {
                blacklist: blacklist.iter().map(|p| Glob::new(p, false)).collect(),
                preferred: preferred.iter().map(|p| Glob::new(p, false)).collect(),
                ..Cache::default()
            })),
            Arc::new(tokio::sync::Notify::new()),
        )
    }
    /// Record a measurement or restore a timestamped persistent cache entry.
    pub fn record(&self, host: &str, milliseconds: f64, measured: SystemTime) -> Result<()> {
        ensure!(
            milliseconds.is_finite() && milliseconds >= 0.0,
            "invalid mirror ping time"
        );
        self.0
            .lock()
            .expect("mirror timing cache")
            .times
            .insert(host.to_owned(), (milliseconds, measured));
        self.1.notify_waiters();
        Ok(())
    }
    /// Status matches MacPorts: -2 blacklisted, -1 unknown, 0 stale,
    /// 1 fresh, 2 preferred. Host policy takes precedence over cached timings.
    pub fn lookup(&self, host: &str, now: SystemTime) -> (i64, Option<f64>) {
        let cache = self.0.lock().expect("mirror timing cache");
        cache.lookup(host, now)
    }
    /// Wait for this URL's host to be fresh or excluded by host policy.
    /// Like upstream, the caller starts probes separately. This future may be
    /// cancelled by dropping it; no blocking thread or cache reservation is held.
    pub async fn wait_for_url(&self, url: &str) -> Result<()> {
        let (_, Some(host)) = parts(url)? else {
            return Ok(());
        };
        loop {
            // Register before inspecting the cache, so a result arriving between
            // the status check and await cannot be lost. notify_waiters wakes all
            // sessions waiting on the same host, unlike a single stored permit.
            let changed = self.1.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if !matches!(self.lookup(&host, SystemTime::now()).0, -1 | 0) {
                return Ok(());
            }
            changed.await;
        }
    }
    pub fn compare(&self, a: &str, b: &str, now: SystemTime) -> Result<i64> {
        let a = parts(a)?;
        let b = parts(b)?;
        if a.0 == "file" {
            return Ok(if b.0 == "file" { 0 } else { -1 });
        }
        if b.0 == "file" {
            return Ok(1);
        }
        let (a, b) = match (a.1, b.1) {
            (None, b) => return Ok(i64::from(b.is_some())),
            (Some(_), None) => return Ok(-1),
            (Some(a), Some(b)) => (self.lookup(&a, now), self.lookup(&b, now)),
        };
        if let (Some(a_ms), Some(b_ms)) = (a.1, b.1) {
            let difference = (a_ms - b_ms).round() as i64;
            if difference != 0 {
                return Ok(difference);
            }
        }
        Ok(b.0 - a.0)
    }
}
pub(super) fn parts(text: &str) -> Result<(String, Option<String>)> {
    match reqwest::Url::parse(text) {
        Ok(url) => Ok((url.scheme().to_owned(), url.host_str().map(str::to_owned))),
        Err(_) if !text.contains(':') => Ok((String::new(), None)),
        Err(error) => Err(error.into()),
    }
}

impl Cache {
    fn lookup(&self, host: &str, now: SystemTime) -> (i64, Option<f64>) {
        if self.blacklist.iter().any(|p| p.matches(host)) {
            return (-2, None);
        }
        if self.preferred.iter().any(|p| p.matches(host)) {
            return (2, None);
        }
        match self.times.get(host) {
            None => (-1, None),
            Some((ms, measured)) => {
                let fresh = now.duration_since(*measured).unwrap_or(Duration::ZERO)
                    <= Duration::from_secs(86400);
                (i64::from(fresh), Some(*ms))
            }
        }
    }
}

struct PendingProbe {
    cache: MirrorTimings,
    host: String,
}
impl Drop for PendingProbe {
    fn drop(&mut self) {
        self.cache
            .0
            .lock()
            .expect("mirror timing cache")
            .pending
            .remove(&self.host);
    }
}
impl MirrorTimings {
    /// Probe missing/stale hosts concurrently. Failures receive upstream's
    /// 10000ms penalty; preferred, blacklisted and fresh hosts are not probed.
    /// Duplicate or already pending hosts are skipped across shared sessions.
    pub async fn probe_urls(&self, urls: &[String], jobs: usize) -> Result<usize> {
        self.probe_with(urls, jobs, |host, service| async move {
            super::mirror_probe::time_connect(&host, &service).await
        })
        .await
    }
    async fn probe_with<F, Fut>(&self, urls: &[String], jobs: usize, probe: F) -> Result<usize>
    where
        F: Fn(String, String) -> Fut,
        Fut: std::future::Future<Output = Result<f64>>,
    {
        use futures_util::{StreamExt, stream};
        ensure!(
            (1..=128).contains(&jobs),
            "probe jobs must be between 1 and 128"
        );
        // Parse the whole batch before starting network work.
        let requests = urls
            .iter()
            .map(|url| parts(url))
            .collect::<Result<Vec<_>>>()?;
        let results = stream::iter(
            requests
                .into_iter()
                .filter_map(|(service, host)| host.map(|host| (host, service)))
                .map(|(host, service)| {
                    let probe = &probe;
                    async move {
                        let pending = {
                            let mut cache = self.0.lock().expect("mirror timing cache");
                            if !matches!(cache.lookup(&host, SystemTime::now()).0, -1 | 0)
                                || !cache.pending.insert(host.clone())
                            {
                                return 0;
                            }
                            PendingProbe {
                                cache: self.clone(),
                                host: host.clone(),
                            }
                        };
                        let ms = probe(host.clone(), service)
                            .await
                            .ok()
                            .filter(|ms| ms.is_finite() && *ms >= 0.0)
                            .unwrap_or(10000.0);
                        self.record(&host, ms, SystemTime::now())
                            .expect("validated timing");
                        drop(pending);
                        1
                    }
                }),
        )
        .buffer_unordered(jobs)
        .collect::<Vec<usize>>()
        .await;
        Ok(results.into_iter().sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn shared_batches_deduplicate_bound_concurrency_and_cache_failures() {
        let cache = MirrorTimings::new(&["blocked".into()], &["preferred".into()]);
        cache.record("fresh", 1.0, SystemTime::now()).unwrap();
        cache.record("stale", 2.0, SystemTime::UNIX_EPOCH).unwrap();
        let urls: Vec<String> = [
            "http://a/",
            "https://a/",
            "http://b/",
            "http://stale/",
            "http://failed/",
            "http://fresh/",
            "http://blocked/",
            "http://preferred/",
            "file:///local",
        ]
        .map(str::to_owned)
        .to_vec();
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let calls = AtomicUsize::new(0);
        let probe = |host: String, _: String| {
            let (active, peak, calls) = (&active, &peak, &calls);
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let running = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(running, Ordering::SeqCst);
                tokio::task::yield_now().await;
                active.fetch_sub(1, Ordering::SeqCst);
                if host == "failed" {
                    anyhow::bail!("connection refused");
                }
                Ok(5.0)
            }
        };
        assert_eq!(cache.probe_with(&urls, 2, &probe).await.unwrap(), 4);
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(
            cache.lookup("failed", SystemTime::now()),
            (1, Some(10000.0))
        );
        assert_eq!(cache.probe_with(&urls, 2, &probe).await.unwrap(), 0);
        let shared = cache.clone();
        let urls = vec!["http://new/".into()];
        let (a, b) = tokio::join!(
            cache.probe_with(&urls, 2, &probe),
            shared.probe_with(&urls, 2, &probe)
        );
        assert_eq!(a.unwrap() + b.unwrap(), 1);
    }

    #[tokio::test]
    async fn cancellation_releases_host_and_invalid_batches_do_no_work() {
        let cache = MirrorTimings::default();
        let urls = vec!["http://cancelled/".into()];
        {
            let pending = cache.probe_with(&urls, 1, |_, _| std::future::pending::<Result<f64>>());
            tokio::pin!(pending);
            assert!(futures_util::poll!(&mut pending).is_pending());
            assert!(cache.0.lock().unwrap().pending.contains("cancelled"));
        }
        assert!(cache.0.lock().unwrap().pending.is_empty());
        assert_eq!(
            cache
                .probe_with(&urls, 1, |_, _| async { Ok(7.0) })
                .await
                .unwrap(),
            1
        );
        let invalid = vec!["http://valid/".into(), "http://[broken".into()];
        assert!(
            cache
                .probe_with(&invalid, 1, |_, _| async {
                    panic!("invalid batch must not probe")
                })
                .await
                .is_err()
        );
        assert!(cache.probe_urls(&urls, 0).await.is_err());
    }
}

#[cfg(test)]
mod wait_tests {
    use super::*;
    use futures_util::poll;

    #[tokio::test]
    async fn wakes_all_matching_waiters_without_waiting_for_other_hosts() {
        let cache = MirrorTimings::default();
        cache.record("fast", 20.0, SystemTime::UNIX_EPOCH).unwrap();
        let one = cache.wait_for_url("http://fast/a");
        let two = cache.wait_for_url("https://fast/b");
        let slow = cache.wait_for_url("https://slow/");
        tokio::pin!(one, two, slow);
        assert!(poll!(&mut one).is_pending());
        assert!(poll!(&mut two).is_pending());
        assert!(poll!(&mut slow).is_pending());
        // A stale update is a notification, but must not satisfy the wait.
        cache.record("fast", 5.0, SystemTime::UNIX_EPOCH).unwrap();
        assert!(poll!(&mut one).is_pending());
        cache.record("fast", 2.0, SystemTime::now()).unwrap();
        assert!(poll!(&mut one).is_ready());
        assert!(poll!(&mut two).is_ready());
        assert!(poll!(&mut slow).is_pending());
        cache.record("slow", 10000.0, SystemTime::now()).unwrap();
        slow.await.unwrap();
    }

    #[tokio::test]
    async fn policy_and_cached_results_do_not_require_a_running_probe() {
        let cache = MirrorTimings::new(&["blocked".into()], &["preferred".into()]);
        cache.record("fresh", 3.0, SystemTime::now()).unwrap();
        for url in [
            "http://blocked/",
            "http://preferred/",
            "http://fresh/",
            "file:///tmp/source",
            "relative",
        ] {
            let waiter = cache.wait_for_url(url);
            tokio::pin!(waiter);
            assert!(matches!(poll!(&mut waiter), std::task::Poll::Ready(Ok(()))));
        }
        assert!(cache.wait_for_url("http://[invalid").await.is_err());
    }

    #[tokio::test]
    async fn actual_batch_results_release_waiters_before_batch_completion() {
        let cache = MirrorTimings::default();
        let urls = vec!["http://first/".into(), "http://second/".into()];
        let batch = cache.probe_with(&urls, 2, |host, _| async move {
            if host == "second" {
                std::future::pending::<()>().await;
            }
            Ok(2.0)
        });
        let waiter = cache.wait_for_url("http://first/");
        tokio::pin!(batch, waiter);
        assert!(poll!(&mut waiter).is_pending());
        assert!(poll!(&mut batch).is_pending());
        assert!(poll!(&mut waiter).is_ready());
        assert_eq!(cache.lookup("second", SystemTime::now()).0, -1);
    }
}
