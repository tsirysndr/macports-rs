//! Once-per-process diagnostic for DNS wildcard interception.
use super::*;
use molt::{MoltResult, check_args};
use std::sync::atomic::{AtomicBool, Ordering};
static CHECKED: AtomicBool = AtomicBool::new(false);

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("check_broken_dns", broken, id);
    interp.add_context_command("portfetch::check_dns", check, id);
}
fn once(checked: &AtomicBool, resolve: impl FnOnce() -> bool) -> bool {
    !checked.swap(true, Ordering::AcqRel) && resolve()
}
fn resolves_invalid_host() -> bool {
    let mut result = std::ptr::null_mut();
    // SAFETY: the literal is NUL terminated. Null service/hints are accepted by
    // getaddrinfo; the output allocation is released below, including on error.
    let status = unsafe {
        libc::getaddrinfo(
            c"invalid-host.macports.org".as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            &mut result,
        )
    };
    if !result.is_null() {
        // SAFETY: result belongs to the preceding getaddrinfo call.
        unsafe { libc::freeaddrinfo(result) };
    }
    status == 0
}
fn broken(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    Ok(once(&CHECKED, resolves_invalid_host).into())
}
fn check(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        if interp.eval_value(&Value::from(vec![Value::from("check_broken_dns")]))?.as_bool()? {
            interp.eval_value(&Value::from(vec!["ui_warn".into(),
                "Your DNS servers incorrectly claim to know the address of nonexistent hosts. This may cause checksum mismatches for some ports. See this page for more information: <https://trac.macports.org/wiki/MisbehavingServers>".into()]))?;
        }
        Ok(Value::empty())
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    #[test]
    fn only_one_concurrent_caller_resolves_and_can_report_broken_dns() {
        let checked = AtomicBool::new(false);
        let calls = AtomicUsize::new(0);
        let reports = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..16)
                .map(|_| {
                    scope.spawn(|| {
                        once(&checked, || {
                            calls.fetch_add(1, Ordering::Relaxed);
                            true
                        })
                    })
                })
                .collect();
            threads
                .into_iter()
                .map(|thread| usize::from(thread.join().unwrap()))
                .sum::<usize>()
        });
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(reports, 1);
        let healthy = AtomicBool::new(false);
        assert!(!once(&healthy, || false));
        assert!(!once(&healthy, || panic!(
            "healthy results must also be cached"
        )));
    }
}
