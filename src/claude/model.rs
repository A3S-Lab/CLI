pub(crate) fn canonical_model_name(model: &str) -> String {
    let trimmed = model.trim();
    if !trimmed.starts_with("claude") {
        return trimmed.to_string();
    }
    let Some(open_bracket) = trimmed.rfind('[') else {
        return trimmed.to_string();
    };
    if trimmed.ends_with(']') {
        let stripped = trimmed[..open_bracket].trim_end();
        if !stripped.is_empty() {
            return stripped.to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_model_name_strips_claude_code_context_suffix() {
        assert_eq!(
            canonical_model_name(" claude-opus-4-8[1m] "),
            "claude-opus-4-8"
        );
        assert_eq!(
            canonical_model_name("claude-opus-4-8 [1m]"),
            "claude-opus-4-8"
        );
        assert_eq!(
            canonical_model_name("claude-sonnet-4-6"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            canonical_model_name("openai/gpt-5[preview]"),
            "openai/gpt-5[preview]"
        );
    }
}
