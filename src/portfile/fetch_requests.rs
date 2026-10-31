//! Live source-fetch candidates and the native standard-fetch transport.
use super::*;
use crate::install::source::Request;
use molt::{MoltResult, check_args};
pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("percent_encode", encode, id);
    interp.add_context_command("assemble_url", assemble, id);
    interp.add_context_command("portfetch::fetchfiles", fetch_files, id);
}
fn encode(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "str")?;
    Ok(crate::install::source::percent_encode(argv[1].as_str()).into())
}
fn assemble(_: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 3, 3, "site distfile")?;
    Ok(crate::install::source::assemble_url(argv[1].as_str(), argv[2].as_str()).into())
}
fn fetch_files(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    let requests = interp.with_global_scope(requests)?;
    if requests.is_empty() {
        return Ok(Value::empty());
    }
    let options = interp.with_global_scope(options)?;
    let state = interp.context::<State>(id);
    let progress = state
        .progress
        .clone()
        .unwrap_or_else(|| crate::progress::Progress::new(false));
    let proxies = state.proxies.clone();
    std::thread::scope(|scope| {
        scope
            .spawn(|| -> anyhow::Result<usize> {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?
                    .block_on(crate::install::source::download(
                        &requests,
                        &options,
                        &progress,
                        proxies.as_deref(),
                    ))
            })
            .join()
            .unwrap_or_else(|_| Err(anyhow::anyhow!("source fetch worker panicked")))
    })
    .map_err(|err| error(format!("{err:#}")))?;
    Ok(Value::empty())
}
fn options(interp: &mut Interp) -> Result<crate::install::source::Options, Exception> {
    let mut text = |name: &str| -> Result<String, Exception> {
        Ok(if interp.var_exists(&name.into()) {
            interp.scalar(name)?.to_string()
        } else {
            String::new()
        })
    };
    let (user, password) = (text("fetch.user")?, text("fetch.password")?);
    let ignore = text("fetch.ignore_sslcert")?;
    let remote = text("fetch.remote_time")?;
    let agent = text("fetch.user_agent")?;
    let prefix = text("UI_PREFIX")?;
    let flag = |value: &str| -> Result<bool, Exception> {
        Ok(!value.is_empty() && Value::from(value).as_bool()?)
    };
    Ok(crate::install::source::Options {
        credentials: (!user.is_empty() || !password.is_empty()).then_some((user, password)),
        ignore_sslcert: flag(&ignore)?,
        remote_time: flag(&remote)?,
        user_agent: (!agent.is_empty()).then_some(agent),
        prefix,
    })
}
fn requests(interp: &mut Interp) -> Result<Vec<Request>, Exception> {
    let candidates = interp.scalar("portfetch::fetch_urls")?.as_list()?;
    let mut sorted = false;
    let mut result = Vec::new();
    for pair in candidates.chunks(2) {
        let filename = pair.get(1).map(Value::as_str).unwrap_or("");
        let directory = interp.scalar("distpath")?;
        let path = format!("{}/{filename}", directory.as_str());
        if filesystem::predicate("isfile", &path) {
            continue;
        }
        if !filesystem::predicate("writable", directory.as_str()) {
            return Err(error(format!("{directory} must be writable")));
        }
        if !sorted {
            interp.eval_value(&Value::from(vec![
                "portfetch::sortsites".into(),
                "::portfetch::fetch_urls".into(),
                "master_sites".into(),
            ]))?;
            sorted = true;
        }
        let sites =
            fetch_selection::sites(interp, pair[0].as_str(), "master_sites", false)?.as_list()?;
        let mut urls = Vec::new();
        let mut assembled_sites = Vec::new();
        for site in sites.iter() {
            assembled_sites.push(site.to_string());
            urls.push(
                interp
                    .eval_value(&Value::from(vec![
                        "assemble_url".into(),
                        site.clone(),
                        filename.into(),
                    ]))?
                    .to_string(),
            );
        }
        result.push(Request {
            filename: filename.into(),
            destination: path.into(),
            urls,
            sites: assembled_sites,
        });
    }
    Ok(result)
}
impl Session {
    /// Build requests after fetch initialization and startup. This preserves
    /// repeated candidates and mirror order; it does not download or authenticate.
    pub fn source_fetch_requests(&mut self) -> Result<Vec<Request>> {
        check_supported(&mut self.interp, self.id)?;
        anyhow::ensure!(
            self.interp
                .context::<State>(self.id)
                .system_executor
                .is_some(),
            "source fetch requires execution mode"
        );
        let result = self.interp.with_global_scope(requests).map_err(failure);
        check_supported(&mut self.interp, self.id)?;
        result
    }

    /// Proxy settings for native source transport; `None` uses no explicit proxy.
    pub fn set_proxies(&mut self, proxies: Option<crate::install::proxy::Resolved>) {
        self.interp.context::<State>(self.id).proxies = proxies.map(std::sync::Arc::new);
    }
}
