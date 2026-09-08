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
```

## Run (requires built `a3s`)

```bash
just code-tui-a3s-test-run
```

Environment tips:

- Prefer a disposable workspace via `--tui-working-directory`
- Set `A3S_NO_AUTO_INSTALL=1` and point `A3S_WEBVIEW_BIN` at a local debug helper
- Do not synchronize with sleep; only `wait` / `expect` on stable English chrome text

Stable chrome anchors (from the welcome banner tip / metadata):

- `a3s-code v`
- `Type a message`
- `/ for commands`
- `/help` panel title `A3S Code`
