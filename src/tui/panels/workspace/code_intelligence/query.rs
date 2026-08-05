use super::*;
use a3s_code_core::workspace::WorkspaceFileSystem;
use a3s_code_core::{CodeDiagnostic, CodeIntelligenceStatus};

pub(super) async fn execute_ide_intelligence_query(
    provider: Arc<dyn WorkspaceCodeIntelligence>,
    file_system: Arc<dyn WorkspaceFileSystem>,
    prepared: PreparedIdeIntelligenceQuery,
    cancellation: CancellationToken,
) -> Result<IdeIntelligenceResult, String> {
    if cancellation.is_cancelled() {
        return Err("Code Intelligence query cancelled".to_owned());
    }
    let title = prepared.title;
    let saved_version = prepared.saved_version;
    let dirty_buffer = prepared.dirty_buffer;
    match prepared.task {
        IdeIntelligenceTask::Status => {
            let status = provider.status();
            Ok(status_result(title, status, dirty_buffer))
        }
        IdeIntelligenceTask::DocumentSymbols { path } => {
            let result = execute_code_intelligence_with_protocol_retry(
                "document_symbols",
                &cancellation,
                |attempt| provider.document_symbols(&path, attempt),
            )
            .await
            .map_err(|error| sanitize_ide_intelligence_error(&error.to_string()))?;
            let (rows, display_truncated) = document_symbol_rows(result.items, path.as_str());
            Ok(query_result(
                title,
                rows,
                result.truncated || display_truncated,
                saved_version,
                dirty_buffer,
                result.workspace_revision,
                result
                    .document
                    .as_ref()
                    .is_some_and(|document| document.stale),
            ))
        }
        IdeIntelligenceTask::WorkspaceSymbols { query } => {
            let result = execute_code_intelligence_with_protocol_retry(
                "workspace_symbols",
                &cancellation,
                |attempt| provider.search_symbols(&query, WORKSPACE_SYMBOL_LIMIT, attempt),
            )
            .await
            .map_err(|error| sanitize_ide_intelligence_error(&error.to_string()))?;
            let rows = result
                .items
                .into_iter()
                .take(IDE_INTELLIGENCE_MAX_ROWS.saturating_add(1))
                .map(workspace_symbol_row)
                .collect();
            Ok(query_result(
                title,
                rows,
                result.truncated,
                saved_version,
                dirty_buffer,
                result.workspace_revision,
                false,
            ))
        }
        IdeIntelligenceTask::Navigate {
            kind,
            path,
            row,
            expanded_col,
        } => {
            let saved = tokio::select! {
                _ = cancellation.cancelled() => {
                    return Err("Code Intelligence query cancelled".to_owned());
                }
                result = file_system.read_text(&path) => result.map_err(|error| {
                    sanitize_ide_intelligence_error(&format!(
                        "failed to read saved file {}: {error}",
                        sanitize_ide_intelligence_path(path.as_str())
                    ))
                })?,
            };
            let position = editor_position_to_saved_utf16(&saved, row, expanded_col)?;
            let result = execute_code_intelligence_with_protocol_retry(
                "navigation",
                &cancellation,
                |attempt| provider.navigate(kind, &path, position, attempt),
            )
            .await
            .map_err(|error| sanitize_ide_intelligence_error(&error.to_string()))?;
            let rows = result
                .items
                .into_iter()
                .take(IDE_INTELLIGENCE_MAX_ROWS.saturating_add(1))
                .map(navigation_row)
                .collect();
            Ok(query_result(
                title,
                rows,
                result.truncated,
                saved_version,
                dirty_buffer,
                result.workspace_revision,
                result
                    .document
                    .as_ref()
                    .is_some_and(|document| document.stale),
            ))
        }
        IdeIntelligenceTask::Diagnostics { path } => {
            let result = execute_code_intelligence_with_protocol_retry(
                "diagnostics",
                &cancellation,
                |attempt| provider.diagnostics(path.as_ref(), attempt),
            )
            .await
            .map_err(|error| sanitize_ide_intelligence_error(&error.to_string()))?;
            let rows = result
                .items
                .into_iter()
                .take(IDE_INTELLIGENCE_MAX_ROWS.saturating_add(1))
                .map(diagnostic_row)
                .collect();
            Ok(query_result(
                title,
                rows,
                result.truncated,
                saved_version,
                dirty_buffer,
                result.workspace_revision,
                result
                    .document
                    .as_ref()
                    .is_some_and(|document| document.stale),
            ))
        }
    }
}

