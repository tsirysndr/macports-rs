//! `port pkg`, `dmg`, `mpkg` and `mdmg` (portpkg, portdmg, portmpkg,
//! portmdmg): macOS installer packages and disk images from a port's destroot,
//! as flat packages built with pkgbuild(1) and productbuild(1).
use super::local::Host;
use crate::{
    index::Index,
    install::{self, Item},
    portfile::Session,
    query,
    source::Platform,
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `name-version`, with `_revision` when the revision is not 0.
fn image_name(name: &str, version: &str, revision: &str) -> String {
    if revision == "0" || revision.is_empty() {
        format!("{name}-{version}")
    } else {
        format!("{name}-{version}_{revision}")
    }
}

/// portpkg::mp_version_to_apple_version: epoch, seven version numbers
/// (trailing letters as base 26), then the revision.
fn apple_version(epoch: &str, version: &str, revision: &str) -> String {
    let mut text: String = version
        .chars()
        .map(|c| {
            if c == '.' || c.is_ascii_digit() {
                c
            } else {
                '.'
            }
        })
        .collect();
    while text.contains("..") {
        text = text.replace("..", ".");
    }
    let text = text.trim_end_matches('.');
    let mut parts: Vec<String> = text.split('.').map(str::to_string).collect();
    let letters = regex::Regex::new(r"\d([a-zA-Z]+)").expect("static pattern");
    if let Some(captures) = letters.captures(version) {
        let mut value: u64 = 0;
        for c in captures[1].chars() {
            let ordinal = c.to_ascii_lowercase() as u64 - 'a' as u64 + 1;
            value = 26 * value + ordinal;
        }
        parts.push(value.to_string());
    }
    while parts.len() < 7 {
        parts.push("0".into());
    }
    let epoch = if epoch.is_empty() { "0" } else { epoch };
    let revision = if revision.is_empty() { "0" } else { revision };
    std::iter::once(epoch.to_string())
        .chain(parts)
        .chain(std::iter::once(revision.to_string()))
        .collect::<Vec<_>>()
        .join(".")
}

fn value(session: &mut Session, name: &str) -> String {
    session.scalar(name).unwrap_or_default()
}

fn truthy(text: &str) -> bool {
    matches!(text, "1" | "yes" | "true" | "on")
}

fn run(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("cannot run {:?}", command.get_program()))?;
    ensure!(
        status.success(),
        "{:?} failed: {status}",
        command.get_program()
    );
    Ok(())
}

fn welcome_html(session: &mut Session) -> String {
    let name = xml_escape(&value(session, "subport"));
    let version = xml_escape(&value(session, "version"));
    let revision = value(session, "revision");
    let revision = if revision == "0" {
        String::new()
    } else {
        format!("_{}", xml_escape(&revision))
    };
    let mut long_description = value(session, "long_description");
    if long_description.is_empty() {
        long_description = value(session, "description");
    }
    let homepage = xml_escape(&value(session, "homepage"));
    let mut html = format!(
        "\n<html lang=\"en\">\n<head>\n    <meta http-equiv=\"content-type\" content=\"text/html; charset=iso-8859-1\">\n    <title>Install {name}</title>\n</head>\n<body>\n<font face=\"Helvetica\"><b>Welcome to the {name} for macOS Installer</b></font>\n<p>\n<font face=\"Helvetica\">{}</font>\n<p>\n",
        xml_escape(&long_description)
    );
    if !homepage.is_empty() {
        html.push_str(&format!(
            "<font face=\"Helvetica\"><a href=\"{homepage}\">{homepage}</a></font><p>\n"
        ));
    }
    html.push_str(&format!(
        "<font face=\"Helvetica\">This installer guides you through the steps necessary to install {name} {version}{revision} for macOS. To get started, click Continue.</font>\n</body>\n</html>\n"
    ));
    html
}

