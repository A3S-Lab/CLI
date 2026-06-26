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

## Updating

In the TUI, **`/update`** upgrades to the latest release and restarts into your
session — Homebrew installs are refreshed + upgraded; standalone installs swap
the binary directly.

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
