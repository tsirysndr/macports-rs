//! Resolve an inactive registry image against an evaluated activation plan.
use super::*;
use crate::{registry::Entry, tcl_list};

/// Match a fresh registry snapshot to the exact requested image. Registry
/// membership is not archive authentication; use Store::load before activation.
pub fn select_inactive<'a>(
    entries: &'a [Entry],
    item: &Item<'_>,
    target: &Target,
) -> Result<&'a Entry> {
    ensure!(
        item.action == Action::Activate,
        "inactive image selection requires an activate plan step"
    );
    ensure!(
        item.variants_resolved && item.provider_fallbacks.is_empty(),
        "inactive image selection requires resolved variants and dependency providers"
    );
    let number = |field| -> Result<i64> {
        let value = item.port.get(field);
        let value = if value.is_empty() {
            0
        } else {
            value
                .parse()
                .with_context(|| format!("invalid planned {field}"))?
        };
        ensure!(value >= 0, "negative planned {field}");
        Ok(value)
    };
    let revision = number("revision")?;
    let epoch = number("epoch")?;
    let mut selected = None;
    for entry in entries
        .iter()
        .filter(|e| e.name.eq_ignore_ascii_case(&item.port.name))
    {
        ensure!(
            !entry.active,
            "{} already has an active image; refresh the installation plan",
            item.port.name
        );
        let installed_epoch = if entry.epoch.is_empty() {
            0
        } else {
            entry
                .epoch
                .parse::<i64>()
                .context("invalid installed epoch")?
        };
        if entry.version != item.port.get("version")
            || entry.revision != revision
            || installed_epoch != epoch
            || entry.variants != item.variants
        {
            continue;
        }
        let mut archs = tcl_list::parse(&entry.archs)?;
        archs.sort();
        archs.dedup();
        if format!("{}_{}", entry.platform, entry.os_major) != target.platform()
            || archs != target.architectures()
        {
            continue;
        }
        ensure!(
            selected.is_none(),
            "multiple inactive images match {}; registry selection is ambiguous",
            item.port.name
        );
        selected = Some(entry);
    }
    selected.with_context(|| {
        format!(
            "no compatible inactive image for {} @{}{}; refresh the installation plan",
            item.port.name,
            item.port.version(),
            item.variants
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{index::Port, install::target::Architecture, source::Platform};
    use std::collections::BTreeMap;
    fn entry(id: i64) -> Entry {
        Entry {
            id,
            name: "App".into(),
            version: "1".into(),
            revision: 2,
            epoch: "3".into(),
            variants: "+ssl".into(),
            active: false,
            requested: true,
            requested_variants: "+ssl".into(),
            platform: "darwin".into(),
            os_major: "24".into(),
            archs: "arm64".into(),
            date: String::new(),
        }
    }
    #[test]
    fn matches_complete_identity_and_rejects_stale_or_ambiguous_snapshots() {
        let port = Port {
            name: "app".into(),
            fields: BTreeMap::from([
                ("version".into(), "1".into()),
                ("revision".into(), "2".into()),
                ("epoch".into(), "3".into()),
            ]),
        };
        let item = Item {
            port: &port,
            variants: "+ssl".into(),
            variants_resolved: true,
            provider_fallbacks: vec![],
            requested: true,
            action: Action::Activate,
        };
        let target = Target::macos(
            &Platform {
                darwin_major: 24,
                architecture: "arm",
            },
            "darwin",
            Architecture::Build("arm64"),
        )
        .unwrap();
        assert_eq!(
            select_inactive(&[entry(17)], &item, &target).unwrap().id,
            17
        );
        for field in [
            "version", "revision", "epoch", "variants", "platform", "os_major", "archs",
        ] {
            let mut e = entry(17);
            match field {
                "version" => e.version = "2".into(),
                "revision" => e.revision = 3,
                "epoch" => e.epoch = "4".into(),
                "variants" => e.variants.clear(),
                "platform" => e.platform = "any".into(),
                "os_major" => e.os_major = "23".into(),
                "archs" => e.archs = "x86_64".into(),
                _ => unreachable!(),
            }
            assert!(select_inactive(&[e], &item, &target).is_err(), "{field}");
        }
        assert!(
            select_inactive(&[entry(1), entry(2)], &item, &target)
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        let mut active = entry(3);
        active.active = true;
        active.version = "older".into();
        assert!(
            select_inactive(&[entry(1), active], &item, &target)
                .unwrap_err()
                .to_string()
                .contains("active image")
        );
        let mut malformed = entry(4);
        malformed.epoch = "invalid".into();
        assert!(select_inactive(&[malformed], &item, &target).is_err());
    }
}
