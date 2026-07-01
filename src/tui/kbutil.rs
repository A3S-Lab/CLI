//! `/kb`: ingest raw material — typed text, a file, or a folder — into the
//! project knowledge base at `.a3s/kb/sources/`. `/okf` later compiles these
//! sources into cross-linked OKF concept pages. Ingestion is deterministic plain
//! file I/O (no LLM): it always works, never mangles the originals (files are
//! copied verbatim; provenance is logged separately in `SOURCES.md`).

use std::path::{Path, PathBuf};

/// The KB vault root (`<cwd>/.a3s/kb`). OKF concept pages live here; ingested raw
/// material lands under `sources/`.
pub(crate) fn kb_dir(cwd: &str) -> PathBuf {
    Path::new(cwd).join(".a3s").join("kb")
}

/// Cap a folder ingest so `/kb .` on a big tree can't run away.
const MAX_DIR_FILES: usize = 300;
/// Skip any single file larger than this (KB stores text notes, not blobs).
const MAX_FILE_BYTES: u64 = 1_048_576; // 1 MiB

/// Ingest `arg` into the KB and return a one-line human summary. `now` is an
/// RFC3339 timestamp (injected so the logic is testable).
pub(crate) fn add_to_kb(cwd: &str, arg: &str, now: &str) -> String {
    let arg = arg.trim();
    let sources = kb_dir(cwd).join("sources");
    if arg.is_empty() {
        let n = count_md(&kb_dir(cwd));
        return format!(
            "KB at .a3s/kb · {n} note(s). usage: /kb <text> | /kb <file> | /kb <folder>"
        );
    }
    let path = {
        let p = Path::new(arg);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            Path::new(cwd).join(p)
        }
    };
    let result = if path.is_file() {
        ingest_file(&path, &sources).map(|dest| {
            log_source(&sources, now, "file", arg, &dest);
            format!("✔ added file to KB · {}", show(cwd, &dest))
        })
    } else if path.is_dir() {
        ingest_dir(&path, &sources).map(|(added, skipped)| {
            log_source(&sources, now, "folder", arg, &sources.join(dir_name(&path)));
            let skip = if skipped > 0 {
                format!(" ({skipped} skipped)")
            } else {
                String::new()
            };
            format!(
                "✔ added {added} file(s) from {arg} to KB{skip} · .a3s/kb/sources/{}/",
                dir_name(&path)
            )
        })
    } else {
        ingest_text(arg, &sources, now)
            .map(|dest| format!("✔ captured note to KB · {}", show(cwd, &dest)))
    };
    result.unwrap_or_else(|e| format!("✗ /kb failed: {e}"))
}

/// Capture typed text as an OKF note (frontmatter + body).
fn ingest_text(text: &str, sources: &Path, now: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(sources)?;
    let title = text.lines().next().unwrap_or("note").trim();
    let dest = unique_path(&sources.join(format!("{}.md", slug(title))));
    let body = format!("---\ntype: note\nsource: user\nadded: {now}\n---\n\n{text}\n");
    std::fs::write(&dest, body)?;
    Ok(dest)
}

/// Copy one text file into the vault verbatim.
fn ingest_file(file: &Path, sources: &Path) -> std::io::Result<PathBuf> {
    if !is_text_file(file)? {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not a text file (KB stores text)",
        ));
    }
    std::fs::create_dir_all(sources)?;
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("source");
    let dest = unique_path(&sources.join(name));
    std::fs::copy(file, &dest)?;
    Ok(dest)
}

