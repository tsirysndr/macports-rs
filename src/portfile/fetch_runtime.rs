//! Native source-fetch cache inspection and transport dispatch.
use super::*;
use molt::{MoltResult, check_args};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("portfetch::files_present", files_present, id);
    interp.add_context_command("portfetch::fetch_main", main, id);
}

fn files_present(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        let files = interp.scalar("portfetch::fetch_urls")?.as_list()?;
        // Match foreach's padding of an odd final pair. File presence alone
        // does not replace the subsequent checksum target.
        for pair in files.chunks(2) {
            let directory = interp.scalar("distpath")?;
            let filename = pair.get(1).map(Value::as_str).unwrap_or("");
            let path = format!("{}/{filename}", directory.as_str());
            if !std::path::Path::new(&path).is_file() {
                return Ok(false.into());
            }
        }
        Ok(true.into())
    })
}

fn main(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    if interp.context::<State>(id).system_executor.is_none() {
        return commands::unsupported(interp, id, argv);
    }
    interp.with_global_scope(|interp| {
        let kind = interp.scalar("fetch.type")?;
        if kind.as_str() == "standard" && !interp.var_exists(&"all_dist_files".into()) {
            return Ok(0.into());
        }
        let command = match kind.as_str() {
            "bzr" => "portfetch::bzrfetch",
            "cvs" => "portfetch::cvsfetch",
            "svn" => "portfetch::svnfetch",
            "git" => "portfetch::gitfetch",
            "hg" => "portfetch::hgfetch",
            _ => "portfetch::fetchfiles",
        };
        interp.eval_value(&Value::from(vec![Value::from(command)]))
    })
}
