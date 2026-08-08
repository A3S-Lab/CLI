mod support;

#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::process::{Command, Output};

use sha2::{Digest, Sha256};
use support::{a3s_bin, configure_component_env, TempWorkspace};

fn run_registry(temp: &TempWorkspace, args: &[&str]) -> Output {
    let mut command = Command::new(a3s_bin());
    configure_component_env(&mut command, temp);
    command
        .args(["--output", "json"])
        .args(args)
        .output()
        .unwrap()
}

fn registry_success(temp: &TempWorkspace, args: &[&str]) -> serde_json::Value {
    let output = run_registry(temp, args);
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn registry_failure(temp: &TempWorkspace, args: &[&str]) -> String {
    let output = run_registry(temp, args);
    assert!(!output.status.success(), "unexpected success for {args:?}");
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn mutation_revision(value: &serde_json::Value) -> &str {
    value["data"]["registrySources"]["snapshot"]["revision"]
        .as_str()
        .unwrap()
}

fn snapshot_revision(value: &serde_json::Value) -> &str {
    value["data"]["registrySources"]["revision"]
        .as_str()
        .unwrap()
}

#[test]
fn registry_lifecycle_uses_one_use_owned_acl_across_cli_configs() {
    let temp = TempWorkspace::new("registry-lifecycle");
    let first_config = temp.path("config/first.acl");
    let second_config = temp.path("config/second.acl");
    let first_config = first_config.to_str().unwrap();
    let second_config = second_config.to_str().unwrap();
    let digest = "a".repeat(64);

    let added = registry_success(
        &temp,
        &[
            "--config",
            first_config,
            "registry",
            "add",
            "acme",
            "https://acme.example/components/",
            "--root-sha256",
            &digest,
            "--yes",
        ],
    );
    assert_eq!(added["command"], "registry.add");
    assert_eq!(
        added["data"]["registrySources"]["snapshot"]["defaultRegistry"],
        "acme"
    );
    let source = &added["data"]["registrySources"]["snapshot"]["sources"][0];
    assert_eq!(source["name"], "acme");
    assert_eq!(source["rootSha256"], digest);
    assert_eq!(source["enabled"], true);

    let registry_file = temp.path("state/use/registries.acl");
    let acl = std::fs::read_to_string(&registry_file).unwrap();
    assert!(acl.contains("registries"), "{acl}");
    assert!(acl.contains("default_registry = \"acme\""), "{acl}");
    assert!(acl.contains("registry \"acme\""), "{acl}");
    assert!(
        acl.contains(&format!("root_sha256 = \"{digest}\"")),
        "{acl}"
    );
    assert!(!temp.path("config/registries").exists());

    let listed = registry_success(&temp, &["--config", second_config, "registry", "list"]);
    assert_eq!(
        listed["data"]["registrySources"]["sources"][0]["name"],
        "acme"
    );
    let revision = snapshot_revision(&listed).to_string();

    let shown = registry_success(&temp, &["registry", "show", "acme"]);
    assert_eq!(shown["data"]["revision"], revision);
    assert_eq!(shown["data"]["default"], true);
    assert_eq!(shown["data"]["registry"]["rootSha256"], digest);

    let removed = registry_success(
        &temp,
        &[
            "registry",
            "remove",
            "acme",
            "--revision",
            &revision,
            "--yes",
        ],
    );
    assert_eq!(removed["command"], "registry.remove");
    assert!(removed["data"]["registrySources"]["snapshot"]["sources"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(registry_file.exists());
}

#[test]
fn registry_mutations_use_revision_cas_and_preserve_stable_source_names() {
    let temp = TempWorkspace::new("registry-source-controls");
    let original_digest = "c".repeat(64);
    let replacement_digest = "d".repeat(64);

    registry_success(
        &temp,
        &[
            "registry",
            "add",
            "acme",
            "https://acme.example/components/",
            "--root-sha256",
            &original_digest,
            "--yes",
        ],
    );
    registry_success(
        &temp,
        &[
            "registry",
            "add",
            "backup",
            "https://backup.example/components/",
            "--root-sha256",
            &"b".repeat(64),
            "--yes",
        ],
    );
    let listed = registry_success(&temp, &["registry", "list"]);
    let initial_revision = snapshot_revision(&listed).to_string();

    let default_conflict = registry_failure(
        &temp,
        &[
            "registry",
            "disable",
            "acme",
            "--revision",
            &initial_revision,
            "--yes",
        ],
    );
    assert!(
        default_conflict.contains("current default"),
        "{default_conflict}"
    );

    let defaulted = registry_success(
        &temp,
        &[
            "registry",
            "default",
            "backup",
            "--revision",
            &initial_revision,
            "--yes",
        ],
    );
    let default_revision = mutation_revision(&defaulted).to_string();
    assert_eq!(
        defaulted["data"]["registrySources"]["snapshot"]["defaultRegistry"],
        "backup"
    );

    let stale = registry_failure(
        &temp,
        &[
            "registry",
            "disable",
            "acme",
            "--revision",
            &initial_revision,
            "--yes",
        ],
    );
    assert!(stale.contains("changed after it was reviewed"), "{stale}");

    let disabled = registry_success(
        &temp,
        &[
            "registry",
            "disable",
            "acme",
            "--revision",
            &default_revision,
            "--yes",
        ],
    );
    let disabled_revision = mutation_revision(&disabled).to_string();
    assert_eq!(
        disabled["data"]["registrySources"]["snapshot"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["name"] == "acme")
            .unwrap()["enabled"],
        false
    );

    let replaced = registry_success(
        &temp,
        &[
            "registry",
            "replace",
            "acme",
            "https://mirror.example/v4/",
            "--root-sha256",
            &replacement_digest,
            "--revision",
            &disabled_revision,
            "--yes",
        ],
    );
    let replaced_revision = mutation_revision(&replaced).to_string();
    let replacement = replaced["data"]["registrySources"]["snapshot"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["name"] == "acme")
        .unwrap();
    assert_eq!(replacement["registryUrl"], "https://mirror.example/v4/");
    assert_eq!(replacement["rootSha256"], replacement_digest);
    assert_eq!(replacement["enabled"], false);

    let enabled = registry_success(
        &temp,
        &[
            "registry",
            "enable",
            "acme",
            "--revision",
            &replaced_revision,
            "--yes",
        ],
    );
    let enabled_revision = mutation_revision(&enabled).to_string();
    assert_eq!(enabled["data"]["registrySources"]["changed"], true);

    let unchanged = registry_success(
        &temp,
        &[
            "registry",
            "enable",
            "acme",
            "--revision",
            &enabled_revision,
            "--yes",
        ],
    );
    assert_eq!(unchanged["data"]["registrySources"]["changed"], false);
}

#[test]
fn registry_sources_require_explicit_authority_and_safe_canonical_identity() {
    let temp = TempWorkspace::new("registry-source-authority");
    let digest = "e".repeat(64);
    let denied = registry_failure(
        &temp,
        &[
            "registry",
            "add",
            "acme",
            "https://acme.example/components/",
            "--root-sha256",
            &digest,
        ],
    );
    assert!(denied.contains("requires '--yes'"), "{denied}");

    for (name, url) in [
        (
            "credentials",
            "https://user:secret@acme.example/components/",
        ),
        ("query", "https://acme.example/components/?token=secret"),
        ("fragment", "https://acme.example/components/#alternate"),
        ("plaintext", "http://acme.example/components/"),
    ] {
        let rendered = registry_failure(
            &temp,
            &[
                "registry",
                "add",
                name,
                url,
                "--root-sha256",
                &digest,
                "--yes",
            ],
        );
        assert!(rendered.contains("Registry"), "{rendered}");
    }
    assert!(!temp.path("state/use/registries.acl").exists());

    registry_success(
        &temp,
        &[
            "registry",
            "add",
            "acme",
            "https://acme.example/components/",
            "--root-sha256",
            &digest,
            "--yes",
        ],
    );
    let duplicate = registry_failure(
        &temp,
        &[
            "registry",
            "add",
            "acme",
            "https://mirror.example/components/",
            "--root-sha256",
            &digest,
            "--yes",
        ],
    );
    assert!(duplicate.contains("already exists"), "{duplicate}");
}

#[test]
fn registry_trusted_roots_are_imported_by_digest_into_use_owned_state() {
    let temp = TempWorkspace::new("registry-root-copy");
    let source = temp.path("bootstrap-root.json");
    let root_bytes = br#"{"signed":{"_type":"root","version":1}}"#;
    std::fs::write(&source, root_bytes).unwrap();
    let digest = format!("{:x}", Sha256::digest(root_bytes));
    let source_path = source.to_str().unwrap();

    let added = registry_success(
        &temp,
        &[
            "registry",
            "add",
            "files",
            "https://files.example/components/",
            "--root-sha256",
            &digest,
            "--trusted-root",
            source_path,
            "--yes",
        ],
    );
    assert_eq!(
        added["data"]["registrySources"]["snapshot"]["sources"][0]["importedTrustedRoot"],
        true
    );
    let owned = temp.path(&format!(
        "state/use/registry-trust-roots/sha256/{digest}.json"
    ));
    assert_eq!(std::fs::read(&owned).unwrap(), root_bytes);
    std::fs::write(&source, b"changed outside registry ownership").unwrap();
    assert_eq!(std::fs::read(&owned).unwrap(), root_bytes);

    let replacement_source = temp.path("replacement-root.json");
    let replacement_bytes = br#"{"signed":{"_type":"root","version":2}}"#;
    std::fs::write(&replacement_source, replacement_bytes).unwrap();
    let replacement_digest = format!("{:x}", Sha256::digest(replacement_bytes));
    let replacement_source_path = replacement_source.to_str().unwrap();
    let revision = mutation_revision(&added).to_string();
    let replaced = registry_success(
        &temp,
        &[
            "registry",
            "replace",
            "files",
            "https://mirror.example/components/",
            "--root-sha256",
            &replacement_digest,
            "--trusted-root",
            replacement_source_path,
            "--revision",
            &revision,
            "--yes",
        ],
    );
    let replacement_owned = temp.path(&format!(
        "state/use/registry-trust-roots/sha256/{replacement_digest}.json"
    ));
    assert_eq!(
        std::fs::read(&replacement_owned).unwrap(),
        replacement_bytes
    );
    assert_eq!(std::fs::read(&owned).unwrap(), root_bytes);

    let replacement_revision = mutation_revision(&replaced).to_string();
    registry_success(
        &temp,
        &[
            "registry",
            "remove",
            "files",
            "--revision",
            &replacement_revision,
            "--yes",
        ],
    );
    let listed = registry_success(&temp, &["registry", "list"]);
    assert!(listed["data"]["registrySources"]["sources"]
        .as_array()
        .unwrap()
        .is_empty());
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
