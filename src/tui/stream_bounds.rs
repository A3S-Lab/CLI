//! UTF-8-safe memory bounds for model text retained during one TUI turn.

pub(crate) const MAX_ASSISTANT_STREAM_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_REASONING_STREAM_BYTES: usize = 1024 * 1024;
pub(crate) const ASSISTANT_STREAM_TRUNCATION: &str =
    "\n\n[assistant output truncated at 4 MiB by A3S Code]\n";
const REASONING_STREAM_TRUNCATION: &str = "\n\n[reasoning output truncated at 1 MiB by A3S Code]\n";

pub(crate) fn append_assistant_text(target: &mut String, delta: &str) -> bool {
    append_bounded_text(
        target,
        delta,
        MAX_ASSISTANT_STREAM_BYTES,
        ASSISTANT_STREAM_TRUNCATION,
    )
}

pub(crate) fn append_reasoning_text(target: &mut String, delta: &str) -> bool {
    append_bounded_text(
        target,
        delta,
        MAX_REASONING_STREAM_BYTES,
        REASONING_STREAM_TRUNCATION,
    )
}

/// Append one source delta and return whether it was retained completely.
///
/// Once truncated, the marker remains the terminal suffix and later deltas are
/// ignored. If an earlier delta filled the complete limit, enough UTF-8 text is
/// removed from its tail to make the truncation marker visible.
pub(crate) fn append_bounded_text(
    target: &mut String,
    delta: &str,
    max_bytes: usize,
    marker: &str,
) -> bool {
    if target.ends_with(marker) {
        return false;
    }
    if target.len().saturating_add(delta.len()) <= max_bytes {
        target.push_str(delta);
        return true;
    }

    let marker_end = utf8_prefix_len(marker, max_bytes);
    let marker = &marker[..marker_end];
    let content_limit = max_bytes.saturating_sub(marker.len());
    let retained_target = utf8_prefix_len(target, content_limit);
    target.truncate(retained_target);
    let remaining = content_limit.saturating_sub(target.len());
    let retained_delta = utf8_prefix_len(delta, remaining);
    target.push_str(&delta[..retained_delta]);
    target.push_str(marker);
    false
}

fn utf8_prefix_len(value: &str, max_bytes: usize) -> usize {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_append_is_utf8_safe_and_stops_after_the_marker() {
        let mut target = "界".repeat(8);
        let marker = "\n[truncated]\n";

        assert!(!append_bounded_text(&mut target, "more", 26, marker));
        assert!(target.len() <= 26);
        assert!(target.ends_with(marker));
        let settled = target.clone();
        assert!(!append_bounded_text(&mut target, "ignored", 26, marker));
        assert_eq!(target, settled);
    }
}