fn query_result(
    title: String,
    rows: Vec<IdeIntelligenceRow>,
    truncated: bool,
    saved_version: bool,
    dirty_buffer: bool,
    workspace_revision: u64,
    stale: bool,
) -> IdeIntelligenceResult {
    sanitize_ide_intelligence_result(IdeIntelligenceResult {
        title,
        rows,
        truncated,
        saved_version,
        dirty_buffer,
        stale,
        workspace_revision: Some(workspace_revision),
    })
}

pub(super) fn status_result(
    title: String,
    status: CodeIntelligenceStatus,
    dirty_buffer: bool,
) -> IdeIntelligenceResult {
    let mut rows = vec![IdeIntelligenceRow {
        text: format!(
            "Workspace: {} · {}",
            intelligence_state_label(status.state),
            capability_labels(status.capabilities)
        ),
        target: None,
    }];
    if let Some(message) = status.message {
        let message =
            sanitize_ide_intelligence_field(&message, IDE_INTELLIGENCE_STATUS_MESSAGE_MAX_CHARS);
        if !message.is_empty() {
            rows.push(IdeIntelligenceRow {
                text: message,
                target: None,
            });
        }
    }
    let available_language_rows = IDE_INTELLIGENCE_MAX_ROWS.saturating_sub(rows.len());
    let languages_truncated = status.languages.len() > available_language_rows;
    for language in status.languages.into_iter().take(available_language_rows) {
        let language_name = sanitize_ide_intelligence_nonempty(
            language.language.as_str(),
            IDE_INTELLIGENCE_LANGUAGE_MAX_CHARS,
            "language",
        );
        let mut text = format!(
            "{}: {} · {}",
            language_name,
            intelligence_state_label(language.state),
            capability_labels(language.capabilities)
        );
        if let Some(message) = language.message {
            let message = sanitize_ide_intelligence_field(
                &message,
                IDE_INTELLIGENCE_STATUS_MESSAGE_MAX_CHARS,
            );
            if !message.is_empty() {
                text.push_str(" · ");
                text.push_str(&message);
            }
        }
        rows.push(IdeIntelligenceRow { text, target: None });
    }
    sanitize_ide_intelligence_result(IdeIntelligenceResult {
        title,
        rows,
        truncated: languages_truncated,
        saved_version: false,
        dirty_buffer,
        stale: false,
        workspace_revision: None,
    })
}

