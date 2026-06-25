//! Claude Code skill discovery, loading, and the disabled-skills persistence.

/// Discover Claude Code skill directories — personal (`~/.claude/skills`),
/// project (`<ws>/.claude/skills`), and plugin-bundled (`~/.claude/plugins/**/
/// skills`) — so a3s can load Claude `SKILL.md` skills directly. a3s's skill
/// loader already understands the `<name>/SKILL.md` layout and YAML frontmatter.
/// Parse a SKILL.md's YAML frontmatter for `name` + `description`.
fn parse_skill_meta(path: &std::path::Path) -> Option<(String, String)> {
    let content = std::fs::read_to_string(path).ok()?;
    let rest = content.trim_start().strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let (mut name, mut desc) = (None, None);
    for line in rest[..end].lines() {
        if let Some(v) = line.strip_prefix("name:") {
            name = Some(v.trim().trim_matches(['"', '\'']).to_string());
        } else if let Some(v) = line.strip_prefix("description:") {
            desc = Some(v.trim().trim_matches(['"', '\'']).to_string());
        }
    }
    let name = name?;
    if name.is_empty() {
        return None;
    }
    Some((name, desc.unwrap_or_default()))
}

/// `~/.a3s/disabled_skills` — names the user has turned off via `/plugins`.
fn disabled_skills_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".a3s/disabled_skills"))
}

pub(crate) fn load_disabled_skills() -> std::collections::HashSet<String> {
    disabled_skills_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn save_disabled_skills(set: &std::collections::HashSet<String>) {
    if let Some(p) = disabled_skills_path() {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut names: Vec<&String> = set.iter().collect();
        names.sort();
        let body = names
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = std::fs::write(p, body);
    }
}

/// Load skill (name, description) pairs from the skill dirs, for the slash menu.
pub(crate) fn load_skills(dirs: &[std::path::PathBuf]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let md = if p.is_dir() {
                p.join("SKILL.md")
            } else if p.extension().and_then(|x| x.to_str()) == Some("md") {
                p.clone()
            } else {
                continue;
            };
            if md.is_file() {
                if let Some(meta) = parse_skill_meta(&md) {
                    out.push(meta);
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Count discoverable Claude skills (`<name>/SKILL.md` dirs + flat `*.md`)
/// across the skill dirs — shown on the start screen so compatibility is visible.
pub(crate) fn count_skill_files(dirs: &[std::path::PathBuf]) -> usize {
    let mut n = 0;
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                let p = e.path();
                let is_skill_dir = p.is_dir() && p.join("SKILL.md").is_file();
                let is_flat_md = p.extension().and_then(|x| x.to_str()) == Some("md");
                if is_skill_dir || is_flat_md {
                    n += 1;
                }
            }
        }
    }
    n
}

pub(crate) fn claude_skill_dirs(workspace: &str) -> Vec<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    let project = std::path::Path::new(workspace).join(".claude/skills");
    if project.is_dir() {
        dirs.push(project);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        let personal = home.join(".claude/skills");
        if personal.is_dir() {
            dirs.push(personal);
        }
        // Depth 6 covers nested plugin layouts: plugins/cache/<plugin>/<plugin>/
        // <version>/skills and marketplaces/<mkt>/external_plugins/<plugin>/skills.
        collect_skills_dirs(&home.join(".claude/plugins"), 0, 6, &mut dirs);
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Recursively collect directories literally named `skills` (Claude plugins
/// bundle their skills there), bounded in depth and skipping dotfiles.
fn collect_skills_dirs(
    dir: &std::path::Path,
    depth: usize,
    max: usize,
    out: &mut Vec<std::path::PathBuf>,
) {
    if depth > max {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == "skills" {
            out.push(p);
        } else if !name.starts_with('.') && name != "node_modules" {
            collect_skills_dirs(&p, depth + 1, max, out);
        }
    }
}
