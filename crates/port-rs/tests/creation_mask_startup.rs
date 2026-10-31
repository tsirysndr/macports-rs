//! Run the real CLI so mask capture occurs in main before any runtime exists.
#![cfg(target_os = "macos")]
use std::{fs, os::unix::process::CommandExt, process::Command};

#[test]
fn cli_captures_inherited_mask_before_loading_portfiles_and_command_chains() {
    let tree = tempfile::tempdir().unwrap();
    fs::create_dir_all(tree.path().join("test/app")).unwrap();
    fs::write(
        tree.path().join("test/app/Portfile"),
        r#"
        PortSystem 1.0
        name app
        version 1
        variant snapshot description "initial mask ${port_rs.initial_creation_mask}" {}
    "#,
    )
    .unwrap();
    let record = "name app version 1 portdir test/app variants snapshot";
    fs::write(
        tree.path().join("PortIndex"),
        format!("app {}\n{record}\n", record.len() + 1),
    )
    .unwrap();
    for mask in [0, 0o027, 0o077] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_port"));
        command.env("PORT_RS_INDEX", tree.path().join("PortIndex"));
        command.env("PATH", "");
        command
            .current_dir(tree.path())
            .args(["variants", "app", ";", "variants", "app"]);
        // SAFETY: runs only in the newly forked child, before exec; umask is a
        // leaf OS call and cannot change the test runner's process mask.
        unsafe {
            command.pre_exec(move || {
                libc::umask(mask);
                Ok(())
            });
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            text.matches(&format!("initial mask {mask:04o}")).count(),
            2,
            "{text}"
        );
    }
}
