//! Package a staged destroot into an upstream-compatible `.tbz2` port image
//! (`portinstall::create_archive`), including its +CONTENTS inventory.
use anyhow::{Context, Result, ensure};
use md5::{Digest, Md5};
use std::{
    fmt::Write as _,
    fs,
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
};

/// Values the Portfile session supplies for the image's control files.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub destroot: PathBuf,
    pub statefile: PathBuf,
    pub portfile: PathBuf,
    pub subport: String,
    pub version: String,
    pub revision: u64,
    pub epoch: u64,
    /// Enabled variants, without the `+` prefix.
    pub variants: Vec<String>,
    /// `get_canonical_archs`, space separated.
    pub archs: String,
    /// `name-version_revision` for each resolvable library/runtime dependency.
    pub dependencies: Vec<String>,
    pub description: Option<String>,
    pub long_description: Option<String>,
    pub homepage: Option<String>,
    pub platform: Option<Platform>,
    pub ports_commit: Option<String>,
    pub source_date_epoch: String,
    pub macports_version: String,
    pub configured_cxx_stdlib: String,
    pub default_cxx_stdlib: String,
}

/// Platform stamp written for platform-specific images.
#[derive(Debug, Clone, Default)]
pub struct Platform {
    pub subplatform: String,
    pub os_version: Option<String>,
    pub macos_version: Option<String>,
    /// `("xcodeversion" | "xcodecltversion", value)` when known.
    pub toolchain: Option<(String, String)>,
}

/// Write the archive to `output`. The destroot keeps no control files afterwards.
pub fn create(inputs: &Inputs, output: &Path) -> Result<()> {
    let destroot = &inputs.destroot;
    ensure!(
        destroot.is_dir(),
        "no destroot found at: {}",
        destroot.display()
    );
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let result = (|| -> Result<()> {
        fs::copy(&inputs.statefile, destroot.join("+STATE"))
            .with_context(|| format!("cannot copy {}", inputs.statefile.display()))?;
        fs::copy(&inputs.portfile, destroot.join("+PORTFILE"))
            .with_context(|| format!("cannot copy {}", inputs.portfile.display()))?;
        let mut comment = String::new();
        if let Some(description) = &inputs.description {
            writeln!(comment, "{description}")?;
        }
        fs::write(destroot.join("+COMMENT"), comment)?;
        let mut desc = String::new();
        if let Some(long) = &inputs.long_description {
            writeln!(desc, "{long}")?;
        }
        if let Some(homepage) = &inputs.homepage {
            writeln!(desc, "\nWWW: {homepage}")?;
        }
        fs::write(destroot.join("+DESC"), desc)?;
        let contents = contents(inputs)?;
        fs::write(destroot.join("+CONTENTS"), contents)?;
        write_archive(destroot, output)
    })();
    for entry in fs::read_dir(destroot)?.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('+') && entry.path().is_file() {
            fs::remove_file(entry.path())?;
        }
    }
    if result.is_err() {
        let _ = fs::remove_file(output);
    }
    result
}

fn walk(root: &Path, directory: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(directory)?.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            walk(root, &path, out)?;
        } else {
            out.push(path.strip_prefix(root)?.to_path_buf());
        }
    }
    Ok(())
}

fn md5_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Md5::new();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        }))
}

fn contents(inputs: &Inputs) -> Result<String> {
    let mut text = String::new();
    let mut variants = inputs.variants.clone();
    variants.sort();
    let suffix: String = variants.iter().map(|v| format!("+{v}")).collect();
    writeln!(
        text,
        "@name {}-{}_{}{suffix}",
        inputs.subport, inputs.version, inputs.revision
    )?;
    writeln!(text, "@portname {}", inputs.subport)?;
    writeln!(text, "@portepoch {}", inputs.epoch)?;
    writeln!(text, "@portversion {}", inputs.version)?;
    writeln!(text, "@portrevision {}", inputs.revision)?;
    writeln!(text, "@archs {}", inputs.archs)?;
    for variant in variants {
        writeln!(text, "@portvariant +{variant}")?;
    }
    for dependency in &inputs.dependencies {
        writeln!(text, "@pkgdep {dependency}")?;
    }
    writeln!(text, "@macports_version {}", inputs.macports_version)?;
    if let Some(platform) = &inputs.platform {
        if !platform.subplatform.is_empty() {
            writeln!(text, "@os.subplatform {}", platform.subplatform)?;
        }
        if let Some(version) = &platform.os_version {
            writeln!(text, "@os.version {version}")?;
            if let Some(macos) = &platform.macos_version {
                writeln!(text, "@macos_version {macos}")?;
            }
            if let Some((key, value)) = &platform.toolchain {
                writeln!(text, "@{key} {value}")?;
            }
        }
    }
    if let Some(commit) = &inputs.ports_commit {
        writeln!(text, "@ports_commit {commit}")?;
    }
    writeln!(text, "@source_date_epoch {}", inputs.source_date_epoch)?;
    let mut paths = Vec::new();
    walk(&inputs.destroot, &inputs.destroot, &mut paths)?;
    let mut control = Vec::new();
    let mut binaries = Vec::new();
    for relative in paths {
        let name = relative.to_string_lossy().into_owned();
        if name.starts_with('+') {
            control.push(name);
            continue;
        }
        writeln!(text, "{name}")?;
        let full = inputs.destroot.join(&relative);
        if fs::symlink_metadata(&full)?.is_file() {
            writeln!(text, "@comment MD5:{}", md5_file(&full)?)?;
            let binary = crate::macho::is_binary(&mut fs::File::open(&full)?)?;
            if binary {
                binaries.push(full);
            }
            writeln!(text, "@comment binary:{}", u8::from(binary))?;
        }
    }
    for name in control {
        writeln!(text, "@ignore")?;
        writeln!(text, "{name}")?;
    }
    let actual = actual_cxx_stdlib(&binaries);
    writeln!(text, "@cxx_stdlib {actual}")?;
    let overridden = actual != "none" && inputs.configured_cxx_stdlib != inputs.default_cxx_stdlib;
    writeln!(text, "@cxx_stdlib_overridden {}", u8::from(overridden))?;
    Ok(text)
}