pub(super) fn document_symbol_rows(
    symbols: Vec<DocumentSymbol>,
    path: &str,
) -> (Vec<IdeIntelligenceRow>, bool) {
    let mut rows = Vec::new();
    let mut stack = symbols
        .into_iter()
        .rev()
        .map(|symbol| (symbol, 0_usize))
        .collect::<Vec<_>>();
    let mut depth_truncated = false;
    while let Some((mut symbol, depth)) = stack.pop() {
        if rows.len() >= IDE_INTELLIGENCE_MAX_ROWS {
            return (rows, true);
        }
        let child_depth = depth.saturating_add(1);
        stack.extend(
            std::mem::take(&mut symbol.children)
                .into_iter()
                .rev()
                .map(|child| (child, child_depth)),
        );
        let display_depth = depth.min(IDE_INTELLIGENCE_MAX_SYMBOL_DEPTH);
        depth_truncated |= depth > IDE_INTELLIGENCE_MAX_SYMBOL_DEPTH;
        let prefix = if display_depth == 0 {
            String::new()
        } else {
            let mut prefix = "│ ".repeat(display_depth.saturating_sub(1));
            prefix.push_str(if depth > IDE_INTELLIGENCE_MAX_SYMBOL_DEPTH {
                "… "
            } else {
                "└ "
            });
            prefix
        };
        let name = sanitize_ide_intelligence_nonempty(
            &symbol.name,
            IDE_INTELLIGENCE_SYMBOL_NAME_MAX_CHARS,
            "symbol",
        );
        let detail = symbol
            .detail
            .as_deref()
            .map(|detail| {
                sanitize_ide_intelligence_field(detail, IDE_INTELLIGENCE_SYMBOL_DETAIL_MAX_CHARS)
            })
            .filter(|detail| !detail.is_empty())
            .map(|detail| format!(" · {detail}"))
            .unwrap_or_default();
        rows.push(IdeIntelligenceRow {
            text: sanitize_ide_intelligence_row(&format!(
                "{prefix}{name} · {}{detail} · {}",
                symbol_kind_label(symbol.kind),
                display_position(symbol.selection_range.start)
            )),
            target: Some(IdeIntelligenceTarget {
                path: path.to_owned(),
                position: symbol.selection_range.start,
            }),
        });
    }
    (rows, depth_truncated)
}

pub(super) fn workspace_symbol_row(symbol: SymbolInformation) -> IdeIntelligenceRow {
    let name = sanitize_ide_intelligence_nonempty(
        &symbol.name,
        IDE_INTELLIGENCE_SYMBOL_NAME_MAX_CHARS,
        "symbol",
    );
    let container = symbol
        .container_name
        .as_deref()
        .map(|container| {
            sanitize_ide_intelligence_field(container, IDE_INTELLIGENCE_SYMBOL_DETAIL_MAX_CHARS)
        })
        .filter(|container| !container.is_empty())
        .map(|container| format!(" · {container}"))
        .unwrap_or_default();
    let target = target_from_location(&symbol.location);
    IdeIntelligenceRow {
        text: sanitize_ide_intelligence_row(&format!(
            "{} · {}{} · {}:{}",
            name,
            symbol_kind_label(symbol.kind),
            container,
            sanitize_ide_intelligence_path(symbol.location.path.as_str()),
            display_position(symbol.location.range.start)
        )),
        target: Some(target),
    }
}

pub(super) fn navigation_row(location: CodeLocation) -> IdeIntelligenceRow {
    let target = target_from_location(&location);
    IdeIntelligenceRow {
        text: sanitize_ide_intelligence_row(&format!(
            "{}:{}",
            sanitize_ide_intelligence_path(location.path.as_str()),
            display_position(location.range.start)
        )),
        target: Some(target),
    }
}

pub(super) fn diagnostic_row(diagnostic: CodeDiagnostic) -> IdeIntelligenceRow {
    let severity = diagnostic
        .severity
        .map(diagnostic_severity_label)
        .unwrap_or("diagnostic");
    let mut origin = diagnostic
        .source
        .as_deref()
        .map(|source| {
            sanitize_ide_intelligence_field(source, IDE_INTELLIGENCE_DIAGNOSTIC_ORIGIN_MAX_CHARS)
        })
        .unwrap_or_default();
    if let Some(code) = diagnostic.code {
        let code = sanitize_ide_intelligence_field(
            &code,
            IDE_INTELLIGENCE_DIAGNOSTIC_ORIGIN_MAX_CHARS.saturating_sub(
                origin
                    .chars()
                    .count()
                    .saturating_add(usize::from(!origin.is_empty())),
            ),
        );
        if !code.is_empty() {
            if !origin.is_empty() {
                origin.push('/');
            }
            origin.push_str(&code);
        }
    }
    let origin = if origin.is_empty() {
        String::new()
    } else {
        format!(" [{origin}]")
    };
    let message = sanitize_ide_intelligence_nonempty(
        &diagnostic.message,
        IDE_INTELLIGENCE_DIAGNOSTIC_MESSAGE_MAX_CHARS,
        "diagnostic message unavailable",
    );
    let target = target_from_location(&diagnostic.location);
    IdeIntelligenceRow {
        text: sanitize_ide_intelligence_row(&format!(
            "{} · {}:{} · {}{}",
            severity,
            sanitize_ide_intelligence_path(diagnostic.location.path.as_str()),
            display_position(diagnostic.location.range.start),
            message,
            origin
        )),
        target: Some(target),
    }
}

