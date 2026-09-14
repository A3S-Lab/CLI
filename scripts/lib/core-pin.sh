# Shared local-vs-published Core pin check.
# Source from crates/cli scripts. cwd must be the CLI crate root.
#
# A local pin compiles this tree. It is not a published crates.io or git pin.
# Either of these is local:
# - .cargo/config.toml path-patches ../code/core
# - Cargo.toml depends on a3s-code-core via path = "../code/core"

core_pin_is_local() {
  if [[ -f .cargo/config.toml ]] && rg -q 'path = "../code/core"' .cargo/config.toml; then
    return 0
  fi
  if [[ -f Cargo.toml ]] && rg -q 'a3s-code-core = \{[^}]*path = "\.\./code/core"' Cargo.toml; then
    return 0
  fi
  return 1
}

core_pin_local_reason() {
  if [[ -f .cargo/config.toml ]] && rg -q 'path = "../code/core"' .cargo/config.toml; then
    echo ".cargo/config.toml path-patches ../code/core"
    return 0
  fi
  if [[ -f Cargo.toml ]] && rg -q 'path = "../code/core"' Cargo.toml; then
    echo "Cargo.toml a3s-code-core uses path = \"../code/core\""
    return 0
  fi
  echo "local Core path pin"
}