fn host_architectures(session: &mut Session) -> String {
    if value(session, "supported_archs") == "noarch" {
        return " hostArchitectures=\"arm64,i386,ppc,x86_64\"".into();
    }
    let universal = session
        .invoke("variant_isset", &["universal"])
        .is_ok_and(|set| set == "1");
    let mut archs =
        crate::tcl_list::parse(&value(session, "configure.universal_archs")).unwrap_or_default();
    if universal && archs.len() >= 2 {
        archs.sort();
        return format!(" hostArchitectures=\"{}\"", archs.join(","));
    }
    let build = value(session, "configure.build_arch");
    if build.is_empty() {
        String::new()
    } else {
        format!(" hostArchitectures=\"{build}\"")
    }
}

/// Resources shared by pkg and mpkg: Welcome.html and the background image.
fn write_resources(session: &mut Session, root: &Path, directory: &Path) -> Result<()> {
    fs::create_dir_all(directory)?;
    fs::write(directory.join("Welcome.html"), welcome_html(session))?;
    let background = root.join("_resources/port1.0/package/background.tiff");
    if background.is_file() {
        fs::copy(background, directory.join("background.tiff"))?;
    }
    Ok(())
}

fn info_plist(name: &str, version: &str, revision: &str) -> String {
    let (name, version, revision) = (xml_escape(name), xml_escape(version), xml_escape(revision));
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n    \n<dict>\n    <key>CFBundleGetInfoString</key>\n    <string>{name} {version}</string>\n    <key>CFBundleIdentifier</key>\n    <string>org.macports.{name}</string>\n    <key>CFBundleName</key>\n    <string>{name}</string>\n    <key>CFBundleShortVersionString</key>\n    <string>{version}</string>\n    <key>IFMajorVersion</key>\n    <integer>{revision}</integer>\n    <key>IFMinorVersion</key>\n    <integer>0</integer>\n    <key>IFPkgFlagAllowBackRev</key>\n    <true/>\n    <key>IFPkgFlagAuthorizationAction</key>\n    <string>RootAuthorization</string>\n    <key>IFPkgFlagDefaultLocation</key>\n    <string>/</string>\n    <key>IFPkgFlagInstallFat</key>\n    <false/>\n    <key>IFPkgFlagIsRequired</key>\n    <false/>\n    <key>IFPkgFlagRelocatable</key>\n    <false/>\n    <key>IFPkgFlagRestartAction</key>\n    <string>NoRestart</string>\n    <key>IFPkgFlagRootVolumeOnly</key>\n    <true/>\n    <key>IFPkgFlagUpdateInstalledLanguages</key>\n    <false/>\n    <key>IFPkgFormatVersion</key>\n    <real>0.10000000149011612</real>\n</dict>\n</plist>\n"
    )
}

fn description_plist(name: &str, version: &str, description: &str) -> String {
    let (name, version, description) = (
        xml_escape(name),
        xml_escape(version),
        xml_escape(description),
    );
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n    \n<dict>\n    <key>IFPkgDescriptionDeleteWarning</key>\n    <string></string>\n    <key>IFPkgDescriptionDescription</key>\n    <string>{description}</string>\n    <key>IFPkgDescriptionTitle</key>\n    <string>{name}</string>\n    <key>IFPkgDescriptionVersion</key>\n    <string>{version}</string>\n</dict>\n</plist>\n"
    )
}

/// Total size of the regular files below `path`.
fn directory_size(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_dir() {
        fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| directory_size(&entry.path()))
            .sum()
    } else {
        metadata.len()
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    run(Command::new("/bin/cp").arg("-PR").arg(from).arg(to))
}

