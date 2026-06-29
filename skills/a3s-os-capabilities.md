---
name: a3s-os-capabilities
description: "书安OS progressive API — the way to answer ANY question about the signed-in 书安OS platform: your platform account/identity, what the platform can do, and its data / resources / state (LLM/OCR, assets, packages, runtime, knowledge, observability, …). One action-dispatched endpoint, broad-to-narrow: list -> search -> describe -> execute. Use this — NOT the local shell (whoami/paths) — for anything about the OS platform. Available only when signed in."
kind: instruction
allowed-tools: "bash(*), read(*)"
---

# 书安OS progressive API (渐进式 API)

Discover and call the 书安OS platform capabilities you are authorized for,
*progressively* (broad → narrow), through **one** endpoint. This is the whole
platform's capability surface — not just one domain: AI capabilities (LLM/OCR
config), assets, packages, registry, runtime, resources, knowledge bases,
observability, marketplace, and more. Security is one domain among these.

Use this skill for **any** question about the OS platform — when the user says
"OS" they mean this signed-in platform, not the local operating system. That
includes your platform **account / identity** ("what's my OS account?"), what the
platform can do, and its data/state ("what LLM/OCR is configured", "OCR this
PDF", "list my assets", "search platform operations for X"). Do NOT answer these
from the local shell or filesystem (`whoami`, paths, env) — those describe this
machine, not the OS platform. Start with `list` to find the right module/op.

Single endpoint (`action`-dispatched, always `POST` with a JSON body):

```
POST {{BASE_URL}}/api/v1/kernel/capabilities
```

Flow ("先广后窄" / broad-then-narrow) — like a CLI, expand on demand instead of
loading every manual into context:

```
list  →  search  →  describe  →  execute
(modules) (find op) (op schema)  (run it)
 git --help  apropos  rebase --help  rebase -i
```

## Authentication

The endpoint and Bearer token are **already in your shell environment** (exported
when you signed in) — use them directly; do NOT read `~/.a3s/os-auth.json` or any
config file on each call:

- `$A3S_OS_BASE_URL` — the platform base URL (`{{BASE_URL}}`)
- `$A3S_OS_TOKEN` — the Bearer token

Everything is permission-filtered by that token; you only ever see/run what the
signed-in user may access.

## Request

```json
{ "action": "list | search | describe | execute",
  "module":    "<module name> (describe / execute)",
  "query":     "<keywords> (search)",
  "operation": "<operation name> (execute = the op to run; describe = return just that one op's full schema)",
  "params":    { } }
```

| action | needs | returns |
| --- | --- | --- |
| `list` | — | every module you can access (name, description, path, operationCount) |
| `search` | `query` | matching operations across modules |
| `describe` | `module` (+ optional `operation`) | the module's sub-modules + its operations; or, with `operation`, just that ONE operation's full input/output schema |
| `execute` | `module`, `operation`, `params` | the operation result (`data`), plus an optional `viewUrl` deep link and `ui` agent-ui directive |

## Rules

- **Stay narrow — never dump the whole catalog.** Walk it like a CLI, one rung at
  a time, fetching only what the user's question needs: `list` (modules only) →
  pick the relevant module → `describe` it to see its **sub-modules** and
  operation counts (drill into a sub-module, don't enumerate every operation) →
  `search`/`describe <module> <operation>` for the ONE operation you'll run →
  `execute`. Show the user only the operation(s) that answer them, not every
  interface.
- **Keep output tight — extract, don't dump.** Pipe every response through `jq`
  to pull only the fields you need (e.g. `... | jq -r '.data.modules[].name'` for
  a module list, `... | jq '.data | keys'` to peek a shape) so the result is a few
  relevant lines, not a raw JSON blob. In your reply to the user, summarize in a
  few lines — do NOT paste the whole response back. (The TUI already collapses
  long tool output to ~5 lines, but a targeted `jq` result is what keeps both the
  tool line and your answer short.)
- Never guess `module`, `operation`, field names, or enums. `list` / `search` /
  `describe` first, then build `params` from the returned schema. `describe` with
  an `operation` gives that op's exact schema — the rung right before `execute`.
- On an `execute` schema error, re-`describe` that operation and fix `params`
  instead of inventing fields.
- Prefer read/`GET`-style operations for discovery; write operations (create /
  update / delete) run with the user's real platform permissions — confirm intent
  before mutating platform state.
- **Always surface `viewUrl`.** Many responses include a `viewUrl` — a deep link
  to the console page for exactly what the user asked about. WHENEVER the response
  contains one, extract it robustly (it may be top-level or nested, e.g.
  `jq -r '.. | .viewUrl? // empty'` over the response) and present it to the user
  as a clearly labeled, clickable link on its own line — e.g.
  `🔗 在控制台查看: <viewUrl>` (write the label in the user's language). The TUI
  renders bare URLs as clickable, so include the full URL. Never fabricate a
  `viewUrl` that wasn't returned, and never drop one that was.
- The `ui` field (`protocol: "agent-ui"`) is a host-rendered remote component —
  note that it exists if present, but don't try to render it yourself.

## Examples

```bash
# Endpoint + token come from the env exported at login — no config read needed.
API="$A3S_OS_BASE_URL/api/v1/kernel/capabilities"
post() { curl -s -X POST "$API" -H "Authorization: Bearer $A3S_OS_TOKEN" -H 'Content-Type: application/json' -d "$1"; }

post '{"action":"list"}'                                              # 1. what modules exist
post '{"action":"search","query":"ocr"}'                             # 2. find operations
post '{"action":"describe","module":"kernel","operation":"runOcr"}'  # 3. exact schema
```

```json
// 4a. list the system's configured LLM/OCR capabilities (masked projection)
{ "action": "execute", "module": "kernel", "operation": "listAiCaps" }
```

```json
// 4b. run OCR through the platform's configured backend (you never see its URL/key)
{ "action": "execute", "module": "kernel", "operation": "runOcr",
  "params": { "url": "https://…/spec.pdf", "mimeType": "application/pdf", "modelType": "document", "outputFormat": "markdown" } }
```
