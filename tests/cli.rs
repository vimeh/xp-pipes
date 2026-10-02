use std::process::Command;

#[test]
fn cli_does_not_need_a_display_and_reports_distinct_error_statuses() {
    for (args, expected) in [
        (vec!["--help"], 0),
        (vec!["--version"], 0),
        (vec!["--speed", "0"], 2),
        (vec!["--not-an-option"], 2),
        (vec!["--grid", "512x512x512"], 2),
        (vec!["--windowed", "--max-seconds", "1"], 1),
        (
            vec!["--windowed", "--tessellation", "4", "--speed-level", "0"],
            1,
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_wlpipes"))
            .args(&args)
            .env_remove("WAYLAND_SOCKET")
            .env("WAYLAND_DISPLAY", "/nonexistent-wlpipes-test-display")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(expected), "{args:?}: {output:?}");
        if expected != 0 {
            assert!(String::from_utf8_lossy(&output.stderr).contains("wlpipes:"));
        }
    }
}
