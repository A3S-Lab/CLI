#![cfg(unix)]

mod support;

use std::process::{Command, Output};

use support::{a3s_bin, TempWorkspace};

fn run(home: &std::path::Path, config: &std::path::Path, args: &[&str]) -> Output {
    Command::new(a3s_bin())
        .args(args)
        .env("HOME", home)
        .env("A3S_CONFIG_FILE", config)
        .env("RUST_BACKTRACE", "0")
        .output()
        .unwrap_or_else(|error| panic!("failed to run a3s {args:?}: {error}"))
}

#[test]
fn root_login_and_logout_share_the_os_session_store() {
    let workspace = TempWorkspace::new("root-os-auth");
    let home = workspace.path("home");
    let config = workspace.path("config.acl");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(&config, "os = \"https://os.example.test\"\n").unwrap();

    let login = run(&home, &config, &["login", "test-secret-token"]);
    assert!(
        login.status.success(),
        "login failed: {}",
        String::from_utf8_lossy(&login.stderr)
    );
    let stdout = String::from_utf8_lossy(&login.stdout);
    assert!(stdout.contains("signed in to A3S OS"));
    assert!(stdout.contains("ssh key: none found"));

    let store_path = home.join(".a3s/os-auth.json");
    let store = std::fs::read_to_string(&store_path).unwrap();
    assert!(store.contains("https://os.example.test"));
    assert!(store.contains("test-secret-token"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&store_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    let logout = run(&home, &config, &["logout"]);
    assert!(
        logout.status.success(),
        "logout failed: {}",
        String::from_utf8_lossy(&logout.stderr)
    );
    assert!(String::from_utf8_lossy(&logout.stdout).contains("signed out from A3S OS"));
    assert!(!store_path.exists());
    assert!(!home.join(".a3s/os-skills").exists());
}

#[test]
fn root_auth_help_does_not_require_configuration() {
    let workspace = TempWorkspace::new("root-os-auth-help");
    let home = workspace.path("home");
    let missing_config = workspace.path("missing.acl");
    std::fs::create_dir_all(&home).unwrap();

    for (args, expected) in [
        (&["login", "--help"][..], "usage: a3s login [token]"),
        (&["logout", "--help"][..], "usage: a3s logout"),
    ] {
        let output = run(&home, &missing_config, args);
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains(expected));
        assert_eq!(String::from_utf8_lossy(&output.stderr), "");
        assert!(!home.join(".a3s").exists());
    }
}
