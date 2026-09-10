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
        6,
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
        "{\"target\": \"aarch64-unknown-linux-gnu\", \"os\": \"ubuntu-24.04-arm\", \"helper\": \"a3s-webview\", \"moli\": \"moli\", \"features\": \"local-cpu-embedding\", \"lto\": \"false\"}"
    ));
    assert!(
        !workflow.contains("\"target\": \"aarch64-unknown-linux-gnu\", \"os\": \"ubuntu-latest\"")
    );
    assert!(workflow.contains("CARGO_PROFILE_RELEASE_LTO: ${{ matrix.lto }}"));
}

#[test]
fn homebrew_formula_installs_bundled_webview_without_separate_formula() {
    let workflow = include_str!("../.github/workflows/release.yml");

    assert!(workflow.contains("${{ matrix.helper }}"));
    assert!(workflow.contains(r#"bin.install "a3s", "a3s-webview""#));
    assert!(workflow.contains(r#"bin.install "moli""#));
    assert!(workflow.contains(r#"bin.install "libzvec_c_api.dylib""#));
    assert!(workflow.contains(r#"bin.install "libzvec_c_api.so""#));
    assert!(!workflow.contains(r#"depends_on "a3s-lab/tap/a3s-webview""#));
    // Quoted <<'RB' keeps formula comments like $ORIGIN literal under `set -u`.
    assert!(
        workflow.contains("<<'RB'"),
        "Homebrew formula must use a quoted heredoc so shell does not expand $ORIGIN"
    );
    assert!(
        workflow.contains("@loader_path / $ORIGIN resolve"),
        "formula comment should keep literal $ORIGIN (quoted heredoc, not bash escape)"
    );
    assert!(
        workflow.contains("__BASE__")
            && workflow.contains("__TAG__")
            && workflow.contains("__MAC_ARM__")
            && workflow.contains("FORMULA_BASE"),
        "formula urls/shas must be substituted from placeholders after the heredoc"
    );
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
fn release_resolves_the_composable_runtime_graph_and_pins_native_code() {
    let manifest = include_str!("../Cargo.toml");
    let workflow = include_str!("../.github/workflows/release.yml");

    for dependency in [
        "a3s-code-core = { version = \"=8.5.1\", git = \"https://github.com/A3S-Lab/Code.git\", rev = \"e35e708625e78b112fe1ef0a8cc2dce7f92db157\", default-features = false, features = [\"scientific\"] }",
        "a3s-use = { version = \"=0.3.11\"",
        "a3s-use-core = \"=0.2.9\"",
        "a3s-use-extension = \"=0.3.11\"",
        "a3s-box-core = \"=3.2.0\"",
        "a3s-box-runtime = { version = \"=3.2.0\"",
        "a3s-runtime = \"=0.3.0\"",
        "a3s-gateway = \"=1.1.1\"",
        "a3s-tui = \"=0.1.15\"",
    ] {
        assert!(
            manifest.contains(dependency),
            "release manifest omitted published dependency `{dependency}`"
        );
    }
    let lock = include_str!("../Cargo.lock");
    assert!(
        lock.contains("name = \"a3s-tui\"\nversion = \"0.1.15\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"d4edc1a57074390db682cd8c0ee1e24aee38d325deee3c04f465e6abfe4b1e05\""),
        "Cargo.lock must pin a3s-tui from crates.io with checksum (no path-only entry)"
    );
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Use\""));
    assert!(manifest.contains(
        "a3s-memory = { version = \"=0.1.4\", git = \"https://github.com/A3S-Lab/Memory.git\", rev = \"97a5e885d196be77dc1823ad86238e77d942ed73\" }"
    ));
    assert!(manifest.contains(
        "a3s-flow = { version = \"=1.1.0\", git = \"https://github.com/A3S-Lab/Flow.git\", rev = \"2948ad51a1395177764766c3ddf7e44338f9e374\" }"
    ));
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Box.git\""));
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Runtime\""));
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Gateway.git\""));

    for release_input in [
        "A3S_WEBVIEW_VERSION: 0.1.5",
        "A3S_CODE_CORE_VERSION: 8.5.1",
        "A3S_CODE_CORE_REVISION: e35e708625e78b112fe1ef0a8cc2dce7f92db157",
        "A3S_TUI_VERSION: 0.1.15",
        "A3S_SEARCH_VERSION: 3.1.0",
        "A3S_SEARCH_REVISION: c30e3dd04de8f2874113cda435439d6938bd3eb6",
        "A3S_MEMORY_VERSION: 0.1.4",
        "A3S_MEMORY_REVISION: 97a5e885d196be77dc1823ad86238e77d942ed73",
        "\"a3s-memory $A3S_MEMORY_VERSION\"",
    ] {
        assert!(
            workflow.contains(release_input),
            "release workflow omitted `{release_input}`"
        );
    }
    assert!(workflow.contains("\"$A3S_SEARCH_VERSION\" \"$A3S_CODE_CORE_REVISION\""));
    assert!(workflow.contains("\"$A3S_SEARCH_REVISION\" \"$A3S_MEMORY_VERSION\""));
    assert!(workflow.contains("\"$A3S_MEMORY_REVISION\""));

    for requirement in [
        "\"a3s-use 0.3.4\"",
        "\"a3s-use-core 0.2.4\"",
        "\"a3s-use-extension $A3S_USE_EXTENSION_VERSION\"",
        "A3S_USE_EXTENSION_VERSION: 0.3.4",
        "\"a3s-box-runtime $A3S_BOX_RUNTIME_VERSION\"",
        "A3S_GATEWAY_VERSION: 1.1.1",
        "\"a3s-gateway $A3S_GATEWAY_VERSION\"",
        "\"a3s-tui $A3S_TUI_VERSION\"",
    ] {
        assert!(
            workflow.contains(requirement),
            "release preflight omitted `{requirement}`"
        );
    }
}

#[test]
fn pull_requests_and_releases_gate_the_native_sandbox_on_every_platform() {
    let ci = include_str!("../.github/workflows/ci.yml");
    let release = include_str!("../.github/workflows/release.yml");
    let regression = "commands::code::sandbox::tests::real_native_sandbox_enforces_local_policy";

    assert!(ci.contains("native-sandbox:"));
    assert!(ci.contains("platform: linux, os: ubuntu-22.04"));
    assert!(ci.contains("platform: macos, os: macos-latest"));
    assert!(ci.contains("platform: windows, os: windows-latest"));
    assert!(ci.contains(regression));
    assert!(release.contains("native-sandbox-behavior:"));
    assert!(release.contains("platform: linux, os: ubuntu-22.04"));
    assert!(release.contains("platform: macos, os: macos-latest"));
    assert!(release.contains("platform: windows, os: windows-latest"));
    assert!(release.contains(regression));
    assert!(release.contains("provision-zvec-native.sh"));
    assert!(release.contains("Configure loader rpath for bundled zvec"));
    assert!(release.contains("@loader_path"));
    assert!(release.contains(r"rpath,$ORIGIN"));
    assert!(release.contains("Bundle zvec into non-Windows release archives"));
    assert!(release.contains("Verify bundled zvec linkage and rpath"));
    assert!(release.contains("Smoke packaged a3s code TUI entry"));
    assert!(release.contains("Smoke Homebrew a3s code TUI entry"));
    assert!(release.contains("Provision Linux bubblewrap for packaged TUI smoke"));
    assert!(release.contains("contains(matrix.target, 'unknown-linux-gnu')"));
    assert!(release.contains("apt-get install --no-install-recommends --yes bubblewrap"));
    assert!(
        release.contains(r#"7z x -y "-o${work}" "${base}.zip""#)
            || release.contains("7z x -y \"-o${work}\" \"${base}.zip\""),
        "Windows packaged TUI smoke must use 7z to extract the release zip"
    );
    assert!(
        release.contains(r#"smoke_base="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/a3s-packaged-tui-smoke.$$""#),
        "packaged TUI smoke must unpack under RUNNER_TEMP (not bare MSYS /tmp)"
    );
    assert!(
        release.contains("command -v 7z >/dev/null 2>&1")
            && release.contains("elif command -v unzip >/dev/null 2>&1"),
        "Windows packaged TUI smoke must prefer 7z over unzip"
    );
    assert!(
        release.contains(r#"binary="$(find "$work" -type f -name a3s.exe -print -quit)""#),
        "Windows packaged TUI smoke must resolve a3s.exe specifically"
    );
    assert!(
        release.contains("chmod +x \"$binary\"")
            && release.contains("x86_64-pc-windows-msvc")
            && release.contains(
                "Windows zip/7z extract often omits the MSYS executable bit"
            ),
        "Windows packaged TUI smoke must chmod +x the extracted a3s.exe before exec"
    );
    assert!(release.contains("A3S_CODE_TUI_SMOKE=1"));
    assert!(release.contains("A3S_CODE_TUI_PROMPT='!echo packaged-tui-ok'"));
    for removed in [
        "managed-srt",
        "managed_srt",
        "@anthropic-ai/sandbox-runtime",
        "support/managed-srt",
        "release-compat",
    ] {
        assert!(
            !ci.contains(removed),
            "CI retained removed SRT input: {removed}"
        );
        assert!(
            !release.contains(removed),
            "release retained removed SRT input: {removed}"
        );
    }
}

#[test]
fn release_archives_bundle_the_pinned_platform_moli_runtime() {
    let workflow = include_str!("../.github/workflows/release.yml");

    for input in [
        "A3S_MOLI_VERSION: 1.1.1",
        "moli-runtime-preflight:",
        "repository: A3S-Lab/Code",
        "bash code-core/scripts/package_moli.sh \"$RELEASE_TARGET\" moli-package",
        "name: moli-runtime-${{ matrix.target }}",
        "executable: moli",
        "executable: moli.exe",
        "moli/${MOLI_NAME}",
    ] {
        assert!(
            workflow.contains(input),
            "release workflow omitted `{input}`"
        );
    }
    assert!(workflow.contains("- moli-runtime-preflight"));
    assert!(workflow.contains("include: |\n            ${{ matrix.helper }}\n            moli"));
    assert!(workflow.contains("Verify Moli is inside every release archive"));
    assert!(workflow.contains("bin.install \"moli\""));
}
