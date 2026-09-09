//! Hermetic contract for the Code TUI L2 a3s-test matrix (P0 suites).
//!
//! These suites run via `a3s-test` (owned PTY) in the monorepo. CLI CI still
//! proves the matrix files stay present and keep the capability-critical chrome
//! anchors so L2 cannot silently rot without needing crates/test in this repo.

use std::fs;
use std::path::PathBuf;

fn suites_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/a3s-test/suites")
}

fn read_suite(name: &str) -> String {
    let path = suites_dir().join(name);
    fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("missing L2 a3s-test suite {}: {error}", path.display())
    })
}

fn assert_contains(suite: &str, body: &str, needle: &str) {
    assert!(
        body.contains(needle),
        "{suite} must retain capability chrome anchor {needle:?}"
    );
}

#[test]
fn p0_a3s_test_suites_cover_boot_help_resize_exit_contracts() {
    let boot = read_suite("01-boot.acl");
    assert_contains("01-boot.acl", &boot, r#"surface = "tui""#);
    assert_contains("01-boot.acl", &boot, r#"regex = "a3s-code v[0-9]""#);
    assert_contains("01-boot.acl", &boot, r#"text = "Type a message""#);
    assert_contains("01-boot.acl", &boot, r#"text = "/ for commands""#);

    let help = read_suite("02-composer-help.acl");
    assert_contains("02-composer-help.acl", &help, r#"surface = "tui""#);
    assert_contains("02-composer-help.acl", &help, r#"text = "/help""#);
    assert_contains("02-composer-help.acl", &help, r#"text = "A3S Code""#);
    assert_contains("02-composer-help.acl", &help, r#"text = "/sandbox""#);

    let resize = read_suite("06-resize-exit.acl");
    assert_contains("06-resize-exit.acl", &resize, r#"surface = "tui""#);
    assert_contains("06-resize-exit.acl", &resize, "terminal_resize");
    assert_contains("06-resize-exit.acl", &resize, r#"key = "Control+c""#);
    assert_contains("06-resize-exit.acl", &resize, r#"regex = "a3s-code v[0-9]""#);
}

#[test]
fn p0_matrix_documents_required_case_ids() {
    let matrix = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/a3s-test/MATRIX.md"),
    )
    .expect("MATRIX.md");
    for id in ["TUI-BOOT-01", "TUI-EXIT-01", "TUI-NAV-01", "TUI-CMD-01"] {
        assert!(
            matrix.contains(id),
            "MATRIX.md must keep P0 case id {id}"
        );
    }
}
