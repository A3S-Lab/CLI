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
fn release_state_does_not_pin_the_removed_in_process_tui() {
    let workflow = include_str!("../.github/workflows/release.yml");
    let checker = include_str!("../.github/scripts/check-release-state.sh");

    assert!(!workflow.contains("A3S_TUI_VERSION"));
    assert!(!workflow.contains("a3s-tui"));
    assert!(workflow.contains("A3S_CODE_TUI_VERSION: 9.1.2"));
    assert!(workflow.contains(
        "\"$version\" \"$A3S_CODE_CORE_VERSION\" \\\n            \"$A3S_SEARCH_VERSION\" \"$A3S_CODE_CORE_REVISION\""
    ));
    assert!(!checker.contains("require_exact_registry_dependency a3s-tui"));
    assert!(checker.contains("must not depend on a3s-tui"));
}

#[test]
fn release_does_not_require_unlinked_gateway() {
    let manifest = include_str!("../Cargo.toml");
    let workflow = include_str!("../.github/workflows/release.yml");

    assert!(!manifest.contains("a3s-gateway"));
    assert!(!workflow.contains("A3S_GATEWAY_VERSION"));
    assert!(!workflow.contains("a3s-gateway"));
    assert!(workflow.contains("\"a3s-flow $A3S_FLOW_VERSION\""));
    assert!(workflow.contains("\"a3s-lane 0.5.1\""));
    assert!(workflow.contains("\"a3s-runtime 0.3.0\""));
    assert!(workflow.contains("\"a3s-use 0.3.12\""));
}

#[test]
fn manifest_drops_dependencies_of_removed_hosts() {
    let manifest = include_str!("../Cargo.toml");

    assert!(!manifest.contains("tokio-stream"));
    assert!(!manifest.contains("comrak"));
    assert!(!manifest.contains("similar"));
}

#[test]
fn release_targets_are_model_free_and_bound_arm64_lto() {
    let manifest = include_str!("../Cargo.toml");
    let workflow = include_str!("../.github/workflows/release.yml");

    assert!(workflow.contains(
        "{\"target\": \"aarch64-unknown-linux-gnu\", \"os\": \"ubuntu-24.04-arm\", \"helper\": \"a3s-webview\", \"moli\": \"moli\", \"code_tui\": \"a3s-code-tui\", \"code_acp\": \"a3s-code-acp\", \"features\": \"\", \"lto\": \"false\"}"
    ));
    assert!(
        !workflow.contains("\"target\": \"aarch64-unknown-linux-gnu\", \"os\": \"ubuntu-latest\"")
    );
    assert!(workflow.contains("CARGO_PROFILE_RELEASE_LTO: ${{ matrix.lto }}"));
    assert!(workflow.contains("\"lto\": \"thin\""));
    assert!(!workflow.contains("\"lto\": \"true\""));
    assert!(workflow.contains("CARGO_PROFILE_RELEASE_LTO: thin"));
    assert!(!workflow.contains("local-cpu-embedding"));
    assert!(!workflow.contains("a3s-power"));
    assert!(!manifest.contains("local-cpu-embedding"));
    assert!(!manifest.contains("a3s-power"));
    assert!(!manifest.contains("fastembed"));
}