/// The pre-flat `.pkg` bundle (portpkg without PackageMaker): a BOM, a
/// cpio payload and plists under `Contents`.
fn package_bundle(
    session: &mut Session,
    pkgpath: &Path,
    destpath: &Path,
    resources: &Path,
) -> Result<()> {
    let name = value(session, "subport");
    let version = value(session, "version");
    let revision = value(session, "revision");
    let contents = pkgpath.join("Contents");
    let _ = fs::remove_dir_all(pkgpath);
    fs::create_dir_all(contents.join("Resources"))?;
    for entry in fs::read_dir(resources)?.flatten() {
        copy_tree(&entry.path(), &contents.join("Resources"))?;
    }
    fs::write(contents.join("PkgInfo"), "pmkrpkg1\n")?;
    fs::write(
        contents.join("Info.plist"),
        info_plist(&name, &version, &revision),
    )?;
    run(Command::new("/usr/bin/mkbom")
        .arg(destpath)
        .arg(contents.join("Archive.bom")))?;
    let payload = fs::File::create(contents.join("Archive.pax.gz"))?;
    run(Command::new("/bin/pax")
        .args(["-x", "cpio", "-w", "-z", "."])
        .current_dir(destpath)
        .stdout(payload))?;
    fs::write(
        contents.join("Resources/Description.plist"),
        description_plist(&name, &version, &value(session, "description")),
    )?;
    let listing = Command::new("/usr/bin/lsbom")
        .arg("-s")
        .arg(contents.join("Archive.bom"))
        .output()
        .context("Reading package bom failed")?;
    let files = String::from_utf8_lossy(&listing.stdout).lines().count();
    let installed = directory_size(destpath) / 1024
        + fs::metadata(contents.join("Info.plist"))?.len()
        + fs::metadata(contents.join("Archive.bom"))?.len();
    fs::write(
        contents.join("Resources/Archive.sizes"),
        format!(
            "NumFiles {files}\nInstalledSize {installed}\nCompressedSize {}\n",
            directory_size(pkgpath) / 1024
        ),
    )?;
    Ok(())
}

/// Build the destroot from the port's archive, or from source.
fn destroot(
    host: &Host,
    session: &mut Session,
    item: &Item<'_>,
    machine: &Platform,
    worker: &install::mirror_worker::MirrorWorker,
) -> Result<()> {
    let label = super::phase::prepare(session, host, worker, &item.port.name)?;
    if !super::archivefetch::unpack(host, session, item, machine)? {
        super::phase::run_targets(session, &label, &super::phase::ORDER)?;
    }
    host.progress.close_log();
    Ok(())
}

