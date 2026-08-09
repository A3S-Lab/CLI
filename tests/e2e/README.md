# A3S Test Web regressions

`activity-document.acl` exercises the browser-enforced cognitive-package
Activity document boundary directly. `activity-host.acl` opens the production
A3S Web build, waits for its generation-bound iframe to become ready through a
dedicated `a3s.activity.v2` `MessagePort`, and sends a bounded context proposal
through that port into the host-owned review dialog. Both suites use only the
static fixture under `tests/fixtures/activity-document-browser`; neither
contacts a model or an external Registry.

From the CLI directory in the A3S monorepo, build Code and the production Web
assets, then start the isolated fixture host:

```bash
cargo build --bin a3s
(cd ../../apps/web && bun install --frozen-lockfile && bun run build)
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
  --directory tests/fixtures/activity-document-browser/workspace \
  web -d --host 127.0.0.1 --port 43123 \
  --web-dir ../../apps/web/dist/workspace
```

Wait for the catalog to contain `browser-fixture:sandbox`, then validate and
run the suite:

```bash
curl --fail http://127.0.0.1:43123/api/v1/plugins/activities
a3s-test check tests/e2e/activity-document.acl --json
a3s-test run tests/e2e/activity-document.acl --json
a3s-test check tests/e2e/activity-host.acl --json
a3s-test run tests/e2e/activity-host.acl --json
```

Terminate only the exact `Background PID` printed by `a3s web` after the run.
The Rust Marketplace integration tests remain authoritative for response
headers, restart identity, `404`/`410` behavior, upgrade/disable/uninstall, and
managed-path non-disclosure. The host suite records accessibility, console,
page-error, and screenshot evidence under `.a3s-test/runs/`.
