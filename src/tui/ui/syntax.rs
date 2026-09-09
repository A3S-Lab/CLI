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
/// Syntax-highlight palette for the IDE editor (`/theme` cycles these).
/// Diff rendering uses the fixed GitHub Dark reference palette below.
pub(crate) struct SyntaxTheme {
    pub(crate) name: &'static str,
    comment: Color,
    string: Color,
    number: Color,
    keyword: Color,
    typ: Color,
    func: Color,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyntaxSpan {
    pub(crate) content: String,
    pub(crate) color: Option<Color>,
}

// GitHub Dark (Primer) syntax tokens.
const GH_COMMENT: Color = Color::Rgb(139, 148, 158); // #8b949e
const GH_STRING: Color = Color::Rgb(165, 214, 255); // #a5d6ff
const GH_NUMBER: Color = Color::Rgb(121, 192, 255); // #79c0ff
const GH_KEYWORD: Color = Color::Rgb(255, 123, 114); // #ff7b72
const GH_TYPE: Color = Color::Rgb(255, 166, 87); // #ffa657
const GH_FUNCTION: Color = Color::Rgb(88, 166, 255); // #58a6ff

const DIFF_THEME: SyntaxTheme = SyntaxTheme {
    name: "GitHub Diff",
    comment: GH_COMMENT,
    string: GH_STRING,
    number: GH_NUMBER,
    keyword: GH_KEYWORD,
    typ: GH_TYPE,
    func: GH_FUNCTION,
};

/// Built-in themes; index 0 (GitHub Dark) is the default.
pub(crate) const THEMES: &[SyntaxTheme] = &[
    SyntaxTheme {
        name: "GitHub Dark",
        comment: GH_COMMENT,
        string: GH_STRING,
        number: GH_NUMBER,
        keyword: GH_KEYWORD,
        typ: GH_TYPE,
        func: GH_FUNCTION,
    },
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
    highlight_spans_with(line, lang, th)
        .into_iter()
        .map(|span| match span.color {
            Some(color) => Style::new().fg(color).render(&span.content),
            None => span.content,
        })
        .collect()
}

pub(crate) fn highlight_diff_spans(line: &str, lang: &str) -> Vec<SyntaxSpan> {
    highlight_spans_with(line, lang, &DIFF_THEME)
}

fn highlight_spans_with(line: &str, lang: &str, th: &SyntaxTheme) -> Vec<SyntaxSpan> {
    if lang.is_empty() {
        return vec![SyntaxSpan {
            content: line.to_string(),
            color: None,
        }];
    }
    let kw = keywords(lang);
    let line_comment: &str = match lang {
        "python" | "sh" | "toml" => "#",
        "rust" | "js" | "go" | "c" => "//",
        _ => "",
    };
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
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
            push_span(&mut out, rest, Some(th.comment));
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
            push_span(&mut out, s, Some(th.string));
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
            push_span(&mut out, s, Some(th.number));
            continue;
        }
        // Identifier / keyword / type / call.
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let color = if kw.contains(&word.as_str()) {
                Some(th.keyword)
            } else if chars.get(i) == Some(&'(') {
                Some(th.func)
            } else if word.chars().next().is_some_and(|c| c.is_uppercase()) {
                Some(th.typ)
            } else {
                None
            };
            push_span(&mut out, word, color);
            continue;
        }
        push_span(&mut out, c.to_string(), None);
        i += 1;
    }
    out
}

fn push_span(spans: &mut Vec<SyntaxSpan>, content: String, color: Option<Color>) {
    if let Some(last) = spans.last_mut().filter(|span| span.color == color) {
        last.content.push_str(&content);
    } else {
        spans.push(SyntaxSpan { content, color });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_dark_is_the_default_and_matches_diff_syntax_tokens() {
        let default = &THEMES[0];
        assert_eq!(default.name, "GitHub Dark");
        assert_eq!(default.comment, DIFF_THEME.comment);
        assert_eq!(default.string, DIFF_THEME.string);
        assert_eq!(default.number, DIFF_THEME.number);
        assert_eq!(default.keyword, DIFF_THEME.keyword);
        assert_eq!(default.typ, DIFF_THEME.typ);
        assert_eq!(default.func, DIFF_THEME.func);
    }

    #[test]
    fn github_tokens_match_github_dark_diff_palette() {
        assert_eq!(GH_COMMENT, Color::Rgb(139, 148, 158));
        assert_eq!(GH_STRING, Color::Rgb(165, 214, 255));
        assert_eq!(GH_NUMBER, Color::Rgb(121, 192, 255));
        assert_eq!(GH_KEYWORD, Color::Rgb(255, 123, 114));
        assert_eq!(GH_TYPE, Color::Rgb(255, 166, 87));
        assert_eq!(GH_FUNCTION, Color::Rgb(88, 166, 255));
    }
}
