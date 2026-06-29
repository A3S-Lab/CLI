# a3s

The umbrella CLI for the [A3S](https://github.com/A3S-Lab) platform.

`a3s <tool> [args...]` runs the matching A3S tool. `a3s box ...` proxies to
`a3s-box ...` and bootstraps the Box runtime automatically if it is missing:

```
a3s code            # → a3s-code   (launches its TUI)
a3s box ps          # → a3s-box ps (auto-installs a3s-box if needed)
a3s <tool> --help   # a tool's own help
a3s list            # list installed a3s-* tools
a3s --version
```

## Install

```sh
# from source
cargo install a3s

# or from this repo
cargo install --git https://github.com/A3S-Lab/Cli

# or Homebrew
brew install A3S-Lab/tap/a3s
```

Then run the tools you need. `a3s box ...` installs `a3s-box` on first use.

## Account Models

In `a3s code`, `/model` lists configured `config.acl` models plus signed-in
account tabs. When Claude Code is logged in (`claude /login`), the Claude Code
tab can switch the current session to Claude models using the local Claude Code
OAuth credentials, including Claude Code's macOS Keychain entry.
`CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_AUTH_TOKEN` can also provide the account
token for non-standard environments. If Anthropic rejects the raw OAuth Messages
API bridge with a rate-limit or authentication error, a3s falls back to the
installed `claude` CLI in safe streaming mode; Claude Code's own tools stay
disabled while a3s host tools are requested through an adapter protocol and
still execute inside a3s-code. The adapter accepts Claude Code-style
`<function_calls>` output and tool names such as `Read` or `Bash`, normalizes
common argument aliases like `path` to a3s's `file_path`, and feeds tool results
back into the next Claude turn as structured history.

## Testing

```sh
cargo test --all-targets
cargo test --test box_command_soak -- --ignored
```

The ignored soak test repeats `a3s box` after a fake first-use install and
verifies later runs reuse the installed `a3s-box`.

## Updating

In the TUI, **`/update`** upgrades to the latest release and restarts into your
session — Homebrew installs are refreshed + upgraded; standalone installs swap
the binary directly. Both paths verify the installed binary reports the target
version before treating the update as successful.

If you're on an **older build (≤ 0.5.4)** whose `/update` was broken, it can't
upgrade itself, and `brew upgrade a3s` alone won't see the new version (Homebrew
doesn't re-sync a tap on `upgrade`). Bootstrap onto a current build once with:

```sh
brew update && brew upgrade a3s     # or: brew untap a3s-lab/tap && brew tap a3s-lab/tap && brew upgrade a3s
a3s --version
```

From 0.5.5 onward, `/update` handles the tap refresh itself, so this manual step
isn't needed again.

## License

MIT
