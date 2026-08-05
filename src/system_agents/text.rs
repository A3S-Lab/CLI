//! Bounded, terminal-safe text used by cross-process agent status.

use std::ffi::OsStr;
use std::path::Path;

use super::MAX_WORKSPACE_CHARS;

pub(super) fn workspace_basename(workspace: &str) -> String {
    let basename = Path::new(workspace)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("workspace");
    sanitize_nonempty(basename, MAX_WORKSPACE_CHARS, "workspace")
}

pub(super) fn sanitize_optional(value: Option<&str>, max_chars: usize) -> Option<String> {
    value
        .map(|value| sanitize_display_text(value, max_chars))
        .filter(|value| !value.is_empty())
}

pub(super) fn sanitize_nonempty(value: &str, max_chars: usize, fallback: &str) -> String {
    let value = sanitize_display_text(value, max_chars);
    if value.is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

/// Convert untrusted terminal-facing text into a bounded, single-line label.
///
/// This strips complete ANSI/ECMA-48 control strings, C0/C1 controls, and
/// bidirectional formatting controls while preserving ordinary Unicode text.
pub(crate) fn sanitize_display_text(value: &str, max_chars: usize) -> String {
    sanitize_terminal_text(value, max_chars, WhitespaceMode::SingleLine)
}

/// Convert untrusted terminal-facing text into bounded safe text while
/// preserving line boundaries. This shares the exact ANSI, C0/C1, and bidi
/// filtering state machine used by [`sanitize_display_text`].
pub(crate) fn sanitize_multiline_text(value: &str, max_chars: usize) -> String {
    sanitize_terminal_text(value, max_chars, WhitespaceMode::CollapsedLines)
}

/// Convert terminal-facing text into bounded safe text without changing its
/// ordinary spaces or line structure. Tabs advance to a four-character stop;
/// every emitted space still counts against `max_chars`.
pub(crate) fn sanitize_terminal_layout(value: &str, max_chars: usize) -> String {
    sanitize_terminal_text(value, max_chars, WhitespaceMode::PreserveLayout)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WhitespaceMode {
    SingleLine,
    CollapsedLines,
    PreserveLayout,
}

impl WhitespaceMode {
    fn preserves_newlines(self) -> bool {
        matches!(self, Self::CollapsedLines | Self::PreserveLayout)
    }
}

fn sanitize_terminal_text(value: &str, max_chars: usize, whitespace: WhitespaceMode) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Text,
        Escape,
        EscapeIntermediate,
        Csi,
        ControlString,
        ControlStringEscape,
    }

    let mut state = State::Text;
    let mut output = String::new();
    let mut output_chars = 0usize;
    let mut pending_space = false;
    let mut layout_column = 0usize;

    'input: for character in value.chars() {
        match state {
            State::Escape => {
                state = match character {
                    '[' => State::Csi,
                    ']' | 'P' | 'X' | '^' | '_' => State::ControlString,
                    '\u{20}'..='\u{2f}' => State::EscapeIntermediate,
                    _ => State::Text,
                };
                continue;
            }
            State::EscapeIntermediate => {
                if ('\u{30}'..='\u{7e}').contains(&character) {
                    state = State::Text;
                }
                continue;
            }
            State::Csi => {
                if ('\u{40}'..='\u{7e}').contains(&character) {
                    state = State::Text;
                }
                continue;
            }
            State::ControlString => {
                state = match character {
                    '\u{7}' | '\u{9c}' => State::Text,
                    '\u{1b}' => State::ControlStringEscape,
                    _ => State::ControlString,
                };
                continue;
            }
            State::ControlStringEscape => {
                state = if character == '\\' {
                    State::Text
                } else if character == '\u{1b}' {
                    State::ControlStringEscape
                } else {
                    State::ControlString
                };
                continue;
            }
            State::Text => {}
        }

        state = match character {
            '\u{1b}' => State::Escape,
            '\u{9b}' => State::Csi,
            '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => State::ControlString,
            _ => State::Text,
        };
        if !matches!(state, State::Text) {
            continue;
        }

        if whitespace.preserves_newlines() && character == '\n' {
            if output_chars >= max_chars {
                break;
            }
            output.push('\n');
            output_chars += 1;
            pending_space = false;
            layout_column = 0;
            continue;
        }

        let bidi_control = matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{206f}'
        );
        if bidi_control {
            continue;
        }

        if whitespace == WhitespaceMode::PreserveLayout {
            if character == '\t' {
                const TAB_WIDTH: usize = 4;
                let spaces = TAB_WIDTH - (layout_column % TAB_WIDTH);
                for _ in 0..spaces {
                    if output_chars >= max_chars {
                        break 'input;
                    }
                    output.push(' ');
                    output_chars += 1;
                    layout_column += 1;
                }
                continue;
            }
            if character.is_control() {
                continue;
            }
            let character = if character.is_whitespace() {
                ' '
            } else {
                character
            };
            if output_chars >= max_chars {
                break;
            }
            output.push(character);
            output_chars += 1;
            layout_column += 1;
            continue;
        }

        if character.is_control() || character.is_whitespace() {
            pending_space |= !output.is_empty() && !output.ends_with('\n');
            continue;
        }
        if output_chars >= max_chars {
            break;
        }
        if pending_space && output_chars + 1 < max_chars {
            output.push(' ');
            output_chars += 1;
        }
        pending_space = false;
        if output_chars < max_chars {
            output.push(character);
            output_chars += 1;
        }
    }
    output
}
