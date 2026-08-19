#[test]
fn tagged_release_recovery_is_bound_to_main_and_the_frozen_tag() {
    let workflow = include_str!("../.github/workflows/release.yml");

    assert!(workflow.contains("workflow_dispatch:"));
    assert!(workflow.contains("release_tag:"));
    assert!(workflow.contains(
        "RELEASE_TAG: ${{ github.event_name == 'workflow_dispatch' && inputs.release_tag || github.ref_name }}"
    ));
    assert!(workflow.contains("test \"$GITHUB_REF_NAME\" = \"main\""));
    assert!(workflow.contains("git merge-base --is-ancestor \"$checkout_sha\" origin/main"));
    assert!(workflow.contains("git ls-remote origin \"refs/tags/${RELEASE_TAG}^{}\""));
    assert_eq!(
        workflow.matches("ref: ${{ env.RELEASE_TAG }}").count(),
        7,
        "every CLI source checkout must use the immutable release tag"
    );
    assert!(workflow.contains("archive: a3s-${{ env.RELEASE_TAG }}-$target"));
}

#[test]
fn local_cpu_release_targets_use_native_architecture_runners() {
    let workflow = include_str!("../.github/workflows/release.yml");

    assert!(workflow.contains(
        "{ target: aarch64-unknown-linux-gnu, os: ubuntu-24.04-arm, helper: a3s-webview, features: local-cpu-embedding }"
    ));
    assert!(!workflow.contains(
        "{ target: aarch64-unknown-linux-gnu, os: ubuntu-latest, helper: a3s-webview, features: local-cpu-embedding }"
    ));
}

#[test]
fn web_release_install_retries_transient_registry_failures_without_cache() {
    let workflow = include_str!("../.github/workflows/release.yml");

    assert!(workflow.contains("for attempt in 1 2 3 4; do"));
    assert!(workflow.contains("bun install --frozen-lockfile --no-cache"));
    assert!(workflow.contains("Bun install failed after ${attempt} attempts"));
}
