use super::*;

pub(super) const IDE_INTELLIGENCE_QUERY_MAX_CHARS: usize = 256;
pub(super) const IDE_INTELLIGENCE_MAX_ROWS: usize = 2_000;
pub(super) const IDE_INTELLIGENCE_MAX_SYMBOL_DEPTH: usize = 32;
pub(super) const IDE_INTELLIGENCE_TITLE_MAX_CHARS: usize = 240;
pub(super) const IDE_INTELLIGENCE_ROW_MAX_CHARS: usize = 1_200;
pub(super) const IDE_INTELLIGENCE_ERROR_MAX_CHARS: usize = 2_000;
pub(super) const IDE_INTELLIGENCE_STATUS_MESSAGE_MAX_CHARS: usize = 800;
pub(super) const IDE_INTELLIGENCE_SYMBOL_NAME_MAX_CHARS: usize = 256;
pub(super) const IDE_INTELLIGENCE_SYMBOL_DETAIL_MAX_CHARS: usize = 512;
pub(super) const IDE_INTELLIGENCE_LANGUAGE_MAX_CHARS: usize = 64;
pub(super) const IDE_INTELLIGENCE_PATH_MAX_CHARS: usize = 512;
pub(super) const IDE_INTELLIGENCE_DIAGNOSTIC_MESSAGE_MAX_CHARS: usize = 800;
pub(super) const IDE_INTELLIGENCE_DIAGNOSTIC_ORIGIN_MAX_CHARS: usize = 256;

pub(super) fn sanitize_ide_intelligence_field(value: &str, max_chars: usize) -> String {
    crate::system_agents::sanitize_display_text(value, max_chars)
}

pub(super) fn sanitize_ide_intelligence_nonempty(
    value: &str,
    max_chars: usize,
    fallback: &str,
) -> String {
    let value = sanitize_ide_intelligence_field(value, max_chars);
    if value.is_empty() {
        fallback.to_owned()
    } else {
        value
    }
}

pub(super) fn sanitize_ide_intelligence_title(value: &str) -> String {
    sanitize_ide_intelligence_nonempty(value, IDE_INTELLIGENCE_TITLE_MAX_CHARS, "Code Intelligence")
}

pub(super) fn sanitize_ide_intelligence_row(value: &str) -> String {
    sanitize_ide_intelligence_nonempty(
        value,
        IDE_INTELLIGENCE_ROW_MAX_CHARS,
        "No displayable result.",
    )
}

pub(super) fn sanitize_ide_intelligence_error(value: &str) -> String {
    sanitize_ide_intelligence_nonempty(
        value,
        IDE_INTELLIGENCE_ERROR_MAX_CHARS,
        "Code Intelligence returned an unspecified error",
    )
}

pub(super) fn sanitize_ide_intelligence_path(value: &str) -> String {
    sanitize_ide_intelligence_nonempty(value, IDE_INTELLIGENCE_PATH_MAX_CHARS, "workspace path")
}

pub(super) fn sanitize_ide_intelligence_result(
    mut result: IdeIntelligenceResult,
) -> IdeIntelligenceResult {
    result.title = sanitize_ide_intelligence_title(&result.title);
    if result.rows.len() > IDE_INTELLIGENCE_MAX_ROWS {
        result.rows.truncate(IDE_INTELLIGENCE_MAX_ROWS);
        result.truncated = true;
    }
    for row in &mut result.rows {
        row.text = sanitize_ide_intelligence_row(&row.text);
    }
    result
}

pub(super) fn ide_intelligence_flash_line(kind: ToastKind, message: impl AsRef<str>) -> String {
    ide_flash_line(kind, sanitize_ide_intelligence_error(message.as_ref()))
}