/// portpkg::package_pkg; returns the package path.
fn package_pkg(
    host: &Host,
    session: &mut Session,
    root: &Path,
    portfile: &Path,
) -> Result<PathBuf> {
    let flat = truthy(&value(session, "package.flat"));
    let name = value(session, "subport");
    let (epoch, version, revision) = (
        value(session, "epoch"),
        value(session, "version"),
        value(session, "revision"),
    );
    let workpath = PathBuf::from(value(session, "workpath"));
    let destpath = PathBuf::from(value(session, "destroot"));
    let package_destpath = PathBuf::from(value(session, "package.destpath"));
    fs::create_dir_all(&package_destpath)?;
    let image = image_name(&name, &version, &revision);
    let pkgpath = package_destpath.join(format!("{image}.pkg"));
    host.progress.phase(&format!(
        "Creating pkg for {name} version {epoch}_{version}_{revision} at {}",
        pkgpath.display()
    ));
    let newer = |path: &Path| {
        let modified = |path: &Path| fs::metadata(path).and_then(|m| m.modified()).ok();
        matches!((modified(path), modified(portfile)), (Some(a), Some(b)) if a >= b)
    };
    if newer(&pkgpath) {
        host.progress.phase(&format!(
            "Package for {name} version {epoch}_{version}_{revision} at {} is up-to-date",
            pkgpath.display()
        ));
        return Ok(pkgpath);
    }
    // Toplevel etc, var and tmp are symlinks into /private on macOS.
    let private = destpath.join("private");
    let mut moved = Vec::new();
    for directory in ["etc", "var", "tmp"] {
        let path = destpath.join(directory);
        if path.is_dir() {
            fs::create_dir_all(&private)?;
            fs::rename(&path, private.join(directory))?;
            moved.push(directory);
        }
    }
    let prefix = value(session, "prefix");
    for file in
        crate::tcl_list::parse(&value(session, "pkg_post_unarchive_deletions")).unwrap_or_default()
    {
        let path = destpath.join(prefix.trim_start_matches('/')).join(&file);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
            fs::remove_dir_all(&path)?;
        } else if path.exists() {
            fs::remove_file(&path)?;
        }
    }
    let resources = PathBuf::from(value(session, "package.resources"));
    let scripts = PathBuf::from(value(session, "package.scripts"));
    write_resources(session, root, &resources.join("English.lproj"))?;
    if !flat {
        let result = package_bundle(session, &pkgpath, &destpath, &resources);
        let _ = fs::remove_dir_all(&resources);
        for directory in moved {
            fs::rename(private.join(directory), destpath.join(directory))?;
        }
        let _ = fs::remove_dir(&private);
        return result.map(|()| pkgpath);
    }
    fs::create_dir_all(&scripts)?;
    let image_escaped = xml_escape(&image);
    let name_escaped = xml_escape(&name);
    let deployment = value(session, "macosx_deployment_target");
    let distribution = workpath.join("Distribution");
    fs::write(
        &distribution,
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<installer-gui-script minSpecVersion=\"1\">\n    <title>{name_escaped}</title>\n    <options customize=\"never\"{}/>\n    <allowed-os-versions><os-version min=\"{deployment}\"/></allowed-os-versions>\n    <background file=\"background.tiff\" mime-type=\"image/tiff\" alignment=\"bottomleft\" scaling=\"none\"/>\n    <welcome mime-type=\"text/html\" file=\"Welcome.html\"/>\n    <choices-outline>\n        <line choice=\"default\">\n            <line choice=\"org.macports.{name_escaped}\"/>\n        </line>\n    </choices-outline>\n    <choice id=\"default\"/>\n    <choice id=\"org.macports.{name_escaped}\" visible=\"false\">\n        <pkg-ref id=\"org.macports.{name_escaped}\"/>\n    </choice>\n    <pkg-ref id=\"org.macports.{name_escaped}\">{image_escaped}-component.pkg</pkg-ref>\n</installer-gui-script>\n\n",
            host_architectures(session)
        ),
    )?;
    let info = workpath.join("PackageInfo");
    fs::write(
        &info,
        "\n<pkg-info install-location=\"/\" relocatable=\"false\" auth=\"root\">\n</pkg-info>\n",
    )?;
    let apple = apple_version(&epoch, &version, &revision);
    let component = package_destpath.join(format!("{image}-component.pkg"));
    let result = (|| -> Result<()> {
        run(Command::new("/usr/bin/pkgbuild")
            .arg("--root")
            .arg(&destpath)
            .arg("--scripts")
            .arg(&scripts)
            .arg("--info")
            .arg(&info)
            .args(["--install-location", "/", "--identifier"])
            .arg(format!("org.macports.{name}"))
            .args(["--version", &apple])
            .arg(&component))?;
        run(Command::new("/usr/bin/productbuild")
            .arg("--resources")
            .arg(&resources)
            .arg("--identifier")
            .arg(format!("org.macports.{name}"))
            .arg("--distribution")
            .arg(&distribution)
            .arg("--package-path")
            .arg(&package_destpath)
            .arg(&pkgpath))
    })();
    let _ = fs::remove_file(&info);
    let _ = fs::remove_file(&distribution);
    let _ = fs::remove_dir_all(&resources);
    let _ = fs::remove_dir_all(&scripts);
    for directory in moved {
        fs::rename(private.join(directory), destpath.join(directory))?;
    }
    let _ = fs::remove_dir(&private);
    result?;
    Ok(pkgpath)
}

