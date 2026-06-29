use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

pub struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    pub fn new(name: &str) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("a3s-cli-{name}-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap_or_else(|err| {
            panic!("failed to create temp workspace {}: {err}", root.display())
        });
        Self { root }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub fn a3s_bin() -> &'static str {
    env!("CARGO_BIN_EXE_a3s")
}

pub fn host_supports_standalone_box_asset() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
        || cfg!(all(target_os = "linux", target_arch = "aarch64"))
        || cfg!(all(target_os = "linux", target_arch = "x86_64"))
}

pub fn make_executable(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap_or_else(|err| {
            panic!(
                "failed to create parent directory {}: {err}",
                parent.display()
            )
        });
    }
    std::fs::write(path, body)
        .unwrap_or_else(|err| panic!("failed to write executable {}: {err}", path.display()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|err| panic!("failed to chmod executable {}: {err}", path.display()));
    }
}

pub fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

pub fn install_fake_download_tools(
    tool_dir: &Path,
    curl_log: &Path,
    installed_args_log: Option<&Path>,
) {
    let curl_log = sh_quote(curl_log);
    make_executable(
        &tool_dir.join("curl"),
        &format!(
            r#"#!/bin/sh
printf 'curl:%s\n' "$*" >> {curl_log}
case "$*" in
  *"/releases/latest"*)
    printf '%s\n' 'https://github.com/A3S-Lab/Box/releases/tag/v2.5.2'
    exit 0
    ;;
esac

out=''
prev=''
for arg in "$@"; do
  if [ "$prev" = '-o' ] || [ "$prev" = '--output' ]; then
    out="$arg"
  fi
  prev="$arg"
done

if [ -z "$out" ]; then
  printf 'missing curl output path\n' >&2
  exit 2
fi

printf '#### download progress 100.0%%\n' >&2
printf 'fake tarball\n' > "$out"
exit 0
"#
        ),
    );

    let installed_log_line = installed_args_log
        .map(|path| format!("printf '%s\\n' \"$@\" >> {}\n", sh_quote(path)))
        .unwrap_or_default();
    make_executable(
        &tool_dir.join("tar"),
        &format!(
            r#"#!/bin/sh
dest=''
prev=''
for arg in "$@"; do
  if [ "$prev" = '-C' ]; then
    dest="$arg"
  fi
  prev="$arg"
done

if [ -z "$dest" ]; then
  printf 'missing tar destination\n' >&2
  exit 2
fi

/bin/mkdir -p "$dest"
/bin/cat > "$dest/a3s-box" <<'A3S_BOX_SCRIPT'
#!/bin/sh
{installed_log_line}printf 'installed-box:%s\n' "$*"
exit 0
A3S_BOX_SCRIPT
/bin/chmod +x "$dest/a3s-box"
exit 0
"#
        ),
    );
}
