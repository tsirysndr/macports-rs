use port_rs::{
    install::{mirror_times::MirrorTimings, mirror_worker::MirrorWorker},
    portfile::{PortGroups, Session},
    spec::PortSpec,
};
use std::{collections::BTreeMap, time::SystemTime};

#[tokio::test(flavor = "current_thread")]
async fn worker_runs_independently_of_callers_runtime_and_shares_results() {
    let cache = MirrorTimings::default();
    let worker = MirrorWorker::new(cache.clone(), 2).unwrap();
    // An unknown service deterministically exercises resolver failure -> penalty.
    let url = "no-such-probe-service://127.0.0.1/";
    worker.start(vec![url.into(), url.into()]).unwrap();
    worker.clone().wait(url).unwrap();
    assert_eq!(
        cache.lookup("127.0.0.1", SystemTime::now()),
        (1, Some(10000.0))
    );
    worker.wait("file:///tmp/local").unwrap();
    assert!(worker.start(vec!["http://[broken".into()]).is_err());
    assert!(MirrorWorker::new(cache, 0).is_err());
    drop(worker); // Must not panic by dropping a Tokio runtime on this thread.
}
#[test]
fn native_callbacks_use_bound_worker_and_require_execution_mode() {
    let mut s = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    let cache = MirrorTimings::default();
    let worker = MirrorWorker::new(cache.clone(), 2).unwrap();
    s.set_mirror_worker(worker.clone());
    s.enable_system(&BTreeMap::new(), |_| panic!("no shell expected"))
        .unwrap();
    let url = "no-such-probe-service://127.0.0.1/";
    s.invoke("async_ping_start", &[url]).unwrap();
    s.invoke("wait_for_pingtime", &[url]).unwrap();
    assert_eq!(
        s.invoke("compare_pingtimes", &[url, "http://unknown/"])
            .unwrap(),
        "-2"
    );
    let mut readonly = Session::open(
        "name app; version 1",
        &BTreeMap::new(),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    readonly.set_mirror_worker(worker);
    assert!(readonly.invoke("async_ping_start", &[url]).is_err());
}
