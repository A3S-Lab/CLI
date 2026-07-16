//! Extract structured evidence JSON embedded in model or tool text.

pub(crate) fn parse_embedded_structured_evidence_json(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.len() < 8 {
        return None;
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            return Some(value);
        }
    }
    if !(trimmed.contains("\"summary\"") && trimmed.contains("\"sources\"")) {
        return None;
    }
    for (start, ch) in trimmed.char_indices() {
        if !matches!(ch, '{' | '[') {
            continue;
        }
        let Some(end) = balanced_json_end(trimmed, start) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&trimmed[start..end]) {
            return Some(value);
        }
    }
    None
}

fn balanced_json_end(text: &str, start: usize) -> Option<usize> {
    let opener = text.get(start..)?.chars().next()?;
    let expected = match opener {
        '{' => '}',
        '[' => ']',
        _ => return None,
    };
    let mut stack = vec![expected];
    let mut in_string = false;
    let mut escaped = false;

    for (offset, ch) in text[start..].char_indices().skip(1) {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' if stack.pop() == Some(ch) => {
                if stack.is_empty() {
                    return Some(start + offset + ch.len_utf8());
                }
            }
            '}' | ']' => return None,
            _ => {}
        }
    }
    None
}
