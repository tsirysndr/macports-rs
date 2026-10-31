//! Native archive coordinates and archivefetch's candidate-file bookkeeping.
use super::*;
use molt::{MoltResult, check_args};
use ripemd::{Digest, Ripemd160};

pub(super) fn install(interp: &mut Interp, id: ContextID) {
    interp.add_context_command("get_portimage_name", image_name, id);
    interp.add_context_command("_get_archive_platform", platform, id);
    interp.add_context_command("portarchivefetch::checkarchivefiles", check_files, id);
}

fn platform(interp: &mut Interp, _: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        let os = interp.scalar("os.platform")?.to_string();
        for entry in interp.scalar("platforms")?.as_list()?.iter() {
            if entry.as_str() == "any" {
                return Ok("any_any".into());
            }
            let parts = entry.as_list()?;
            let first = parts.first().map(Value::as_str).unwrap_or("");
            if (first == os || first == interp.scalar("os.subplatform")?.as_str())
                && parts.get(1).is_some_and(|part| part.as_str() == "any")
            {
                return Ok(format!("{os}_any").into());
            }
        }
        Ok(format!("{os}_{}", interp.scalar("os.major")?).into())
    })
}

fn image_name(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 1, 1, "")?;
    interp.with_global_scope(|interp| {
        let root = format!(
            "{}-{}_{}",
            interp.scalar("subport")?,
            interp.scalar("version")?,
            interp.scalar("revision")?
        );
        let variants = interp.scalar("portvariants")?.to_string();
        let platform = platform(interp, id, &["_get_archive_platform".into()])?;
        let archs = interp
            .eval_value(&list(["get_canonical_archs".into()]))?
            .as_list()?
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("-");
        let suffix = format!(".{platform}.{archs}.{}", interp.scalar("portarchivetype")?);
        let mut filename = format!("{root}{variants}{suffix}");
        if filename.encode_utf16().count() > 245 && !variants.is_empty() {
            let hash = Ripemd160::digest(variants.as_bytes());
            let hash: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
            filename = format!("{root}+{hash}{suffix}");
        }
        if filename.encode_utf16().count() > 245 {
            return Err(error(format!("filename too long: {filename}")));
        }
        Ok(filename.into())
    })
}

pub(super) fn check_files(interp: &mut Interp, id: ContextID, argv: &[Value]) -> MoltResult {
    check_args(1, argv, 2, 2, "urls")?;
    // The native command executes in its caller's frame, so the supplied URL
    // variable remains local while the archive bookkeeping is global.
    let files = interp.with_global_scope(|interp| -> Result<Vec<Value>, Exception> {
        let destination = paths::join([interp.scalar("portdbpath")?.as_str(), "incoming/verified"]);
        interp.set_scalar("archivefetch.fulldestpath", destination.into())?;
        let filename = image_name(interp, id, &["get_portimage_name".into()])?.to_string();
        let root = filename
            .rsplit_once('.')
            .map_or(filename.as_str(), |(root, _)| root);
        let sites = interp.scalar("archive_sites")?.as_list()?;
        let mut seen = BTreeSet::new();
        let mut files = Vec::new();
        for site in sites.iter() {
            let kind = site.as_str().rsplit(':').next().unwrap_or("");
            if seen.insert(kind.to_owned()) {
                files.extend([kind.into(), format!("{root}.{kind}").into()]);
            }
        }
        Ok(files)
    })?;
    for pair in files.as_chunks::<2>().0 {
        interp.with_global_scope(|interp| {
            let mut all = if interp.var_exists(&"all_archive_files".into()) {
                interp.scalar("all_archive_files")?.as_list()?.to_vec()
            } else {
                Vec::new()
            };
            all.push(pair[1].clone());
            interp.set_scalar("all_archive_files", Value::from(all))
        })?;
        let mut urls = if interp.var_exists(&argv[1]) {
            interp.var(&argv[1])?.as_list()?.to_vec()
        } else {
            Vec::new()
        };
        urls.extend_from_slice(pair);
        interp.set_var(&argv[1], Value::from(urls))?;
    }
    Ok(Value::empty())
}
