#[test]
fn tagged_release_recovery_is_bound_to_main_and_the_frozen_tag() {
    let workflow = include_str!("../.github/workflows/release.yml");

    assert!(workflow.contains("workflow_dispatch:"));
    assert!(workflow.contains("release_tag:"));
    assert!(workflow.contains("release_target:"));
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
    assert!(workflow
        .contains("include: ${{ fromJSON(needs.release-preflight.outputs.release_matrix) }}"));
}

#[test]
fn local_cpu_release_targets_use_native_runners_and_bounded_optimization() {
    let workflow = include_str!("../.github/workflows/release.yml");

    assert!(workflow.contains(
        "{\"target\": \"aarch64-unknown-linux-gnu\", \"os\": \"ubuntu-24.04-arm\", \"helper\": \"a3s-webview\", \"features\": \"local-cpu-embedding\", \"lto\": \"false\"}"
    ));
    assert!(
        !workflow.contains("\"target\": \"aarch64-unknown-linux-gnu\", \"os\": \"ubuntu-latest\"")
    );
    assert!(workflow.contains("CARGO_PROFILE_RELEASE_LTO: ${{ matrix.lto }}"));
}

#[test]
fn release_recovery_can_rebuild_one_validated_target() {
    let workflow = include_str!("../.github/workflows/release.yml");

    assert!(workflow.contains("RECOVERY_TARGET: ${{ inputs.release_target || '' }}"));
    assert!(workflow.contains("release_matrix: ${{ steps.release-matrix.outputs.release_matrix }}"));
    assert!(workflow.contains("--arg target \"$RECOVERY_TARGET\""));
    assert!(workflow.contains("Unsupported release recovery target"));
}

#[test]
fn release_resolves_the_published_composable_runtime_graph() {
    let manifest = include_str!("../Cargo.toml");
    let workflow = include_str!("../.github/workflows/release.yml");

    for dependency in [
        "a3s-code-core = \"=8.0.4\"",
        "a3s-flow = \"=1.1.0\"",
        "a3s-memory = \"=0.1.3\"",
        "a3s-use = { version = \"=0.3.4\"",
        "a3s-use-core = \"=0.2.4\"",
        "a3s-use-extension = \"=0.3.4\"",
        "a3s-box-core = \"=3.2.0\"",
        "a3s-box-runtime = { version = \"=3.2.0\"",
        "a3s-runtime = \"=0.3.0\"",
        "a3s-gateway = \"=1.1.1\"",
    ] {
        assert!(
            manifest.contains(dependency),
            "release manifest omitted published dependency `{dependency}`"
        );
    }
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Use\""));
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Code.git\""));
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Flow.git\""));
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Memory.git\""));
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Box.git\""));
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Runtime\""));
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Gateway.git\""));

    for requirement in [
        "\"a3s-use 0.3.4\"",
        "\"a3s-use-core 0.2.4\"",
        "\"a3s-use-extension $A3S_USE_EXTENSION_VERSION\"",
        "A3S_USE_EXTENSION_VERSION: 0.3.4",
        "\"a3s-box-runtime $A3S_BOX_RUNTIME_VERSION\"",
        "A3S_GATEWAY_VERSION: 1.1.1",
        "\"a3s-gateway $A3S_GATEWAY_VERSION\"",
    ] {
        assert!(
            workflow.contains(requirement),
            "release preflight omitted `{requirement}`"
        );
    }
}
