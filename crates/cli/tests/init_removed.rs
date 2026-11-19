//! `pylon init` is not a command. The CLI names the scaffolder to use and
//! writes no files.

use std::process::Command;

#[test]
fn init_points_to_the_scaffolder_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_pylon"))
        .args(["init", "my-app"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!out.status.success(), "init must fail");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("npm create @pylonsync/pylon@latest"),
        "the error must name the scaffolder, got: {stderr}"
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "init must not write files"
    );
}

#[test]
fn help_does_not_list_init() {
    let out = Command::new(env!("CARGO_BIN_EXE_pylon"))
        .arg("--help")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.lines().any(|l| l.trim_start().starts_with("init")),
        "help must not list an init command"
    );
}
