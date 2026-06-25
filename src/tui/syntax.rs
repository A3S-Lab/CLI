//! Lightweight per-line syntax highlighting → ANSI, plus the `/theme` palettes.

use a3s_tui::style::{Color, Style};

pub(crate) fn lang_of(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "rs" => "rust",
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" => "js",
        "py" => "python",
        "go" => "go",
        "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" => "c",
        "sh" | "bash" | "zsh" => "sh",
        "toml" => "toml",
        _ => "",
    }
}

/// Keyword set per coarse language.
fn keywords(lang: &str) -> &'static [&'static str] {
    match lang {
        "rust" => &[
            "fn", "let", "mut", "pub", "struct", "enum", "impl", "trait", "use", "mod", "match",
            "if", "else", "for", "while", "loop", "return", "const", "static", "async", "await",
            "move", "ref", "where", "as", "in", "crate", "super", "self", "Self", "type", "dyn",
            "unsafe", "extern", "break", "continue", "true", "false",
        ],
        "js" => &[
            "function",
            "const",
            "let",
            "var",
            "return",
            "if",
            "else",
            "for",
            "while",
            "class",
            "extends",
            "new",
            "async",
            "await",
            "import",
            "export",
            "from",
            "default",
            "try",
            "catch",
            "throw",
            "this",
            "typeof",
            "of",
            "in",
            "switch",
            "case",
            "break",
            "continue",
            "null",
            "undefined",
            "true",
            "false",
        ],
        "python" => &[
            "def", "class", "return", "if", "elif", "else", "for", "while", "import", "from", "as",
            "try", "except", "finally", "with", "lambda", "yield", "async", "await", "pass",
            "break", "continue", "raise", "global", "None", "True", "False", "and", "or", "not",
            "in", "is",
        ],
        "go" => &[
            "func",
            "var",
            "const",
            "type",
            "struct",
            "interface",
            "map",
            "chan",
            "go",
            "defer",
            "return",
            "if",
            "else",
            "for",
            "range",
            "switch",
            "case",
            "break",
            "continue",
            "package",
            "import",
            "nil",
            "true",
            "false",
        ],
        "c" => &[
            "int", "char", "void", "float", "double", "long", "short", "unsigned", "struct",
            "enum", "union", "const", "static", "return", "if", "else", "for", "while", "switch",
            "case", "break", "continue", "sizeof", "typedef",
        ],
        _ => &[],
    }
}

/// Lightweight per-line syntax highlighting → ANSI. Handles comments, strings,
/// numbers, keywords, types (CamelCase) and call sites. Single-line only.
/// Syntax-highlight palette for the IDE editor + diffs (`/theme` cycles these).
pub(crate) struct SyntaxTheme {
    pub(crate) name: &'static str,
    comment: Color,
    string: Color,
    number: Color,
    keyword: Color,
    typ: Color,
    func: Color,
}

/// Built-in themes; index 0 (Atom One Dark) is the default.
pub(crate) const THEMES: &[SyntaxTheme] = &[
    SyntaxTheme {
        name: "Atom One Dark",
        comment: Color::Rgb(92, 99, 112),
        string: Color::Rgb(152, 195, 121),
        number: Color::Rgb(209, 154, 102),
        keyword: Color::Rgb(198, 120, 221),
        typ: Color::Rgb(229, 192, 123),
        func: Color::Rgb(97, 175, 239),
    },
    SyntaxTheme {
        name: "Dracula",
        comment: Color::Rgb(98, 114, 164),
        string: Color::Rgb(241, 250, 140),
        number: Color::Rgb(189, 147, 249),
        keyword: Color::Rgb(255, 121, 198),
        typ: Color::Rgb(139, 233, 253),
        func: Color::Rgb(80, 250, 123),
    },
    SyntaxTheme {
        name: "Classic",
        comment: Color::BrightBlack,
        string: Color::Green,
        number: Color::Cyan,
        keyword: Color::Magenta,
        typ: Color::Yellow,
        func: Color::Blue,
    },
];

pub(crate) static SYNTAX_THEME: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn current_theme() -> &'static SyntaxTheme {
    let i = SYNTAX_THEME
        .load(std::sync::atomic::Ordering::Relaxed)
        .min(THEMES.len() - 1);
    &THEMES[i]
}

pub(crate) fn highlight_code(line: &str, lang: &str) -> String {
    highlight_with(line, lang, current_theme())
}

pub(crate) fn highlight_with(line: &str, lang: &str, th: &SyntaxTheme) -> String {
    if lang.is_empty() {
        return line.to_string();
    }
    let kw = keywords(lang);
    let line_comment: &str = match lang {
        "python" | "sh" | "toml" => "#",
        "rust" | "js" | "go" | "c" => "//",
        _ => "",
    };
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Line comment → rest of the line.
        let is_comment = match line_comment {
            "//" => c == '/' && chars.get(i + 1) == Some(&'/'),
            "#" => c == '#',
            _ => false,
        };
        if is_comment {
            let rest: String = chars[i..].iter().collect();
            out.push_str(&Style::new().fg(th.comment).render(&rest));
            break;
        }
        // String literal.
        if c == '"' || c == '\'' || c == '`' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != c {
                if chars[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            out.push_str(&Style::new().fg(th.string).render(&s));
            continue;
        }
        // Number.
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '.' || chars[i] == '_')
            {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            out.push_str(&Style::new().fg(th.number).render(&s));
            continue;
        }
        // Identifier / keyword / type / call.
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let styled = if kw.contains(&word.as_str()) {
                Style::new().fg(th.keyword).render(&word)
            } else if chars.get(i) == Some(&'(') {
                Style::new().fg(th.func).render(&word)
            } else if word.chars().next().is_some_and(|c| c.is_uppercase()) {
                Style::new().fg(th.typ).render(&word)
            } else {
                word
            };
            out.push_str(&styled);
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

pub(crate) fn lang_from_path(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next()?;
    Some(match ext {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "go" => "go",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" => "markdown",
        "sh" | "bash" => "bash",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "java" => "java",
        "rb" => "ruby",
        "html" => "html",
        "css" => "css",
        "sql" => "sql",
        _ => return None,
    })
}
