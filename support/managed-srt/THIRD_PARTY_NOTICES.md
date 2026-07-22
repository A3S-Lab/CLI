# Managed Sandbox Support Notices

Official A3S CLI release archives include the following fixed npm packages as
local command-sandbox support:

| Package | Version | License |
| --- | --- | --- |
| `@anthropic-ai/sandbox-runtime` | 0.0.66 | Apache-2.0 |
| `@pondwader/socks5-server` | 1.0.10 | MIT |
| `commander` | 12.1.0 | MIT |
| `node-forge` | 1.4.0 | BSD-3-Clause OR GPL-2.0 |
| `zod` | 3.25.76 | MIT |

The complete license text for each package is preserved in that package's
directory under `node_modules`.

A3S applies a narrow Linux compatibility patch to sandbox-runtime 0.0.66. The
patch orders the runtime's mandatory child-path mounts before A3S's stricter
parent-directory denies, preventing bubblewrap startup failures without
weakening either deny boundary. The modified file remains under Apache-2.0.
