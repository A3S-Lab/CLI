mod support;

#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::process::Command;

use support::{a3s_bin, configure_component_env, TempWorkspace};

#[test]
fn registry_lifecycle_uses_isolated_acl_files() {
    let temp = TempWorkspace::new("registry-lifecycle");
    let config = temp.path("config/config.acl");
    let digest = format!("sha256:{}", "a".repeat(64));

    let mut add = Command::new(a3s_bin());
    configure_component_env(&mut add, &temp);
    let output = add
        .arg("--config")
        .arg(&config)
        .args([
            "--output",
            "json",
            "registry",
            "add",
            "https://acme.example/components/",
            "--trust-root",
            &digest,
            "--yes",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["command"], "registry.add");
    assert_eq!(result["data"]["registry"]["name"], "acme");
    let registry_file = temp.path("config/registries/acme.acl");
    let acl = std::fs::read_to_string(&registry_file).unwrap();
    assert!(acl.contains("registry \"acme\""), "{acl}");
    assert!(acl.contains("trust_root"), "{acl}");

    let mut list = Command::new(a3s_bin());
    configure_component_env(&mut list, &temp);
    let output = list
        .arg("--config")
        .arg(&config)
        .args(["--output", "json", "registry", "list"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let registries = result["data"]["registries"].as_array().unwrap();
    assert!(registries.iter().any(|registry| registry["name"] == "a3s"));
    assert!(registries.iter().any(|registry| registry["name"] == "acme"));

    let mut show = Command::new(a3s_bin());
    configure_component_env(&mut show, &temp);
    let output = show
        .arg("--config")
        .arg(&config)
        .args(["--output", "json", "registry", "show", "acme"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["data"]["registry"]["trustRoot"], digest);

    let mut remove = Command::new(a3s_bin());
    configure_component_env(&mut remove, &temp);
    let output = remove
        .arg("--config")
        .arg(&config)
        .args(["--output", "json", "registry", "remove", "acme", "--yes"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!registry_file.exists());
}

#[test]
fn registry_sources_can_be_disabled_enabled_and_atomically_replaced_by_stable_name() {
    let temp = TempWorkspace::new("registry-source-controls");
    let config = temp.path("config/config.acl");
    let original_digest = format!("sha256:{}", "c".repeat(64));
    let replacement_digest = format!("sha256:{}", "d".repeat(64));

    let mut add = Command::new(a3s_bin());
    configure_component_env(&mut add, &temp);
    let output = add
        .arg("--config")
        .arg(&config)
        .args([
            "--output",
            "json",
            "registry",
            "add",
            "https://acme.example/components/",
            "--trust-root",
            &original_digest,
            "--yes",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut disable = Command::new(a3s_bin());
    configure_component_env(&mut disable, &temp);
    let output = disable
        .arg("--config")
        .arg(&config)
        .args(["--output", "json", "registry", "disable", "acme", "--yes"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let disabled: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(disabled["command"], "registry.disable");
    assert_eq!(disabled["data"]["changed"], true);
    assert_eq!(disabled["data"]["registry"]["name"], "acme");
    assert_eq!(disabled["data"]["registry"]["enabled"], false);

    let registry_file = temp.path("config/registries/acme.acl");
    let acl = std::fs::read_to_string(&registry_file).unwrap();
    assert!(acl.contains("enabled = false"), "{acl}");

    let mut refresh = Command::new(a3s_bin());
    configure_component_env(&mut refresh, &temp);
    let output = refresh
        .arg("--config")
        .arg(&config)
        .args(["--output", "json", "registry", "refresh", "acme"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    let failed_refresh: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(failed_refresh["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("is disabled")));

    let mut replace = Command::new(a3s_bin());
    configure_component_env(&mut replace, &temp);
    let output = replace
        .arg("--config")
        .arg(&config)
        .args([
            "--output",
            "json",
            "registry",
            "replace",
            "acme",
            "https://mirror.example/v3/",
            "--trust-root",
            &replacement_digest,
            "--yes",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let replaced: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(replaced["command"], "registry.replace");
    assert_eq!(replaced["data"]["replaced"], true);
    assert_eq!(replaced["data"]["registry"]["name"], "acme");
    assert_eq!(
        replaced["data"]["registry"]["url"],
        "https://mirror.example/v3/"
    );
    assert_eq!(
        replaced["data"]["registry"]["trustRoot"],
        replacement_digest
    );
    assert_eq!(replaced["data"]["registry"]["enabled"], false);

    let mut enable = Command::new(a3s_bin());
    configure_component_env(&mut enable, &temp);
    let output = enable
        .arg("--config")
        .arg(&config)
        .args(["--output", "json", "registry", "enable", "acme", "--yes"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let enabled: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(enabled["command"], "registry.enable");
    assert_eq!(enabled["data"]["changed"], true);
    assert_eq!(enabled["data"]["registry"]["enabled"], true);

    let mut enable_again = Command::new(a3s_bin());
    configure_component_env(&mut enable_again, &temp);
    let output = enable_again
        .arg("--config")
        .arg(&config)
        .args(["--output", "json", "registry", "enable", "acme"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let unchanged: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(unchanged["data"]["changed"], false);

    let acl = std::fs::read_to_string(registry_file).unwrap();
    assert!(acl.contains("enabled = true"), "{acl}");
    assert!(
        acl.contains("url = \"https://mirror.example/v3/\""),
        "{acl}"
    );
    assert!(acl.contains(&replacement_digest), "{acl}");
}

#[test]
fn registry_source_mutations_require_explicit_authority_and_reject_the_builtin_source() {
    let temp = TempWorkspace::new("registry-source-authority");
    let config = temp.path("config/config.acl");
    let digest = format!("sha256:{}", "e".repeat(64));

    let mut add = Command::new(a3s_bin());
    configure_component_env(&mut add, &temp);
    let output = add
        .arg("--config")
        .arg(&config)
        .args([
            "registry",
            "add",
            "https://acme.example/components/",
            "--trust-root",
            &digest,
            "--yes",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    for args in [
        vec!["registry", "disable", "acme"],
        vec![
            "registry",
            "replace",
            "acme",
            "https://mirror.example/components/",
            "--trust-root",
            &digest,
        ],
    ] {
        let mut command = Command::new(a3s_bin());
        configure_component_env(&mut command, &temp);
        let output = command
            .arg("--config")
            .arg(&config)
            .arg("--output")
            .arg("json")
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{output:?}");
        let rendered = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(rendered.contains("requires '--yes'"), "{rendered}");
    }

    for args in [
        vec!["registry", "disable", "a3s", "--yes"],
        vec![
            "registry",
            "replace",
            "a3s",
            "https://mirror.example/components/",
            "--trust-root",
            &digest,
            "--yes",
        ],
    ] {
        let mut command = Command::new(a3s_bin());
        configure_component_env(&mut command, &temp);
        let output = command
            .arg("--config")
            .arg(&config)
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("built-in official registry"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn registry_rejects_urls_that_can_leak_secrets_or_change_identity() {
    let temp = TempWorkspace::new("registry-url-policy");
    let config = temp.path("config/config.acl");
    for url in [
        "https://user:secret@acme.example/components/",
        "https://acme.example/components/?token=secret",
        "https://acme.example/components/#alternate",
        "http://acme.example/components/",
    ] {
        let mut command = Command::new(a3s_bin());
        configure_component_env(&mut command, &temp);
        let output = command
            .arg("--config")
            .arg(&config)
            .args([
                "registry",
                "add",
                url,
                "--trust-root",
                &format!("sha256:{}", "b".repeat(64)),
                "--yes",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "unexpectedly accepted {url}");
    }
    assert!(!temp.path("config/registries").exists());
}

#[test]
fn registry_file_trust_root_is_copied_into_owned_configuration() {
    let temp = TempWorkspace::new("registry-root-copy");
    let config = temp.path("config/config.acl");
    let source = temp.path("bootstrap-root.json");
    let root_bytes = br#"{"signed":{"_type":"root","version":1}}"#;
    std::fs::write(&source, root_bytes).unwrap();

    let mut add = Command::new(a3s_bin());
    configure_component_env(&mut add, &temp);
    let output = add
        .arg("--config")
        .arg(&config)
        .args([
            "--output",
            "json",
            "registry",
            "add",
            "https://files.example/components/",
            "--trust-root",
            source.to_str().unwrap(),
            "--yes",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let owned = PathBuf::from(
        result["data"]["registry"]["trustedRootPath"]
            .as_str()
            .unwrap(),
    );
    assert!(
        owned.starts_with(temp.path("config/registries/files/roots")),
        "{}",
        owned.display()
    );
    assert_eq!(std::fs::read(&owned).unwrap(), root_bytes);
    std::fs::write(&source, b"changed outside registry ownership").unwrap();
    assert_eq!(std::fs::read(&owned).unwrap(), root_bytes);
    assert_eq!(
        result["data"]["registry"]["trustedRootPath"],
        owned.to_string_lossy().to_string()
    );

    let replacement_source = temp.path("replacement-root.json");
    let replacement_bytes = br#"{"signed":{"_type":"root","version":2}}"#;
    std::fs::write(&replacement_source, replacement_bytes).unwrap();
    let mut replace = Command::new(a3s_bin());
    configure_component_env(&mut replace, &temp);
    let output = replace
        .arg("--config")
        .arg(&config)
        .args([
            "--output",
            "json",
            "registry",
            "replace",
            "files",
            "https://mirror.example/components/",
            "--trust-root",
            replacement_source.to_str().unwrap(),
            "--yes",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let replacement_owned = PathBuf::from(
        result["data"]["registry"]["trustedRootPath"]
            .as_str()
            .unwrap(),
    );
    assert_ne!(replacement_owned, owned);
    assert_eq!(
        std::fs::read(&replacement_owned).unwrap(),
        replacement_bytes
    );
    assert_eq!(std::fs::read(&owned).unwrap(), root_bytes);
    let acl = std::fs::read_to_string(temp.path("config/registries/files.acl")).unwrap();
    assert!(
        acl.contains(replacement_owned.file_name().unwrap().to_str().unwrap()),
        "{acl}"
    );

    let mut remove = Command::new(a3s_bin());
    configure_component_env(&mut remove, &temp);
    let output = remove
        .arg("--config")
        .arg(&config)
        .args(["registry", "remove", "files", "--yes"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!owned.exists());
    assert!(!replacement_owned.exists());
}

#[test]
fn cache_dry_run_and_clean_stay_inside_the_owned_root() {
    let temp = TempWorkspace::new("cache-boundary");
    let cache = temp.path("cache");
    let outside = temp.path("keep.txt");
    std::fs::create_dir_all(cache.join("nested")).unwrap();
    std::fs::write(cache.join("nested/data.bin"), b"cache").unwrap();
    std::fs::write(&outside, b"keep").unwrap();

    let mut dry_run = Command::new(a3s_bin());
    configure_component_env(&mut dry_run, &temp);
    let output = dry_run
        .args(["--output", "json", "cache", "clean", "--dry-run"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["command"], "cache.clean");
    assert_eq!(result["data"]["dryRun"], true);
    assert!(cache.join("nested/data.bin").is_file());

    let mut clean = Command::new(a3s_bin());
    configure_component_env(&mut clean, &temp);
    let output = clean.args(["cache", "clean", "--yes"]).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(cache.is_dir());
    assert!(std::fs::read_dir(&cache).unwrap().next().is_none());
    assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
}

#[test]
#[cfg(unix)]
fn cache_clean_refuses_a_symbolic_link_root() {
    let temp = TempWorkspace::new("cache-symlink-root");
    let target = temp.path("target");
    let link = temp.path("cache-link");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("important.txt"), b"keep").unwrap();
    symlink(&target, &link).unwrap();

    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, &temp);
    let output = command
        .env("A3S_CACHE_HOME", &link)
        .args(["cache", "clean", "--yes"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(target.join("important.txt").is_file());
    assert!(String::from_utf8_lossy(&output.stderr).contains("symbolic-link"));
}
