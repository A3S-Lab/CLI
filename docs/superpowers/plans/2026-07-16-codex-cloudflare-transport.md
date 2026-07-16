# Codex Cloudflare Transport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make ChatGPT-account Codex requests survive Cloudflare's mitigation handshake by using an official-compatible HTTP generation and a strictly filtered, in-memory Cloudflare cookie jar.

**Architecture:** Add a CLI-private `CodexHttpClient` implementing the existing core HTTP abstraction. It uses a renamed reqwest 0.12 dependency and a session-scoped cookie provider restricted to Cloudflare cookies on HTTPS ChatGPT hosts. `CodexClient` selects this transport while retaining its existing Responses adapter and injected test clients.

**Tech Stack:** Rust 2021, reqwest 0.12, rustls native roots, Tokio, `a3s-code-core 5.3.2` HTTP traits, existing in-file unit and ignored live tests.

---

### Task 1: Lock the Cookie Security Policy with Failing Tests

**Files:**
- Create: `src/account_providers/codex_http.rs`
- Modify: `src/account_providers/mod.rs`
- Test: `src/account_providers/codex_http.rs`

- [ ] **Step 1: Add cookie-policy tests before implementation**

Add tests for allowed Cloudflare cookies on production and staging ChatGPT
HTTPS URLs. Add rejection tests for unrelated hosts, HTTP URLs, account/session
cookies, deceptive cookie names, and mixed safe/unsafe values.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test account_providers::codex_http::tests -- --nocapture
```

Expected: compilation fails because the cookie store and policy helpers do not
exist yet.

- [ ] **Step 3: Implement the minimal filtered cookie store**

Implement a reqwest `CookieStore` wrapper around `Jar`. Gate both writes and
reads by HTTPS ChatGPT host checks, filter `Set-Cookie` values through the
Cloudflare allowlist, and filter the outgoing `Cookie` header again. Keep the
store private and in memory.

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run the exact Task 1 command and confirm all policy tests pass.

### Task 2: Add the Codex-Specific HTTP Adapter

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `src/account_providers/codex_http.rs`
- Test: `src/account_providers/codex_http.rs`

- [ ] **Step 1: Add failing adapter tests**

Add local-server tests for JSON POST behavior, non-2xx error buffering,
successful streaming, header forwarding, and cancellation. Keep auth values
synthetic and assert they never appear in diagnostics.

- [ ] **Step 2: Run adapter tests and verify RED**

Run the exact new test names. Expected: compilation fails because
`CodexHttpClient` and its `HttpClient` implementation do not exist.

- [ ] **Step 3: Add the isolated reqwest 0.12 dependency**

Retain the existing reqwest dependency and add a renamed reqwest 0.12 entry
with only `json`, `stream`, `cookies`, and `rustls-tls-native-roots` features.
Regenerate the lockfile without changing unrelated dependency versions.

- [ ] **Step 4: Implement `CodexHttpClient`**

Build one reqwest client with the filtered cookie provider. Implement `post`
and `post_streaming`, including cancellation, Retry-After extraction, 2xx byte
streams, and buffered non-2xx bodies matching the core trait's semantics.

- [ ] **Step 5: Run adapter tests and verify GREEN**

Run the focused adapter tests and the full `codex_http` test module.

### Task 3: Wire Codex Account Sessions to the New Transport

**Files:**
- Modify: `src/account_providers/codex.rs`
- Modify: `src/account_providers/mod.rs`
- Test: `src/account_providers/codex.rs`

- [ ] **Step 1: Add a failing transport-selection test**

Expose a test-only transport-kind assertion or constructor seam and verify that
login-created Codex clients select the dedicated transport while existing
synthetic HTTP injection tests remain unchanged.

- [ ] **Step 2: Run the exact test and verify RED**

Expected: the constructor still selects `default_http_client()`.

- [ ] **Step 3: Select `CodexHttpClient` in the login constructor**

Construct the dedicated transport in `from_codex_login()`, propagate client
construction errors with context, and preserve shared transport behavior in
`fork_for_session()` and effort clones.

- [ ] **Step 4: Run Codex provider tests and verify GREEN**

Run:

```bash
cargo test account_providers::codex -- --nocapture
```

Expected: existing body, header, quota, SSE, tool-call, and new transport tests
all pass.

### Task 4: CI-Parity and Live Verification

**Files:**
- Modify if required by formatting only: files changed above

- [ ] **Step 1: Run formatting and focused static checks**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 2: Run the complete test and build matrix locally**

Run:

```bash
cargo test --all-targets
cargo build --release
```

Use a clean exported tree with
`.github/scripts/use-published-a3s-crates.sh` so these commands match CI and do
not depend on unpublished sibling repositories.

- [ ] **Step 3: Run the ignored live Codex smoke test**

Run the existing ignored `real_gpt_5_6_sol_native_effort_tool_smoke` test with
the local ChatGPT account. Confirm a real tool call and follow-up response
complete through the new transport.

- [ ] **Step 4: Reproduce the original interactive path**

Build the local binary, launch `a3s code`, select `gpt-5.6-sol` in `/model`, and
send `Reply exactly OK. Do not use tools.` Confirm the response completes and
the old Cloudflare 403/circuit-breaker error does not recur.

### Task 5: Review and Publish the Pull Request

**Files:**
- Review: all changed files

- [ ] **Step 1: Inspect the final diff and repository state**

Check for leaked tokens/cookies, unrelated formatting, generated files, debug
logging, and dependency drift. Confirm only the design, plan, dependency,
Codex transport, and provider wiring are changed.

- [ ] **Step 2: Run verification-before-completion checks**

Re-run every required command from Task 4 after the final edit and record the
fresh outputs.

- [ ] **Step 3: Commit the implementation**

Create focused commits for the design/plan and implementation, using repository
style commit messages.

- [ ] **Step 4: Push and open the PR**

Push `fix/codex-cloudflare-transport` to the authenticated fork and open a PR
against `A3S-Lab/CLI:main`. Include root cause, security boundary, test results,
and live verification in the PR body.
