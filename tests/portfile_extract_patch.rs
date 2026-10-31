use port_rs::{
    build,
    portfile::{PortGroups, Session},
    progress::Progress,
    spec::PortSpec,
};
use std::{collections::BTreeMap, fs, os::unix::process::ExitStatusExt, path::Path, process};

fn sh(directory: &Path, script: &str) {
    assert!(
        process::Command::new("/bin/sh")
            .args(["-c", script])
            .current_dir(directory)
            .status()
            .unwrap()
            .success(),
        "{script}"
    );
}

fn session(root: &Path, body: &str) -> Session {
    let mut s = Session::open(
        &format!(
            "name app; version 1; license MIT; {body}
            proc portfetch::get_full_mirror_sites_path {{}} {{return /nonexistent/mirrors.tcl}}"
        ),
        &BTreeMap::from([
            (
                "portdbpath".into(),
                root.join("db").to_str().unwrap().into(),
            ),
            (
                "portpath".into(),
                root.join("port").to_str().unwrap().into(),
            ),
            ("os.platform".into(), "darwin".into()),
            ("os.major".into(), "24".into()),
            (
                "prefix".into(),
                root.join("prefix").to_str().unwrap().into(),
            ),
        ]),
        &PortSpec::named("app"),
        PortGroups::default(),
    )
    .unwrap();
    let base = root.to_owned();
    s.enable_system(
        &BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        move |request| {
            let command = request.into_command(&base)?;
            let status = process::Command::new(&command.program)
                .args(&command.args)
                .current_dir(&command.directory)
                .env_clear()
                .envs(&command.environment)
                .status()?;
            Ok(build::ProcessResult {
                pid: 1,
                status: process::ExitStatus::from_raw(status.into_raw()),
            })
        },
    )
    .unwrap();
    s.set_progress(Progress::new(true));
    for (name, value) in [
        ("configure.sdkroot", ""),
        ("macosx_deployment_target", ""),
        ("source_date_epoch", "0"),
        (
            "configure.developer_dir",
            "/Library/Developer/CommandLineTools",
        ),
    ] {
        s.invoke("set", &[name, value]).unwrap();
    }
    s
}

fn prepare(root: &Path, suffix: &str, compress: &str) {
    let source = root.join("src/app-1");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("hello.txt"), "hello\n").unwrap();
    let dist = root.join("db/distfiles/app");
    fs::create_dir_all(&dist).unwrap();
    fs::create_dir_all(root.join("port/files")).unwrap();
    fs::write(root.join("port/Portfile"), "name app\n").unwrap();
    sh(
        &root.join("src"),
        &format!(
            "tar -cf - app-1 | {compress} > '{}/app-1{suffix}'",
            dist.display()
        ),
    );
}

fn run(s: &mut Session, phase: &str) {
    let item = s.scalar(&format!("org.macports.{phase}")).unwrap();
    s.run_prepared_target(&item, false).unwrap();
}

#[test]
fn extracts_gzip_and_bzip2_distfiles_into_the_work_directory() {
    for (option, suffix, compress) in [
        ("", ".tar.gz", "gzip"),
        ("use_bzip2 yes", ".tar.bz2", "bzip2"),
    ] {
        let root = tempfile::tempdir().unwrap();
        prepare(root.path(), suffix, compress);
        let mut s = session(root.path(), option);
        for phase in ["main", "fetch", "extract"] {
            s.invoke("set", &["checksum.skip", "yes"]).unwrap();
            run(&mut s, phase);
        }
        let worksrc = s.scalar("worksrcpath").unwrap();
        assert_eq!(
            fs::read_to_string(Path::new(&worksrc).join("hello.txt")).unwrap(),
            "hello\n"
        );
    }
}

#[test]
fn applies_plain_and_compressed_patches_from_filespath() {
    let root = tempfile::tempdir().unwrap();
    prepare(root.path(), ".tar.gz", "gzip");
    let patch = "--- hello.txt\n+++ hello.txt\n@@ -1 +1 @@\n-hello\n+patched\n";
    let second = "--- hello.txt\n+++ hello.txt\n@@ -1 +1 @@\n-patched\n+twice\n";
    let files = root.path().join("port/files");
    fs::write(files.join("one.diff"), patch).unwrap();
    fs::write(files.join("two.diff"), second).unwrap();
    sh(&files, "gzip two.diff");
    let mut s = session(root.path(), "patchfiles one.diff two.diff.gz");
    for phase in ["main", "fetch", "extract", "patch"] {
        run(&mut s, phase);
    }
    let worksrc = s.scalar("worksrcpath").unwrap();
    assert_eq!(
        fs::read_to_string(Path::new(&worksrc).join("hello.txt")).unwrap(),
        "twice\n"
    );
    let root = tempfile::tempdir().unwrap();
    prepare(root.path(), ".tar.gz", "gzip");
    let mut s = session(root.path(), "patchfiles missing.diff");
    let item = s.scalar("org.macports.patch").unwrap();
    let error = s.run_prepared_target(&item, false).unwrap_err().to_string();
    assert!(
        error.contains("Patch file missing.diff is missing"),
        "{error}"
    );
}