/// Copy a folder's text files into `sources/<dirname>/…`, preserving structure.
/// Skips hidden entries + `target`/`node_modules`, binaries, and oversized files.
fn ingest_dir(dir: &Path, sources: &Path) -> std::io::Result<(usize, usize)> {
    let root_dest = sources.join(dir_name(dir));
    let (mut added, mut skipped) = (0usize, 0usize);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if added >= MAX_DIR_FILES {
            break;
        }
        let rd = match std::fs::read_dir(&d) {
            Ok(r) => r,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        for entry in rd.flatten() {
            let p = entry.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || matches!(name, "target" | "node_modules") {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if added >= MAX_DIR_FILES {
                break;
            }
            if !is_text_file(&p).unwrap_or(false) {
                skipped += 1;
                continue;
            }
            let rel = p.strip_prefix(dir).unwrap_or(&p);
            let dest = root_dest.join(rel);
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::copy(&p, &dest).is_ok() {
                added += 1;
            } else {
                skipped += 1;
            }
        }
    }
    Ok((added, skipped))
}

/// A file is "text" if it's under the size cap and its first 8 KiB have no NUL.
fn is_text_file(p: &Path) -> std::io::Result<bool> {
    if std::fs::metadata(p)?.len() > MAX_FILE_BYTES {
        return Ok(false);
    }
    use std::io::Read;
    let mut buf = [0u8; 8192];
    let n = std::fs::File::open(p)?.read(&mut buf)?;
    Ok(!buf[..n].contains(&0))
}

/// Append a provenance line to `sources/SOURCES.md` (copied files carry no
/// frontmatter, so this is where their origin is recorded).
fn log_source(sources: &Path, now: &str, kind: &str, origin: &str, dest: &Path) {
    let _ = std::fs::create_dir_all(sources);
    let name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let line = format!("- {now} · {kind} · {origin} → {name}\n");
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(sources.join("SOURCES.md"))
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Count `.md` files under the vault (recursive) for the status line.
fn count_md(kb: &Path) -> usize {
    let mut n = 0;
    let mut stack = vec![kb.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().and_then(|x| x.to_str()) == Some("md") {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Filesystem-safe slug from a title (keeps unicode letters/digits, e.g. CJK).
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(48).collect();
    if out.is_empty() {
        "note".to_string()
    } else {
        out
    }
}

/// Return `p`, or `p` with a `-2`/`-3`/… suffix if it already exists.
fn unique_path(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = p.extension().and_then(|e| e.to_str());
    let parent = p.parent().unwrap_or_else(|| Path::new("."));
    for i in 2..10_000 {
        let name = match ext {
            Some(e) => format!("{stem}-{i}.{e}"),
            None => format!("{stem}-{i}"),
        };
        let cand = parent.join(name);
        if !cand.exists() {
            return cand;
        }
    }
    p.to_path_buf()
}

fn dir_name(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("folder")
        .to_string()
}

fn show(cwd: &str, p: &Path) -> String {
    p.strip_prefix(cwd).unwrap_or(p).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        // Unique per call (atomic counter) so parallel tests never share a dir.
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let id = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("a3s-kb-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn captures_typed_text_as_a_note() {
        let cwd = tmp();
        let cwds = cwd.to_str().unwrap();
        let out = add_to_kb(
            cwds,
            "Decision: use HCL over TOML for config",
            "2026-07-01T00:00:00Z",
        );
        assert!(out.contains("captured note"), "{out}");
        // A single .md note landed under sources with frontmatter + body.
        let src = kb_dir(cwds).join("sources");
        let note = std::fs::read_dir(&src)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
            .unwrap();
        let body = std::fs::read_to_string(&note).unwrap();
        assert!(body.contains("type: note") && body.contains("source: user"));
        assert!(body.contains("use HCL over TOML"));
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn ingests_a_file_verbatim() {
        let cwd = tmp();
        let cwds = cwd.to_str().unwrap();
        let f = cwd.join("notes.txt");
        std::fs::write(&f, "hello kb").unwrap();
        let out = add_to_kb(cwds, f.to_str().unwrap(), "2026-07-01T00:00:00Z");
        assert!(out.contains("added file"), "{out}");
        let copied = kb_dir(cwds).join("sources").join("notes.txt");
        assert_eq!(std::fs::read_to_string(&copied).unwrap(), "hello kb"); // verbatim
        assert!(kb_dir(cwds).join("sources/SOURCES.md").exists()); // provenance logged
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn ingests_folder_text_files_and_skips_binary() {
        let cwd = tmp();
        let cwds = cwd.to_str().unwrap();
        let dir = cwd.join("docs");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.md"), "alpha").unwrap();
        std::fs::write(dir.join("sub/b.md"), "beta").unwrap();
        std::fs::write(dir.join("bin.dat"), [0u8, 1, 2, 0]).unwrap(); // binary → skipped
        let out = add_to_kb(cwds, dir.to_str().unwrap(), "2026-07-01T00:00:00Z");
        assert!(out.contains("added 2 file(s)"), "{out}");
        let base = kb_dir(cwds).join("sources/docs");
        assert!(base.join("a.md").exists() && base.join("sub/b.md").exists());
        assert!(!base.join("bin.dat").exists()); // binary excluded
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn empty_arg_reports_status() {
        let cwd = tmp();
        let out = add_to_kb(cwd.to_str().unwrap(), "  ", "2026-07-01T00:00:00Z");
        assert!(out.contains("KB at .a3s/kb") && out.contains("usage:"));
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn slug_keeps_unicode_and_dedupes() {
        assert_eq!(slug("Hello, World!"), "hello-world");
        assert_eq!(slug("用 HCL 配置"), "用-hcl-配置");
        assert_eq!(slug("   "), "note");
    }
}
