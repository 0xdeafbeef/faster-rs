#[test]
fn auto_compaction_does_not_abort_during_checkpoint_phases() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_compaction_start_session_race"))
        .output()
        .expect("run race reproducer");

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "race reproducer failed: status={}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status
        );
    }
}