/// portdmg::package_dmg for a .pkg or .mpkg.
fn package_dmg(
    host: &Host,
    source: &Path,
    image: &str,
    destination: &Path,
    portfile: &Path,
) -> Result<()> {
    let final_image = destination.join(format!("{image}.dmg"));
    let modified = |path: &Path| fs::metadata(path).and_then(|m| m.modified()).ok();
    if matches!((modified(&final_image), modified(portfile)), (Some(a), Some(b)) if a >= b) {
        host.progress
            .phase(&format!("Disk Image for {image} is up-to-date"));
        return Ok(());
    }
    let folder = if source.is_dir() {
        source.to_path_buf()
    } else {
        let folder = destination.join(image);
        fs::create_dir_all(&folder)?;
        fs::copy(
            source,
            folder.join(source.file_name().context("package has no name")?),
        )?;
        folder
    };
    let temporary = destination.join(format!("{image}.tmp.dmg"));
    let _ = fs::remove_file(&temporary);
    let _ = fs::remove_file(&final_image);
    run(Command::new("/usr/bin/hdiutil")
        .args([
            "create",
            "-quiet",
            "-fs",
            "HFS+",
            "-volname",
            image,
            "-srcfolder",
        ])
        .arg(&folder)
        .arg(&temporary))
    .with_context(|| format!("Failed to create temporary image: {image}"))?;
    run(Command::new("/usr/bin/hdiutil")
        .arg("convert")
        .arg(&temporary)
        .args(["-format", "UDCO", "-o"])
        .arg(&final_image)
        .arg("-quiet"))
    .with_context(|| {
        format!(
            "Failed to convert to final image: {}",
            final_image.display()
        )
    })?;
    let _ = fs::remove_file(&temporary);
    Ok(())
}

/// pkg for each runtime dependency into `packages`, dependencies first.
fn dependency_packages(
    host: &Host,
    index: &Index,
    root: &Path,
    item: &Item<'_>,
    packages: &Path,
    machine: &Platform,
    worker: &install::mirror_worker::MirrorWorker,
) -> Result<Vec<(String, String)>> {
    let spec = crate::spec::PortSpec::named(&item.port.name);
    let registry = host.context.open_registry(None)?;
    let plan = install::plan_sessions(index, &[spec], registry.as_ref(), |port, spec| {
        host.open(root, port, spec)
    })?;
    drop(registry);
    let install::SessionPlan { plan, mut sessions } = plan;
    let mut result = Vec::new();
    for dependency in plan
        .items()
        .filter(|other| other.port.name != item.port.name)
    {
        let session = sessions
            .get_mut(&dependency.port.name)
            .with_context(|| format!("{}: no evaluated session", dependency.port.name))?;
        session.invoke("set", &["package.destpath", &packages.to_string_lossy()])?;
        destroot(host, session, &dependency, machine, worker)?;
        let portfile = root.join(dependency.port.get("portdir")).join("Portfile");
        package_pkg(host, session, root, &portfile)?;
        let image = image_name(
            &dependency.port.name,
            &value(session, "version"),
            &value(session, "revision"),
        );
        let package = if truthy(&value(session, "package.flat")) {
            format!("{image}-component.pkg")
        } else {
            format!("{image}.pkg")
        };
        result.push((format!("org.macports.{}", dependency.port.name), package));
    }
    Ok(result)
}