/// `macports::get_actual_cxx_stdlib`: which C++ runtime the binaries link.
pub fn actual_cxx_stdlib(binaries: &[PathBuf]) -> &'static str {
    let (mut libcxx, mut libstdcxx) = (false, false);
    for binary in binaries {
        let Ok(mut file) = fs::File::open(binary) else {
            continue;
        };
        let Ok(libraries) = crate::macho::linked_libraries(&mut file) else {
            continue;
        };
        for library in libraries {
            let name = library.rsplit('/').next().unwrap_or(&library);
            if name.starts_with("libc++") && name.ends_with(".dylib") {
                libcxx = true;
            } else if name.starts_with("libstdc++") && name.ends_with(".dylib") {
                libstdcxx = true;
            }
        }
    }
    match (libcxx, libstdcxx) {
        (true, true) => "mixed",
        (true, false) => "libc++",
        (false, true) => "libstdc++",
        (false, false) => "none",
    }
}

fn write_archive(destroot: &Path, output: &Path) -> Result<()> {
    let file =
        fs::File::create(output).with_context(|| format!("cannot create {}", output.display()))?;
    let encoder = bzip2::write::BzEncoder::new(BufWriter::new(file), bzip2::Compression::best());
    let mut builder = tar::Builder::new(encoder);
    let mut entries = vec![PathBuf::new()];
    collect(destroot, Path::new(""), &mut entries)?;
    for relative in entries {
        append(&mut builder, destroot, &relative)?;
    }
    let encoder = builder.into_inner()?;
    let mut writer = encoder.finish()?;
    writer.flush()?;
    writer
        .into_inner()
        .map_err(|err| err.into_error())?
        .sync_all()?;
    Ok(())
}

/// Every entry below `directory`, parents before children, in name order.
fn collect(root: &Path, directory: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut names: Vec<_> = fs::read_dir(root.join(directory))?
        .flatten()
        .map(|entry| entry.file_name())
        .collect();
    names.sort();
    for name in names {
        let relative = directory.join(&name);
        let is_dir = fs::symlink_metadata(root.join(&relative))?.is_dir();
        out.push(relative.clone());
        if is_dir {
            collect(root, &relative, out)?;
        }
    }
    Ok(())
}

/// A PAX extended-header record: `length key=value\n`, length counting itself.
fn pax_record(key: &str, value: &str) -> Vec<u8> {
    let body = format!(" {key}={value}\n");
    let mut length = body.len() + 1;
    while format!("{length}{body}").len() != length {
        length = format!("{length}{body}").len();
    }
    format!("{length}{body}").into_bytes()
}

fn append<W: Write>(builder: &mut tar::Builder<W>, root: &Path, relative: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let full = root.join(relative);
    let metadata = fs::symlink_metadata(&full)?;
    let mut name = format!("./{}", relative.to_string_lossy());
    let mut header = tar::Header::new_ustar();
    header.set_mode(metadata.mode() & 0o7777);
    header.set_uid(u64::from(metadata.uid()));
    header.set_gid(u64::from(metadata.gid()));
    header.set_mtime(u64::try_from(metadata.mtime()).unwrap_or(0));
    let mut pax = Vec::new();
    if metadata.is_dir() {
        if !name.ends_with('/') {
            name.push('/');
        }
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
    } else if metadata.file_type().is_symlink() {
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        let target = fs::read_link(&full)?.to_string_lossy().into_owned();
        if header.set_link_name(&target).is_err() || target.len() > 100 {
            pax.extend(pax_record("linkpath", &target));
            header.set_link_name(&target[target.len().saturating_sub(100)..])?;
        }
    } else {
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(metadata.len());
    }
    if header.set_path(&name).is_err() {
        pax.extend(pax_record("path", &name));
        let tail = &name[name.len().saturating_sub(99)..];
        header.set_path(tail.trim_start_matches('/'))?;
    }
    if !pax.is_empty() {
        let mut extension = tar::Header::new_ustar();
        extension.set_entry_type(tar::EntryType::XHeader);
        extension.set_path("PaxHeader")?;
        extension.set_mode(0o644);
        extension.set_size(pax.len() as u64);
        extension.set_cksum();
        builder.append(&extension, pax.as_slice())?;
    }
    header.set_cksum();
    if metadata.is_file() {
        builder.append(&header, fs::File::open(&full)?)?;
    } else {
        builder.append(&header, std::io::empty())?;
    }
    Ok(())
}
