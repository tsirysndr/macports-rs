//! `port usage` and `port help`, as upstream: usage lines come from the
//! action option table, and help opens MacPorts' man pages when installed.
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

/// Upstream's `cmd_opts_array`: each action's flags and how many arguments
/// each takes.
const OPTIONS: &[(&str, &[(&str, usize)])] = &[
    ("edit", &[("editor", 1)]),
    (
        "info",
        &[
            ("category", 0),
            ("categories", 0),
            ("conflicts", 0),
            ("depends_fetch", 0),
            ("depends_extract", 0),
            ("depends_patch", 0),
            ("depends_build", 0),
            ("depends_lib", 0),
            ("depends_run", 0),
            ("depends_test", 0),
            ("depends", 0),
            ("description", 0),
            ("epoch", 0),
            ("fullname", 0),
            ("heading", 0),
            ("homepage", 0),
            ("index", 0),
            ("license", 0),
            ("line", 0),
            ("long_description", 0),
            ("maintainer", 0),
            ("maintainers", 0),
            ("name", 0),
            ("patchfiles", 0),
            ("platform", 0),
            ("platforms", 0),
            ("portdir", 0),
            ("pretty", 0),
            ("replaced_by", 0),
            ("revision", 0),
            ("subports", 0),
            ("variant", 0),
            ("variants", 0),
            ("version", 0),
        ],
    ),
    ("contents", &[("size", 0), ("units", 1)]),
    ("deps", &[("index", 0), ("no-build", 0), ("no-test", 0)]),
    (
        "rdeps",
        &[("index", 0), ("no-build", 0), ("no-test", 0), ("full", 0)],
    ),
    ("rdependents", &[("full", 0)]),
    (
        "search",
        &[
            ("case-sensitive", 0),
            ("category", 0),
            ("categories", 0),
            ("depends_fetch", 0),
            ("depends_extract", 0),
            ("depends_patch", 0),
            ("depends_build", 0),
            ("depends_lib", 0),
            ("depends_run", 0),
            ("depends_test", 0),
            ("depends", 0),
            ("description", 0),
            ("epoch", 0),
            ("exact", 0),
            ("glob", 0),
            ("homepage", 0),
            ("line", 0),
            ("long_description", 0),
            ("maintainer", 0),
            ("maintainers", 0),
            ("name", 0),
            ("platform", 0),
            ("platforms", 0),
            ("portdir", 0),
            ("regex", 0),
            ("revision", 0),
            ("variant", 0),
            ("variants", 0),
            ("version", 0),
        ],
    ),
    (
        "selfupdate",
        &[("migrate", 0), ("no-sync", 0), ("nosync", 0), ("rsync", 0)],
    ),
    ("space", &[("units", 1), ("total", 0)]),
    ("activate", &[("no-exec", 0)]),
    ("deactivate", &[("no-exec", 0)]),
    (
        "install",
        &[
            ("allow-failing", 0),
            ("no-activate", 0),
            ("no-replace", 0),
            ("no-rev-upgrade", 0),
            ("unrequested", 0),
        ],
    ),
    (
        "uninstall",
        &[
            ("follow-dependents", 0),
            ("follow-dependencies", 0),
            ("no-exec", 0),
        ],
    ),
    ("variants", &[("index", 0)]),
    (
        "clean",
        &[
            ("all", 0),
            ("archive", 0),
            ("dist", 0),
            ("work", 0),
            ("logs", 0),
        ],
    ),
    ("mirror", &[("new", 0)]),
    ("lint", &[("nitpick", 0)]),
    (
        "select",
        &[("list", 0), ("set", 0), ("show", 0), ("summary", 0)],
    ),
    ("log", &[("phase", 1), ("level", 1)]),
    (
        "upgrade",
        &[
            ("force", 0),
            ("enforce-variants", 0),
            ("no-replace", 0),
            ("no-rev-upgrade", 0),
        ],
    ),
    ("rev-upgrade", &[("id-loadcmd-check", 0)]),
    ("diagnose", &[("quiet", 0)]),
    (
        "reclaim",
        &[
            ("enable-reminders", 0),
            ("disable-reminders", 0),
            ("keep-build-deps", 0),
        ],
    ),
    ("fetch", &[("no-mirrors", 0)]),
    ("bump", &[("patch", 0)]),
    (
        "snapshot",
        &[
            ("create", 0),
            ("list", 0),
            ("diff", 1),
            ("all", 0),
            ("delete", 1),
            ("help", 0),
            ("note", 1),
            ("export", 1),
            ("import", 1),
        ],
    ),
    ("restore", &[("snapshot-id", 1), ("all", 0), ("last", 0)]),
    ("migrate", &[("continue", 0), ("all", 0)]),
];

/// Actions taking free-form strings, and actions taking nothing; every other
/// action takes a port list (upstream `action_array`).
const STRINGS: &[&str] = &[
    "usage", "help", "provides", "select", "search", "snapshot", "restore", "migrate",
];
const NONE: &[&str] = &[
    "sync",
    "selfupdate",
    "rev-upgrade",
    "reclaim",
    "diagnose",
    "version",
    "platform",
    "quit",
    "exit",
];