/// portmpkg::package_mpkg; returns the meta-package path.
#[allow(clippy::too_many_arguments)]
fn package_mpkg(
    host: &Host,
    index: &Index,
    root: &Path,
    session: &mut Session,
    item: &Item<'_>,
    machine: &Platform,
    worker: &install::mirror_worker::MirrorWorker,
    portfile: &Path,
) -> Result<PathBuf> {
    let name = value(session, "subport");
    let (epoch, version, revision) = (
        value(session, "epoch"),
        value(session, "version"),
        value(session, "revision"),
    );
    let workpath = PathBuf::from(value(session, "workpath"));
    let destination = PathBuf::from(value(session, "package.destpath"));
    let image = image_name(&name, &version, &revision);
    let mpkgpath = destination.join(format!("{image}.mpkg"));
    if !truthy(&value(session, "package.flat")) {
        // The pre-flat meta-package: component bundles under Contents/Packages.
        let _ = fs::remove_dir_all(&mpkgpath);
        let packages = mpkgpath.join("Contents/Packages");
        let resources = mpkgpath.join("Contents/Resources");
        fs::create_dir_all(&packages)?;
        let mut dependencies =
            dependency_packages(host, index, root, item, &packages, machine, worker)?;
        let own = package_pkg(host, session, root, portfile)?;
        copy_tree(&own, &packages)?;
        dependencies.push((format!("org.macports.{name}"), format!("{image}.pkg")));
        write_resources(session, root, &resources)?;
        fs::write(mpkgpath.join("Contents/PkgInfo"), "pmkrpkg1\n")?;
        fs::write(
            resources.join("Description.plist"),
            description_plist(&name, &version, &value(session, "description")),
        )?;
        let mut packages_xml = String::new();
        for (_, package) in &dependencies {
            packages_xml.push_str(&format!(
                "<dict>\n            <key>IFPkgFlagPackageLocation</key>\n            <string>{}</string>\n            <key>IFPkgFlagPackageSelection</key>\n            <string>selected</string>\n        </dict>\n        ",
                xml_escape(package)
            ));
        }
        let (name_escaped, version_escaped) = (xml_escape(&name), xml_escape(&version));
        fs::write(
            mpkgpath.join("Contents/Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n\n<dict>\n    <key>CFBundleGetInfoString</key>\n    <string>{name_escaped} {version_escaped}</string>\n    <key>CFBundleIdentifier</key>\n    <string>org.macports.mpkg.{name_escaped}</string>\n    <key>CFBundleName</key>\n    <string>{name_escaped}</string>\n    <key>CFBundleShortVersionString</key>\n    <string>{version_escaped}</string>\n    <key>IFMajorVersion</key>\n    <integer>{}</integer>\n    <key>IFMinorVersion</key>\n    <integer>0</integer>\n    <key>IFPkgFlagComponentDirectory</key>\n    <string>./Contents/Packages</string>\n    <key>IFPkgFlagPackageList</key>\n    <array>\n        {packages_xml}</array>\n    <key>IFPkgFormatVersion</key>\n    <real>0.10000000149011612</real>\n</dict>\n</plist>\n",
                xml_escape(&revision)
            ),
        )?;
        let _ = epoch;
        return Ok(mpkgpath);
    }
    let packages = workpath.join("mpkg_packages");
    let resources = workpath.join("mpkg_resources");
    fs::create_dir_all(&packages)?;
    let mut dependencies =
        dependency_packages(host, index, root, item, &packages, machine, worker)?;
    package_pkg(host, session, root, portfile)?;
    let component = destination.join(format!("{image}-component.pkg"));
    run(Command::new("/bin/cp")
        .arg("-PR")
        .arg(&component)
        .arg(&packages))?;
    dependencies.push((
        format!("org.macports.{name}"),
        format!("{image}-component.pkg"),
    ));
    write_resources(session, root, &resources)?;
    let name_escaped = xml_escape(&name);
    let deployment = value(session, "macosx_deployment_target");
    let mut distribution = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<installer-gui-script minSpecVersion=\"1\">\n    <title>{name_escaped}</title>\n    <options customize=\"never\"/>\n    <allowed-os-versions><os-version min=\"{deployment}\"/></allowed-os-versions>\n    <background file=\"background.tiff\" mime-type=\"image/tiff\" alignment=\"bottomleft\" scaling=\"none\"/>\n    <welcome mime-type=\"text/html\" file=\"Welcome.html\"/>\n    <choices-outline>\n    <line choice=\"default\">\n        <line choice=\"org.macports.mpkg.{name_escaped}\"/>\n    </line>\n    </choices-outline>\n    <choice id=\"default\"/>\n    <choice id=\"org.macports.mpkg.{name_escaped}\" visible=\"false\">\n\n"
    );
    for (identifier, _) in &dependencies {
        distribution.push_str(&format!(
            "        <pkg-ref id=\"{}\"/>\n",
            xml_escape(identifier)
        ));
    }
    distribution.push_str("    </choice>\n");
    for (identifier, package) in &dependencies {
        distribution.push_str(&format!(
            "    <pkg-ref id=\"{}\">{}</pkg-ref>\n",
            xml_escape(identifier),
            xml_escape(package)
        ));
    }
    distribution.push_str("</installer-gui-script>\n");
    let distribution_path = workpath.join("Distribution");
    fs::write(&distribution_path, distribution)?;
    let _ = fs::remove_file(&mpkgpath);
    run(Command::new("/usr/bin/productbuild")
        .arg("--resources")
        .arg(&resources)
        .arg("--identifier")
        .arg(format!("org.macports.mpkg.{name}"))
        .arg("--distribution")
        .arg(&distribution_path)
        .arg("--package-path")
        .arg(&packages)
        .args(["--version", &apple_version(&epoch, &version, &revision)])
        .arg(&mpkgpath))?;
    Ok(mpkgpath)
}

