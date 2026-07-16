# Codex Cloudflare Transport Design

## Goal

Make ChatGPT-account Codex models usable from `a3s code` when Cloudflare rejects
the legacy HTTP fingerprint. The fix must stay in the CLI repository, preserve
the existing Codex Responses wire adapter, and never persist ChatGPT account or
session cookies.

## Evidence

The failure is transport-specific rather than an account, model, or request
payload failure:

- A3S 0.9.1 reaches `https://chatgpt.com/backend-api/codex/responses` but gets a
  Cloudflare HTML 403 twice.
- The installed official Codex CLI uses the same account and model on the same
  machine. Its WebSocket attempt gets a 403, then its HTTP fallback succeeds.
- A captured local request shows that the official client uses reqwest 0.12,
  while A3S's `a3s-code-core` HTTP client and the CLI's direct dependency use
  reqwest 0.11.
- The 403 response sets `__cf_bm`. The official client has a dedicated cookie
  store that accepts only documented Cloudflare cookies on HTTPS ChatGPT
  hosts. A3S's generic HTTP client has no cookie store.
- Replaying the cookie or changing only headers with curl remains blocked,
  ruling out a header-only or cookie-only fix. The HTTP/TLS implementation and
  the bounded cookie handshake must be aligned together.

## Considered Approaches

### 1. Codex-specific reqwest 0.12 transport in the CLI

Add a private `CodexHttpClient` that implements the existing
`a3s_code_core::llm::HttpClient` abstraction. It uses reqwest 0.12 with native
roots, streaming, and a session-scoped cookie provider. The provider stores
only an explicit allowlist of Cloudflare infrastructure cookies for HTTPS
ChatGPT hosts. The existing Responses request/stream parser remains unchanged.

This is the recommended approach. It is narrowly scoped to the failing account
provider, requires no unpublished `a3s-code-core` release, preserves A3S tool
calling, and can be unit-tested at both the cookie-policy and HTTP-adapter
boundaries.

### 2. Delegate turns to the installed official `codex` executable

The official executable already owns transport negotiation and token refresh,
but its CLI/app-server protocol is not the same as A3S's `LlmClient` tool-call
contract. Delegation would add a hard runtime dependency and require a second
agent protocol bridge. It is substantially larger and risks changing tool,
streaming, cancellation, and session behavior.

### 3. Change only User-Agent, headers, or retry count

This is small but disproven by the live probes. It would also impersonate a
first-party surface without matching its transport and would remain sensitive
to future Cloudflare policy changes. Extra retries alone repeat the same
blocked request.

## Architecture

`CodexClient::from_codex_login()` constructs one `CodexHttpClient` and stores it
behind the existing `Arc<dyn HttpClient>`. Session forks keep sharing that HTTP
client, so Cloudflare cookies learned by one request are available to later
turns and circuit-breaker retries without becoming process-global.

The transport implements both methods required by `HttpClient`:

- `post()` sends JSON and returns status plus body.
- `post_streaming()` sends JSON, returns the response byte stream for 2xx, and
  buffers only error bodies for non-2xx responses.

Cancellation continues to race request dispatch through `tokio::select!`. The
existing Codex SSE parser, usage-limit mapping, 401 handling, model catalog,
request bodies, and headers do not change.

## Cookie Security Boundary

The cookie provider accepts cookies only when all of these conditions hold:

- the URL scheme is HTTPS;
- the host is `chatgpt.com`, `chat.openai.com`,
  `chatgpt-staging.com`, or a subdomain of the ChatGPT production/staging
  domains;
- the cookie is one of `__cf_bm`, `__cflb`, `__cfruid`, `__cfseq`,
  `__cfwaitingroom`, `_cfuvid`, `cf_clearance`, `cf_ob_info`, `cf_use_ob`, or
  starts with `cf_chl_`.

Before returning a `Cookie` header, it filters the jar again through the same
allowlist. ChatGPT auth, account, and session cookies are never accepted or
returned. The jar lives only as long as the account client's shared HTTP
transport and is never written to disk.

## Dependency Boundary

Keep the existing reqwest 0.11 dependency for unrelated CLI call sites. Add a
renamed reqwest 0.12 dependency used only by the Codex transport. This avoids a
repository-wide HTTP client migration while reusing the 0.12 version already
present transitively in the lockfile.

## Testing

Focused tests cover:

- allowed Cloudflare cookies are stored and returned on ChatGPT HTTPS URLs;
- non-Cloudflare cookies, unrelated hosts, and plain HTTP are rejected;
- a mixed cookie header cannot leak a ChatGPT session cookie;
- the Codex client constructor selects the dedicated transport;
- existing request-body, response-stream, error, and cancellation tests still
  pass.

Final verification runs formatting, clippy, all targets, and a release build
against the same published dependencies used by CI. An ignored live smoke test
then sends a real `gpt-5.6-sol` request through `CodexClient`; the installed
`a3s` binary is also exercised through the interactive model picker so the
original user path is covered.

## Non-Goals

- Implementing Responses-over-WebSocket in A3S.
- Persisting or importing browser cookies.
- Replacing Codex login/token-refresh ownership.
- Fixing the separate `code exec --model codex/...` static-config routing
  limitation; the reported interactive path already injects `CodexClient`.
- Migrating unrelated CLI HTTP call sites from reqwest 0.11.
