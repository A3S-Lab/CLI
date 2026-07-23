# Managed Sandbox Support Notices

Official A3S CLI release archives include the following fixed npm packages as
local command-sandbox support:

| Package | Version | License |
| --- | --- | --- |
| `@anthropic-ai/sandbox-runtime` | 0.0.67 | Apache-2.0 |
| `@pondwader/socks5-server` | 1.0.10 | MIT |
| `commander` | 12.1.0 | MIT |
| `node-forge` | 1.4.0 | BSD-3-Clause OR GPL-2.0 |
| `zod` | 3.25.76 | MIT |

The complete license text for each package is preserved in that package's
directory under `node_modules`.

A3S applies narrow compatibility patches to sandbox-runtime 0.0.67. The Linux
patch orders the runtime's mandatory child-path mounts before A3S's stricter
parent-directory denies, collapses mounts that share a missing ancestor, and
preserves read-only access to the runtime's own seccomp helper when the
surrounding user home is hidden. The macOS patch writes each generated
Seatbelt profile to a private mode-0600 temporary file and invokes
`sandbox-exec -f`, avoiding the operating system argument-size limit without
weakening the profile. The modified files remain under Apache-2.0.
