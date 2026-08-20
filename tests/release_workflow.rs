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
