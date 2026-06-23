# a3s

The umbrella CLI for the [A3S](https://github.com/A3S-Lab) platform.

`a3s <tool> [args...]` runs the matching `a3s-<tool>` binary on your `PATH`, the
same way `git foo` runs `git-foo`. It depends on nothing and dispatches to
whatever A3S tools you have installed:

```
a3s code            # → a3s-code   (launches its TUI)
a3s box ps          # → a3s-box ps
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

Then install the tools you want (`a3s-code`, `a3s-box`, …) and run `a3s <tool>`.

## License

MIT
