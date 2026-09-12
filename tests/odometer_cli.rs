use rustix::fs::{flock, FlockOperation};
use std::{fs, process::Command};

#[test]
fn reports_persistent_totals_while_another_process_holds_the_writer_lock() {
    let directory =
        std::env::temp_dir().join(format!("fievel-odometer-cli-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let state = directory.join("fievel");
    let report = || {
        Command::new(env!("CARGO_BIN_EXE_fievel"))
            .env("XDG_STATE_HOME", &directory)
            .env_remove("HOME")
            .args([
                "--odometer",
                "--config",
                "/nonexistent/fievel.config",
                "--device",
                "/nonexistent/input",
            ])
            .output()
            .unwrap()
    };

    let first = report();
    assert!(first.status.success(), "{:?}", first);
    let stdout = String::from_utf8(first.stdout).unwrap();
    assert!(stdout.contains("Fievel:                  0.000000 miles (estimated)"));
    assert!(stdout.contains("Physical mouse/trackpad:  0.000000 miles (estimated)"));
    assert!(
        !state.exists(),
        "reporting must not create state or acquire a lock"
    );

    fs::create_dir(&state).unwrap();
    let path = state.join("odometer.toml");
    let text = "fievel_input_units = 6082560.0\nphysical_input_units = 12165120.0\n";
    fs::write(&path, text).unwrap();
    let lock_path = state.join("odometer.lock");
    let lock = fs::File::create(&lock_path).unwrap();
    flock(&lock, FlockOperation::NonBlockingLockExclusive).unwrap();
    let active = report();
    assert!(active.status.success(), "{:?}", active);
    let stdout = String::from_utf8(active.stdout).unwrap();
    assert!(stdout.contains("Fievel:                  1.000000 miles (estimated)"));
    assert!(stdout.contains("Physical mouse/trackpad:  2.000000 miles (estimated)"));
    assert_eq!(fs::read_to_string(&path).unwrap(), text);

    drop(lock);
    assert!(
        report().status.success(),
        "saved totals also work after the writer exits"
    );
    for invalid in [
        "broken",
        "fievel_input_units = -1\nphysical_input_units = 0",
        "fievel_input_units = 0\nphysical_input_units = inf",
    ] {
        fs::write(&path, invalid).unwrap();
        let result = report();
        assert!(!result.status.success());
        assert!(!result.stderr.is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
    }
    let reset = Command::new(env!("CARGO_BIN_EXE_fievel"))
        .env("XDG_STATE_HOME", &directory)
        .env_remove("HOME")
        .args(["--reset-odometer", "--config", "/nonexistent/fievel.config"])
        .output()
        .unwrap();
    assert!(reset.status.success(), "{reset:?}");
    let result = report();
    assert!(result.status.success());
    assert!(String::from_utf8(result.stdout)
        .unwrap()
        .contains("0.000000 miles"));
    fs::remove_file(&path).unwrap();
    fs::remove_file(&lock_path).unwrap();
    fs::remove_dir(&state).unwrap();
    fs::remove_dir(&directory).unwrap();
}