fn target_from_location(location: &CodeLocation) -> IdeIntelligenceTarget {
    IdeIntelligenceTarget {
        path: location.path.as_str().to_owned(),
        position: location.range.start,
    }
}

fn display_position(position: CodePosition) -> String {
    format!(
        "{}:{}",
        position.line.saturating_add(1),
        position.character.saturating_add(1)
    )
}

pub(super) fn navigation_label(kind: NavigationKind) -> &'static str {
    match kind {
        NavigationKind::Definition => "Definition",
        NavigationKind::Declaration => "Declaration",
        NavigationKind::References => "References",
        NavigationKind::Implementations => "Implementations",
    }
}

fn intelligence_state_label(state: CodeIntelligenceState) -> &'static str {
    match state {
        CodeIntelligenceState::Starting => "starting",
        CodeIntelligenceState::Ready => "ready",
        CodeIntelligenceState::Degraded => "degraded",
        CodeIntelligenceState::Unavailable => "unavailable",
    }
}

fn capability_labels(capabilities: CodeIntelligenceCapabilities) -> String {
    let mut labels = Vec::new();
    if capabilities.document_symbols {
        labels.push("document symbols");
    }
    if capabilities.workspace_symbols {
        labels.push("workspace symbols");
    }
    if capabilities.definition {
        labels.push("definition");
    }
    if capabilities.declaration {
        labels.push("declaration");
    }
    if capabilities.references {
        labels.push("references");
    }
    if capabilities.implementations {
        labels.push("implementations");
    }
    if capabilities.diagnostics {
        labels.push("diagnostics");
    }
    if labels.is_empty() {
        "no capabilities".to_owned()
    } else {
        labels.join(", ")
    }
}

fn symbol_kind_label(kind: CodeSymbolKind) -> &'static str {
    match kind {
        CodeSymbolKind::File => "file",
        CodeSymbolKind::Module => "module",
        CodeSymbolKind::Namespace => "namespace",
        CodeSymbolKind::Package => "package",
        CodeSymbolKind::Class => "class",
        CodeSymbolKind::Method => "method",
        CodeSymbolKind::Property => "property",
        CodeSymbolKind::Field => "field",
        CodeSymbolKind::Constructor => "constructor",
        CodeSymbolKind::Enum => "enum",
        CodeSymbolKind::Interface => "interface",
        CodeSymbolKind::Function => "function",
        CodeSymbolKind::Variable => "variable",
        CodeSymbolKind::Constant => "constant",
        CodeSymbolKind::String => "string",
        CodeSymbolKind::Number => "number",
        CodeSymbolKind::Boolean => "boolean",
        CodeSymbolKind::Array => "array",
        CodeSymbolKind::Object => "object",
        CodeSymbolKind::Key => "key",
        CodeSymbolKind::Null => "null",
        CodeSymbolKind::EnumMember => "enum member",
        CodeSymbolKind::Struct => "struct",
        CodeSymbolKind::Event => "event",
        CodeSymbolKind::Operator => "operator",
        CodeSymbolKind::TypeParameter => "type parameter",
        CodeSymbolKind::Unknown => "symbol",
        _ => "symbol",
    }
}

fn diagnostic_severity_label(severity: CodeDiagnosticSeverity) -> &'static str {
    match severity {
        CodeDiagnosticSeverity::Error => "error",
        CodeDiagnosticSeverity::Warning => "warning",
        CodeDiagnosticSeverity::Information => "information",
        CodeDiagnosticSeverity::Hint => "hint",
    }
}
