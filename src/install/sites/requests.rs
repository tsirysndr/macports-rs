//! Mirror expansion and requests for the native Signify/tbz2 transport.
use super::{Selection, Site};
use crate::install::{Item, fetch::Request, target::Target};
use anyhow::{Result, ensure};
use std::sync::OnceLock;

/// Live Portfile values; archive.subdir normally defaults to the subport name.
pub struct Subdirectories<'a> {
    /// Portfile `name`, which may differ from the installed subport.
    pub port_name: &'a str,
    pub mirror: &'a str,
    pub archive: &'a str,
    pub dist: &'a str,
}

/// The references used to assemble the request also select its site trust keys.
pub struct PreparedRequest {
    pub request: Request,
    pub references: Vec<String>,
}

impl Selection {
    /// Resolve named archive classes against live Portfile state. The caller
    /// supplies initialized architecture coordinates and compatible site metadata.
    pub fn request_from_session(
        &self,
        session: &mut crate::portfile::Session,
        target: &Target,
        item: &Item<'_>,
        max_bytes: u64,
    ) -> Result<PreparedRequest> {
        let references = if session.invoke("exists", &["archive_sites"])? == "1" {
            crate::tcl_list::parse(&session.scalar("archive_sites")?)?
        } else {
            self.defaults.clone()
        };
        ensure!(
            !references.is_empty(),
            "{} has no archive sites",
            item.port.name
        );
        let name = session.scalar("name")?;
        let archive = session.scalar("archive.subdir")?;
        let dist = session.scalar("dist_subdir")?;
        let mirror = if session.invoke("exists", &["archive_sites.mirror_subdir"])? == "1" {
            session.scalar("archive_sites.mirror_subdir")?
        } else {
            String::new()
        };
        let mut urls = Vec::new();
        for reference in &references {
            ensure!(
                !reference.contains("://"),
                "literal archive-site URLs require signature-type resolution"
            );
            let parts: Vec<_> = reference.split(':').collect();
            ensure!(
                parts.len() == 3 && parts[2] == "tbz2",
                "unsupported native archive-site reference {reference}"
            );
            let site = self
                .get(parts[0])
                .ok_or_else(|| anyhow::anyhow!("unknown archive mirror class {}", parts[0]))?;
            let subdir = format!("{}{mirror}", parts[1]);
            let request = site.signify_request(
                target,
                item,
                &Subdirectories {
                    port_name: &name,
                    mirror: &subdir,
                    archive: &archive,
                    dist: &dist,
                },
                max_bytes,
            )?;
            urls.extend(request.urls);
        }
        Ok(PreparedRequest {
            request: Request {
                name: item.port.name.clone(),
                urls,
                max_bytes,
            },
            references,
        })
    }
}

impl Site {
    /// Build a request without fetching data or reading trust keys. Other archive
    /// and signature formats require their respective native transport support.
    pub fn signify_request(
        &self,
        target: &Target,
        item: &Item<'_>,
        subdirs: &Subdirectories<'_>,
        max_bytes: u64,
    ) -> Result<Request> {
        ensure!(
            self.archive_type == "tbz2",
            "unsupported native archive type {}",
            self.archive_type
        );
        ensure!(
            self.signature_type == "sig",
            "unsupported native archive signature type {}",
            self.signature_type
        );
        ensure!(max_bytes > 0, "archive size limit must be positive");
        ensure!(
            !self.urls.is_empty(),
            "archive site {} has no mirrors",
            self.name
        );
        let filename = target.filename(item)?;
        let urls = self
            .urls
            .iter()
            .map(|mirror| {
                let mut base =
                    resolve_mirror(mirror, subdirs.mirror, subdirs.port_name, subdirs.dist);
                if !base.ends_with('/') {
                    base.push('/');
                }
                base.push_str(subdirs.archive);
                if !base.ends_with('/') {
                    base.push('/');
                }
                let mut url = reqwest::Url::parse(&base)?;
                ensure!(
                    matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
                    "archive URLs require HTTP(S)"
                );
                ensure!(
                    url.username().is_empty()
                        && url.password().is_none()
                        && url.query().is_none()
                        && url.fragment().is_none(),
                    "archive URLs cannot contain credentials, queries, or fragments"
                );
                url.path_segments_mut()
                    .expect("HTTP URL has path segments")
                    .pop_if_empty()
                    .push(&filename);
                Ok(url.into())
            })
            .collect::<Result<_>>()?;
        Ok(Request {
            name: item.port.name.clone(),
            urls,
            max_bytes,
        })
    }
}

/// Match portlib's separate_tag and resolve_mirror_tags, including first-only
/// $name substitution and literal concatenation of mirror subdirectories.
pub fn resolve_mirror(element: &str, subdir: &str, name: &str, dist_subdir: &str) -> String {
    let after_scheme = element.find(':').map_or(2, |index| index + 3);
    let slash = element
        .get(after_scheme..)
        .and_then(|tail| tail.find('/'))
        .map(|index| after_scheme + index);
    let (element, mut tag) = match (slash, element.rfind(':')) {
        (Some(slash), Some(colon)) if slash < colon => (&element[..colon], &element[colon + 1..]),
        _ => (element, ""),
    };
    static NAME: OnceLock<regex::Regex> = OnceLock::new();
    let pattern = NAME.get_or_init(|| regex::Regex::new(r"\$(?:name\b|\{name\})").unwrap());
    let embedded = pattern.find(element);
    if embedded.is_some() {
        tag = "";
    }
    let directory = if tag == "mirror" {
        dist_subdir
    } else if subdir.is_empty() && tag != "nosubdir" {
        name
    } else {
        subdir
    };
    if let Some(found) = embedded {
        format!(
            "{}{}{}",
            &element[..found.start()],
            directory,
            &element[found.end()..]
        )
    } else {
        format!("{element}{directory}")
    }
}