pub(super) fn run_target(
    out: &mut impl Write,
    index: &Index,
    ports: &[String],
    target: &str,
    quiet: bool,
    root: &Path,
) -> Result<()> {
    let _ = out;
    let host = Host::new(root, None, quiet)?;
    let machine = crate::source::detect_platform()?;
    let worker = install::mirror_worker::MirrorWorker::new(host.context.mirror_timings.clone(), 8)?;
    let install::SessionPlan { plan, mut sessions } =
        super::archivefetch::plan_requested(&host, index, ports, root)?;
    for item in plan.items().filter(|item| item.requested) {
        let session = sessions
            .get_mut(&item.port.name)
            .with_context(|| format!("{}: no evaluated session", item.port.name))?;
        let port = query::lookup(index, &item.port.name)?;
        let portfile = root.join(port.get("portdir")).join("Portfile");
        destroot(&host, session, &item, &machine, &worker)?;
        let name = value(session, "subport");
        let image = image_name(
            &name,
            &value(session, "version"),
            &value(session, "revision"),
        );
        let destination = PathBuf::from(value(session, "package.destpath"));
        match target {
            "pkg" => {
                package_pkg(&host, session, root, &portfile)?;
            }
            "dmg" => {
                let pkg = package_pkg(&host, session, root, &portfile)?;
                host.progress.phase(&format!(
                    "Creating disk image for {name}-{}_{}",
                    value(session, "version"),
                    value(session, "revision")
                ));
                package_dmg(&host, &pkg, &image, &destination, &portfile)?;
            }
            "mpkg" | "mdmg" => {
                let mpkg = package_mpkg(
                    &host, index, root, session, &item, &machine, &worker, &portfile,
                )?;
                if target == "mdmg" {
                    host.progress.phase(&format!(
                        "Creating disk image for {name}-{}",
                        value(session, "version")
                    ));
                    package_dmg(&host, &mpkg, &image, &destination, &portfile)?;
                }
            }
            other => bail!("unknown package target {other}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_versions_pad_to_seven_parts_with_letters_in_base_26() {
        assert_eq!(apple_version("0", "1.2.3", "4"), "0.1.2.3.0.0.0.0.4");
        assert_eq!(apple_version("1", "1.0.1c", "0"), "1.1.0.1.3.0.0.0.0");
        assert_eq!(apple_version("0", "2.4-beta", "0"), "0.2.4.0.0.0.0.0.0");
        assert_eq!(image_name("zlib", "1.3", "0"), "zlib-1.3");
        assert_eq!(image_name("zlib", "1.3", "2"), "zlib-1.3_2");
    }
}
