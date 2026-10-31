//! Shared mirror timing cache and MacPorts URL ordering policy.
use anyhow::{Context as _, Result, ensure};
use molt::glob::Glob;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

#[derive(Clone, Default)]
pub struct MirrorTimings(Arc<Mutex<Cache>>);
#[derive(Default)]
struct Cache {
    blacklist: Vec<Glob>,
    preferred: Vec<Glob>,
    times: BTreeMap<String, (f64, SystemTime)>,
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
        Self(Arc::new(Mutex::new(Cache {
            blacklist: blacklist.iter().map(|p| Glob::new(p, false)).collect(),
            preferred: preferred.iter().map(|p| Glob::new(p, false)).collect(),
            ..Cache::default()
        })))
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
        Ok(())
    }
    /// Status matches MacPorts: -2 blacklisted, -1 unknown, 0 stale,
    /// 1 fresh, 2 preferred. Host policy takes precedence over cached timings.
    pub fn lookup(&self, host: &str, now: SystemTime) -> (i64, Option<f64>) {
        let cache = self.0.lock().expect("mirror timing cache");
        if cache.blacklist.iter().any(|p| p.matches(host)) {
            return (-2, None);
        }
        if cache.preferred.iter().any(|p| p.matches(host)) {
            return (2, None);
        }
        match cache.times.get(host) {
            None => (-1, None),
            Some((ms, measured)) => {
                let fresh = now.duration_since(*measured).unwrap_or(Duration::ZERO)
                    <= Duration::from_secs(86400);
                (i64::from(fresh), Some(*ms))
            }
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
fn parts(text: &str) -> Result<(String, Option<String>)> {
    match reqwest::Url::parse(text) {
        Ok(url) => Ok((url.scheme().to_owned(), url.host_str().map(str::to_owned))),
        Err(_) if !text.contains(':') => Ok((String::new(), None)),
        Err(error) => Err(error.into()),
    }
}