#[test]
fn homebrew_formula_installs_bundled_webview_without_separate_formula() {
    let workflow = include_str!("../.github/workflows/release.yml");

    assert!(workflow.contains("${{ matrix.helper }}"));
    assert!(
        workflow.contains(r#"bin.install "a3s", "a3s-webview", "a3s-code-tui", "a3s-code-acp""#)
    );
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

    // Published Core pin: crates.io version + immutable Code.git rev.
    // A path = "../code/core" dependency is a local tree, not a shipped pin.
    assert!(
        manifest.contains(
            "a3s-code-core = { version = \"=9.1.2\", git = \"https://github.com/A3S-Lab/Code.git\", rev = \"98fb3db9c049ac4d1d73233326e5337b4e5a34be\", default-features = false, features = [\"scientific\"] }"
        ),
        "CLI Core pin must be the 9.1.2 git rev"
    );
    assert!(
        !manifest.contains("path = \"../code/core\""),
        "a published pin must not path-depend on ../code/core"
    );

    for dependency in [
        "a3s-use-core = \"=0.2.10\"",
        "a3s-use-extension = \"=0.3.12\"",
        "a3s-box-core = \"=3.2.0\"",
        "a3s-box-runtime = { version = \"=3.2.0\"",
        "a3s-box-netproxy = \"=3.2.7\"",
    ] {
        assert!(
            manifest.contains(dependency),
            "release manifest omitted published dependency `{dependency}`"
        );
    }
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Use\""));
    assert!(manifest.contains(
        "a3s-memory = { version = \"=0.1.4\", git = \"https://github.com/A3S-Lab/Memory.git\", rev = \"97a5e885d196be77dc1823ad86238e77d942ed73\" }"
    ));
    assert!(!manifest.contains("a3s-runtime"));
    assert!(!manifest.contains("a3s-tui"));
    assert!(!manifest.contains("a3s-gateway"));
    assert!(!manifest.contains("a3s-flow"));
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Box.git\""));
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Runtime\""));
    assert!(!manifest.contains("git = \"https://github.com/A3S-Lab/Gateway.git\""));

    for release_input in [
        "A3S_WEBVIEW_VERSION: 0.1.5",
        "A3S_CODE_CORE_VERSION: 9.1.2",
        "A3S_CODE_CORE_REVISION: 98fb3db9c049ac4d1d73233326e5337b4e5a34be",
        "A3S_CODE_TUI_VERSION: 9.1.2",
        "A3S_CODE_TUI_REVISION: 98fb3db9c049ac4d1d73233326e5337b4e5a34be",
        "A3S_ACL_REVISION: 5317e166222495585909d81f2caffdca90273c99",
        "A3S_VEC_REVISION: 730c953be34fc5efee775c8ef15dd07c20c4d402",
        "A3S_SEARCH_VERSION: 3.1.4",
        "A3S_SEARCH_REVISION: e38555cebb5a0fe9a982bde72700971262ac0773",
        "A3S_MEMORY_VERSION: 0.1.4",
        "A3S_MEMORY_REVISION: 97a5e885d196be77dc1823ad86238e77d942ed73",
        "\"a3s-memory $A3S_MEMORY_VERSION\"",
    ] {
        assert!(
            workflow.contains(release_input),
            "release workflow omitted `{release_input}`"
        );
    }
    assert!(
        !workflow.contains("\"a3s-code-core $A3S_CODE_CORE_VERSION\""),
        "git-pinned a3s-code-core 9.1.2 is not a crates.io prerequisite"
    );
    assert!(workflow.contains("\"$A3S_SEARCH_VERSION\" \"$A3S_CODE_CORE_REVISION\""));
    assert!(workflow.contains("\"$A3S_SEARCH_REVISION\" \"$A3S_MEMORY_VERSION\""));
    assert!(workflow.contains("\"$A3S_MEMORY_REVISION\""));

    for requirement in [
        "\"a3s-use 0.3.12\"",
        "\"a3s-use-core 0.2.10\"",
        "\"a3s-use-extension $A3S_USE_EXTENSION_VERSION\"",
        "A3S_USE_EXTENSION_VERSION: 0.3.12",
        "\"a3s-box-runtime $A3S_BOX_RUNTIME_VERSION\"",
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
    assert!(release.contains("lexical FTS is a3s-vec"));
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
        release
            .contains(r#"smoke_base="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/a3s-packaged-tui-smoke.$$""#),
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
            && release.contains("Windows zip/7z extract often omits the MSYS executable bit"),
        "Windows packaged TUI smoke must chmod +x the extracted a3s.exe before exec"
    );
    assert!(
        !release.contains("A3S_CODE_TUI_SMOKE"),
        "packaged smoke must not launch the interactive pager"
    );
    assert!(
        release.contains("\"$pager\" --help >/dev/null")
            && release.contains("\"$acp\" --help >/dev/null")
            && release.contains("a3s-code-tui.exe")
            && release.contains("a3s-code-acp.exe"),
        "packaged smoke must exercise bundled pager and ACP --help, including Windows names"
    );
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
        "code-surface:",
        "needs.release-preflight.outputs.code_build_matrix",
        "[.[] | (. + {part: \"acp\"}), (. + {part: \"tui\"})]",
        "matrix.part == 'acp'",
        "matrix.part == 'tui'",
        "name: a3s-code-acp-${{ matrix.target }}",
        "name: a3s-code-tui-${{ matrix.target }}",
        "repository: A3S-Lab/Vec",
        "path: layout/crates/vec",
        "repository: A3S-Lab/Effect",
        "path: layout/packages/effect",
        "A3S_EFFECT_REVISION: 08a11f782190cf8f83e034064dd018a3f99ec9e1",
        "Drop unused Code workspace patches",
        "[[patch.unused]]",
        "Install protobuf compiler on macOS",
        "brew install protobuf",
        "Install protobuf compiler on Windows",
        "choco install protoc -y",
        "replacing stale draft",
        "Pin find-msvc-tools for the pager toolchain",
        "version = \\\"0.1.14\\\"",
        "Make protoc dependency output portable",
        "--descriptor_set_out={null_device}",
        "cargo build --locked --release -p a3s-code-acp --bin a3s-code-acp",
        "cargo build --locked --release -p a3s-code-pager-bin --bin a3s-code-tui",
        "Swatinem/rust-cache@v2",
        "prefix-key: ${{ matrix.target }}-acp",
        "prefix-key: ${{ matrix.target }}-tui",
        "prefix-key: ${{ matrix.target }}-cli",
        "CARGO_PROFILE_RELEASE_INCREMENTAL: \"false\"",
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

#[test]
fn homebrew_job_writes_a3s_formula_with_code_caveats() {
    let workflow = include_str!("../.github/workflows/release.yml");

    assert!(
        workflow.contains("cat > tap/Formula/a3s.rb <<'RB'"),
        "homebrew job must overwrite Formula/a3s.rb"
    );
    assert!(
        workflow.contains("git add Formula/a3s.rb"),
        "homebrew job must stage Formula/a3s.rb"
    );
    assert!(
        workflow.contains("def caveats")
            && workflow.contains("The interactive Code TUI is launched with:")
            && workflow.contains("a3s code"),
        "Homebrew formula template must keep caveats pointing at `a3s code`"
    );
}

#[test]
fn homebrew_smoke_installs_umbrella_a3s_and_smokes_code() {
    let workflow = include_str!("../.github/workflows/release.yml");

    assert!(workflow.contains("homebrew-smoke:"));
    assert!(
        workflow.contains("brew install a3s-lab/tap/a3s"),
        "homebrew-smoke must install the umbrella `a3s` formula, not a3s-code"
    );
    assert!(
        !workflow.contains("brew install a3s-lab/tap/a3s-code"),
        "homebrew-smoke must not install the legacy a3s-code formula"
    );
    assert!(
        workflow.contains("\"$binary\" code --help")
            && workflow.contains("Smoke Homebrew a3s code TUI entry"),
        "homebrew-smoke must exercise `a3s code`"
    );
    assert!(
        !workflow.contains("A3S_CODE_TUI_SMOKE"),
        "homebrew-smoke must not launch the interactive pager"
    );
    assert!(
        workflow.contains("pager=\"${bin_dir}/a3s-code-tui\"")
            && workflow.contains("acp=\"${bin_dir}/a3s-code-acp\"")
            && workflow.contains("\"$pager\" --help >/dev/null")
            && workflow.contains("\"$acp\" --help >/dev/null"),
        "homebrew-smoke must exercise the bundled pager and ACP --help"
    );
}