pub(super) fn print_usage() {
    eprintln!(
        "Usage: port\n        [-bcdfknNopqRstuvy] [-D portdir|portname] [-F cmdfile] action [actionflags]\n        [[portname|pseudo-portname|port-url] [@version] [+-variant]... [option=value]...]...\n    \n\"port help\" or \"man 1 port\" for more information."
    );
}

/// One action's usage line, as upstream `action_get_usage`.
fn usage_line(action: &str) -> String {
    let mut line = format!("Usage: {action}");
    if let Some((_, options)) = OPTIONS.iter().find(|(name, _)| *name == action) {
        for (option, count) in *options {
            line.push_str(&format!(" --{option}"));
            for index in 1..=*count {
                line.push_str(&format!(" <arg{index}>"));
            }
        }
    }
    if STRINGS.contains(&action) {
        line.push_str(" <arguments>");
    } else if !NONE.contains(&action) {
        line.push_str(" <portlist>");
    }
    line
}

pub(super) fn usage(topics: &[String]) -> Result<()> {
    if topics.is_empty() {
        print_usage();
        return Ok(());
    }
    for topic in topics {
        if !super::ACTIONS.contains(&topic.as_str()) {
            bail!("No usage for topic {topic}");
        }
        eprintln!("{}", usage_line(topic));
    }
    Ok(())
}

fn prefix() -> PathBuf {
    super::local::configuration()
        .and_then(|config| config.get("prefix").map(String::from))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/opt/local"))
}

/// The man page for a topic: `port.1`, `port-<action>.1`, a section 5 or 7
/// page, or `portundocumented.7` for actions without a page.
fn man_page(prefix: &Path, action: Option<&str>, topic: Option<&str>) -> Option<PathBuf> {
    let man = prefix.join("share/man");
    let page = match (action, topic) {
        (None, None) => Some("man1/port.1.gz".to_string()),
        (Some(action), _) => Some(format!("man1/port-{action}.1.gz")),
        (None, Some(topic)) => ["5", "7"]
            .iter()
            .map(|section| format!("man{section}/{topic}.{section}.gz"))
            .find(|page| man.join(page).is_file()),
    }?;
    let path = man.join(&page);
    if path.is_file() {
        Some(path)
    } else if action.is_some() || topic.is_none() {
        Some(man.join("man7/portundocumented.7.gz")).filter(|path| path.is_file())
    } else {
        None
    }
}

/// Show `port help [topic]`; without MacPorts' man pages, show this
/// implementation's own help for the action.
pub(super) fn help(
    topics: &[String],
    fallback: impl FnOnce(Option<&str>) -> Result<()>,
) -> Result<()> {
    let topic = topics.first().map(String::as_str);
    let action = topic.filter(|topic| super::ACTIONS.contains(topic));
    if let Some(page) = man_page(&prefix(), action, topic) {
        let status = std::process::Command::new("/usr/bin/man")
            .arg(&page)
            .status();
        match status {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => bail!("Unable to show man page using /usr/bin/man: {status}"),
            Err(err) => bail!("Unable to show man page using /usr/bin/man: {err}"),
        }
    }
    match (topic, action) {
        (Some(_), None) => bail!("Sorry, no help for this topic is available."),
        _ => fallback(action),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_lines_match_upstream() {
        assert_eq!(
            usage_line("install"),
            "Usage: install --allow-failing --no-activate --no-replace --no-rev-upgrade --unrequested <portlist>"
        );
        assert_eq!(
            usage_line("log"),
            "Usage: log --phase <arg1> --level <arg1> <portlist>"
        );
        assert_eq!(usage_line("sync"), "Usage: sync");
        assert_eq!(usage_line("provides"), "Usage: provides <arguments>");
    }

    #[test]
    fn man_pages_fall_back_to_portundocumented() {
        let temp = tempfile::tempdir().unwrap();
        let man = temp.path().join("share/man");
        std::fs::create_dir_all(man.join("man1")).unwrap();
        std::fs::create_dir_all(man.join("man5")).unwrap();
        std::fs::create_dir_all(man.join("man7")).unwrap();
        for page in [
            "man1/port.1.gz",
            "man1/port-install.1.gz",
            "man5/macports.conf.5.gz",
            "man7/portundocumented.7.gz",
        ] {
            std::fs::write(man.join(page), "").unwrap();
        }
        let page = |action, topic| {
            man_page(temp.path(), action, topic)
                .map(|path| path.strip_prefix(&man).unwrap().display().to_string())
        };
        assert_eq!(page(None, None).as_deref(), Some("man1/port.1.gz"));
        assert_eq!(
            page(Some("install"), Some("install")).as_deref(),
            Some("man1/port-install.1.gz")
        );
        assert_eq!(
            page(Some("sync"), Some("sync")).as_deref(),
            Some("man7/portundocumented.7.gz")
        );
        assert_eq!(
            page(None, Some("macports.conf")).as_deref(),
            Some("man5/macports.conf.5.gz")
        );
        assert_eq!(page(None, Some("nothing")), None);
    }
}
