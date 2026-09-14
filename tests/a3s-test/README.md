# A3S Code TUI — a3s-test L2 matrix

Interactive TUI regression suites for `a3s code`, driven by **a3s-test** owned
PTY + VT semantic assertions.

| Layer | Tool | Coverage |
| --- | --- | --- |
| L0 | `cargo test --bin a3s` | Permissions, sandbox, panels, chrome units |
| L1 | `A3S_CODE_TUI_SMOKE=1` | Headless stream / shell / research |
| **L2** | **`a3s-test run … surface=tui`** | First frame, composer, slash, resize, exit |
| L3 | a3s-test GUI | Real RemoteUI windows (optional) |

## Layout

```text
crates/cli/tests/a3s-test/
  README.md                 # this file
  MATRIX.md                 # case IDs and priority
  suites/
    01-boot.acl             # cold start first-frame chrome
    02-composer-help.acl    # /help panel path
    06-resize-exit.acl      # resize + quit key sequence
```

## Check (no TUI process)

```bash
just code-tui-a3s-test-check
# or:
cargo run --manifest-path crates/test/Cargo.toml -p a3s-test-cli -- \
  check crates/cli/tests/a3s-test/suites/*.acl --json
cargo test --manifest-path crates/cli/Cargo.toml --test a3s_test_matrix
```

CLI CI also runs `a3s_test_matrix` hermetically so P0 suite anchors cannot
rot without crates/test present in the CLI repository checkout.

## Run (requires built `a3s`)

```bash
just code-tui-a3s-test-run
```

Environment tips:

- Prefer a disposable workspace via `--tui-working-directory`
- Set `A3S_NO_AUTO_INSTALL=1` and point `A3S_WEBVIEW_BIN` at a local debug helper
- Do not synchronize with sleep; only `wait` / `expect` on stable English chrome text

Live model proof (not part of `just code-tui-a3s-test-run`):

```bash
A3S_CONFIG_FILE="$PWD/.a3s/config.acl" A3S_NO_AUTO_INSTALL=1 \
  crates/test/target/debug/a3s-test run crates/cli/tests/a3s-test/live/ask-user-overlay.acl \
  --tui-executable crates/cli/target/debug/a3s \
  --tui-arg code \
  --tui-working-directory /absolute/disposable/workspace
```

`live/desktop-handoff.acl` is a host proof, not a CI suite. Do not pin
`A3S_DESKTOP_BIN`; `/desktop` must discover the newest Desktop, then focus one
already open on the same workspace or launch one and name a pid that is still
alive. That handoff does not wait on a model. A later scenario asks `17+26`
and waits for `43`, which is not in the prompt, so a composer placeholder
cannot satisfy it.

That suite is only green when the configured `boyue/deepseek-v4-flash` session
paints the question overlay and a picker selection resumes `answered · token-right`.
A unit call to `ask_user::answer` is not a substitute. It does not prove a
write. `live/write-then-read.acl` is that separate proof: the TUI must show
`Added proof-read.txt` (write tool diff chrome), then `Read proof-read.txt`,
then `OK_19e2c7` only after the Read chrome (regex ordered wait). That reply
string is not in the prompt. The written file must also exist under the
isolation worktree with `proof-read-7c2e91`. A welcome banner or a thinking
paraphrase of the prompt is not a pass. Run with
`--command-timeout-ms 240000`. The workspace must be a Git repository with a
commit; isolation refuses an unknown source revision.
A debug binary also needs `libzvec_c_api.dylib` on the loader path. macOS SIP
strips `DYLD_LIBRARY_PATH` when a protected parent execs the binary, so set it
from an unrestricted launcher if the parent is `script`, `expect`, or similar.

Stable chrome anchors (from the welcome banner tip / metadata):

- `a3s-code v`
- `Type a message`
- `/ for commands`
- `/help` panel title `A3S Code`
