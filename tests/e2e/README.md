# A3S Test Web regressions

`activity-document.acl` exercises the browser-enforced part of the cognitive
package Activity document boundary. It uses only the static fixture under
`tests/fixtures/activity-document-browser` and never contacts a model or an
external Registry.

From the CLI repository root, build Code and start the isolated fixture host:

```bash
cargo build --bin a3s
mkdir -p /tmp/a3s-activity-document-e2e/{data,state,cache,runtime,web-state}

A3S_DATA_HOME=/tmp/a3s-activity-document-e2e/data \
A3S_STATE_HOME=/tmp/a3s-activity-document-e2e/state \
A3S_CACHE_HOME=/tmp/a3s-activity-document-e2e/cache \
A3S_RUNTIME_HOME=/tmp/a3s-activity-document-e2e/runtime \
A3S_CODE_WEB_STATE_DIR=/tmp/a3s-activity-document-e2e/web-state \
A3S_USE_INSTALL_DIR="$PWD/tests/fixtures/activity-document-browser" \
RUST_MIN_STACK=2097152 \
target/debug/a3s \
  --config tests/fixtures/activity-document-browser/config.acl \
  web -d --host 127.0.0.1 --port 43123 \
  --workspace tests/fixtures/activity-document-browser/workspace \
  --web-dir tests/fixtures/activity-document-browser/web
```

Wait for the catalog to contain `browser-fixture:sandbox`, then validate and
run the suite:

```bash
curl --fail http://127.0.0.1:43123/api/v1/plugins/activities
a3s-test check tests/e2e/activity-document.acl --json
a3s-test run tests/e2e/activity-document.acl --json
```

Terminate only the exact `Background PID` printed by `a3s web` after the run.
The Rust Marketplace integration tests remain authoritative for response
headers, restart identity, `404`/`410` behavior, upgrade/disable/uninstall, and
managed-path non-disclosure.
