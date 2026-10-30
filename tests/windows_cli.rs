#![cfg(target_os = "windows")]

use std::{fs, process::Command};

#[test]
fn odometer_report_and_reset_use_the_windows_local_app_data_directory() {
    let directory =
        std::env::temp_dir().join(format!("fievel-windows-odometer-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let run = |arguments: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_fievel"))
            .env("LOCALAPPDATA", &directory)
            .env("APPDATA", &directory)
            .args(arguments)
            .output()
            .unwrap()
    };

    let report = run(&["--odometer"]);
    assert!(report.status.success(), "{report:?}");
    assert!(String::from_utf8(report.stdout)
        .unwrap()
        .contains("Fievel:                  0.000000 miles (estimated)"));

    fs::create_dir(directory.join("Fievel")).unwrap();
    fs::write(
        directory.join("Fievel/odometer.toml"),
        "fievel_input_units = 6082560.0\nphysical_input_units = 12165120.0\n",
    )
    .unwrap();
    let reset = run(&["--reset-odometer"]);
    assert!(reset.status.success(), "{reset:?}");
    let report = run(&["--odometer"]);
    assert!(report.status.success(), "{report:?}");
    let stdout = String::from_utf8(report.stdout).unwrap();
    assert!(stdout.contains("Fievel:                  0.000000 miles (estimated)"));
    assert!(stdout.contains("Physical mouse/trackpad:  0.000000 miles (estimated)"));

    let mut resets = Vec::new();
    for _ in 0..2 {
        resets.push(
            Command::new(env!("CARGO_BIN_EXE_fievel"))
                .env("LOCALAPPDATA", &directory)
                .arg("--reset-odometer")
                .spawn()
                .unwrap(),
        );
    }
    for reset in resets {
        let output = reset.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn config_validation_and_overrides_work_on_windows() {
    let directory =
        std::env::temp_dir().join(format!("fievel-windows-config-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let config = directory.join("fievel.config");
    fs::write(&config, "notify = false\n[keys]\nfree_mouse = 'f4'\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_fievel"))
        .arg("--config")
        .arg(&config)
        .args(["--check-config", "--speed", "250"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.contains("KEY_F4"));
    assert!(stdout.contains("normal: 250.0"));
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn first_run_creates_commented_config_under_appdata() {
    let directory = std::env::temp_dir().join(format!(
        "fievel-windows-default-config-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_fievel"))
        .env("APPDATA", &directory)
        .env_remove("USERPROFILE")
        .arg("--check-config")
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let config = directory.join("Fievel/fievel.config");
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        include_str!("../fievel.config")
    );

    fs::remove_dir_all(directory).unwrap();
}
