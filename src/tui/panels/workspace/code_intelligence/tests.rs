use super::*;

#[test]
fn parses_supported_ide_commands_without_claiming_editor_commands() {
    assert_eq!(
        parse_ide_intelligence_command("status"),
        Some(Ok(IdeIntelligenceCommand::Status))
    );
    assert_eq!(
        parse_ide_intelligence_command("symbols"),
        Some(Ok(IdeIntelligenceCommand::Symbols { query: None }))
    );
    assert_eq!(
        parse_ide_intelligence_command("symbols Runtime Registry"),
        Some(Ok(IdeIntelligenceCommand::Symbols {
            query: Some("Runtime Registry".to_owned())
        }))
    );
    assert_eq!(
        parse_ide_intelligence_command("diagnostics workspace"),
        Some(Ok(IdeIntelligenceCommand::Diagnostics { workspace: true }))
    );
    for (command, kind) in [
        ("definition", NavigationKind::Definition),
        ("declaration", NavigationKind::Declaration),
        ("references", NavigationKind::References),
        ("implementations", NavigationKind::Implementations),
    ] {
        assert_eq!(
            parse_ide_intelligence_command(command),
            Some(Ok(IdeIntelligenceCommand::Navigate(kind)))
        );
    }
    assert!(parse_ide_intelligence_command("diagnostics file")
        .expect("semantic command")
        .is_err());
    assert!(parse_ide_intelligence_command("w").is_none());
}

#[test]
fn semantic_commands_are_scoped_to_the_workspace_ide_surface() {
    let workspace = Ide::workspace(Vec::new());
    assert!(parse_ide_intelligence_command_for_ide(&workspace, "status").is_some());

    let config = Ide::browse(Vec::new(), "config");
    assert!(parse_ide_intelligence_command_for_ide(&config, "status").is_none());

    // A reused editor can have the same display title without becoming the
    // actual workspace `/ide` product surface.
    let readonly = Ide::browse(Vec::new(), "workspace");
    assert!(parse_ide_intelligence_command_for_ide(&readonly, "symbols query").is_none());

    let mut knowledge_base = Ide::browse(Vec::new(), "knowledge base");
    knowledge_base.kb_root = Some(PathBuf::from(".a3s/kb"));
    assert!(
        parse_ide_intelligence_command_for_ide(&knowledge_base, "diagnostics workspace").is_none()
    );
}

#[test]
fn maps_expanded_tabs_and_astral_characters_to_saved_utf16() {
    let text = "\t😀call();\n";
    assert_eq!(
        editor_position_to_saved_utf16(text, 0, 4).unwrap(),
        CodePosition::new(0, 1)
    );
    assert_eq!(
        editor_position_to_saved_utf16(text, 0, 5).unwrap(),
        CodePosition::new(0, 3)
    );
    assert_eq!(saved_utf16_to_editor_column("\t😀call();", 1).unwrap(), 4);
    assert_eq!(saved_utf16_to_editor_column("\t😀call();", 3).unwrap(), 5);
    assert!(saved_utf16_to_editor_column("😀", 1).is_err());
}

#[test]
fn rejects_cursor_positions_that_only_exist_in_an_unsaved_buffer() {
    let error = editor_position_to_saved_utf16("short\n", 0, 12).unwrap_err();
    assert!(error.contains("saved version"));
    assert!(editor_position_to_saved_utf16("short\n", 2, 0).is_err());
}

#[test]
fn rejects_provider_paths_that_escape_the_workspace() {
    let root = tempfile::tempdir().expect("temporary workspace");
    let services = WorkspaceServices::local(root.path());
    assert!(services.normalize_path("../outside.rs").is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn semantic_jump_uses_workspace_symlink_containment() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("temporary workspace");
    let outside = tempfile::tempdir().expect("temporary outside directory");
    let outside_file = outside.path().join("secret.rs");
    std::fs::write(&outside_file, "fn secret() {}\n").unwrap();
    symlink(&outside_file, root.path().join("escape.rs")).unwrap();
    let services = WorkspaceServices::local(root.path());
    let path = services.normalize_path("escape.rs").unwrap();
    let result = read_ide_intelligence_jump(
        services.fs(),
        path,
        root.path().join("escape.rs"),
        CodePosition::new(0, 0),
        CancellationToken::new(),
    )
    .await;
    assert!(result.is_err());
}

#[test]
fn stale_request_ids_cannot_replace_the_active_view() {
    let mut ide = Ide::workspace(Vec::new());
    ide.intelligence_request_id = 8;
    ide.intelligence = Some(IdeIntelligenceView::loading(8, "new", true, false));
    assert!(ide_intelligence_request_is_current(&ide, 8));
    assert!(!ide_intelligence_request_is_current(&ide, 7));
    ide.intelligence_request_id = 9;
    assert!(!ide_intelligence_request_is_current(&ide, 8));
}

#[test]
fn stale_completion_cannot_mutate_the_latest_result_view() {
    let mut ide = Ide::workspace(Vec::new());
    ide.intelligence_request_id = 8;
    ide.intelligence = Some(IdeIntelligenceView::loading(8, "latest", true, false));

    let stale = IdeIntelligenceResult {
        title: "stale".to_owned(),
        rows: Vec::new(),
        truncated: false,
        saved_version: true,
        dirty_buffer: false,
        stale: false,
        workspace_revision: Some(1),
    };
    assert!(!apply_ide_intelligence_result_to_ide(
        &mut ide,
        7,
        Ok(stale)
    ));
    assert_eq!(
        ide.intelligence.as_ref().map(|view| view.title.as_str()),
        Some("latest")
    );

    let latest = IdeIntelligenceResult {
        title: "complete".to_owned(),
        rows: Vec::new(),
        truncated: true,
        saved_version: true,
        dirty_buffer: false,
        stale: true,
        workspace_revision: Some(2),
    };
    assert!(apply_ide_intelligence_result_to_ide(
        &mut ide,
        8,
        Ok(latest)
    ));
    let view = ide.intelligence.as_ref().unwrap();
    assert_eq!(view.title, "complete");
    assert!(view.truncated);
    assert!(view.stale);
    assert_eq!(view.workspace_revision, Some(2));
}

#[test]
fn latest_query_cancels_and_supersedes_the_previous_query() {
    let mut ide = Ide::workspace(Vec::new());
    let (first_id, first_cancellation) = replace_ide_intelligence_request(&mut ide);
    ide.intelligence = Some(IdeIntelligenceView::loading(first_id, "first", true, false));

    let (second_id, second_cancellation) = replace_ide_intelligence_request(&mut ide);
    ide.intelligence = Some(IdeIntelligenceView::loading(
        second_id, "second", true, false,
    ));

    assert!(first_cancellation.is_cancelled());
    assert!(!second_cancellation.is_cancelled());
    assert_ne!(first_id, second_id);
    assert!(!ide_intelligence_request_is_current(&ide, first_id));
    assert!(ide_intelligence_request_is_current(&ide, second_id));
}

#[test]
fn latest_jump_cancels_and_supersedes_the_previous_jump() {
    let mut ide = Ide::workspace(Vec::new());
    ide.intelligence_request_id = 8;
    ide.intelligence = Some(IdeIntelligenceView::loading(8, "results", true, false));

    let (first_id, first_cancellation) = replace_ide_intelligence_jump_request(&mut ide);
    assert!(ide_intelligence_jump_request_is_current(&ide, 8, first_id));
    assert!(!first_cancellation.is_cancelled());

    let (second_id, second_cancellation) = replace_ide_intelligence_jump_request(&mut ide);
    assert!(first_cancellation.is_cancelled());
    assert!(!second_cancellation.is_cancelled());
    assert_ne!(first_id, second_id);
    assert!(!ide_intelligence_jump_request_is_current(&ide, 8, first_id));
    assert!(ide_intelligence_jump_request_is_current(&ide, 8, second_id));
    assert!(!ide_intelligence_jump_request_is_current(
        &ide, 7, second_id
    ));
}

#[test]
fn dirty_jump_preserves_the_same_file_and_rejects_a_different_file() {
    let mut ide = Ide::workspace(Vec::new());
    let path = PathBuf::from("src/current.rs");
    let mut file = IdeFile::new(
        path.clone(),
        vec!["unsaved first".to_owned(), "unsaved second".to_owned()],
        false,
        false,
    );
    file.dirty = true;
    ide.file = Some(file);

    assert!(validate_ide_intelligence_jump_target(&ide, &path).is_ok());
    assert_eq!(
        validate_ide_intelligence_jump_target(&ide, Path::new("src/other.rs")),
        Err(DIRTY_JUMP_MESSAGE)
    );

    let preserved = install_ide_intelligence_jump(
        &mut ide,
        IdeIntelligenceJump {
            path,
            lines: vec!["saved first".to_owned(), "saved second".to_owned()],
            row: 1,
            col: 99,
        },
        10,
    );
    assert!(preserved);
    let file = ide.file.as_ref().unwrap();
    assert!(file.dirty);
    assert_eq!(file.lines[0], "unsaved first");
    assert_eq!(file.lines[1], "unsaved second");
    assert_eq!(file.row, 1);
    assert_eq!(file.col, "unsaved second".chars().count());
}

#[test]
fn dropping_an_ide_cancels_active_semantic_work() {
    let (query, jump) = {
        let ide = Ide::workspace(Vec::new());
        (
            ide.intelligence_cancellation.clone(),
            ide.intelligence_jump_cancellation.clone(),
        )
    };
    assert!(query.is_cancelled());
    assert!(jump.is_cancelled());
}

#[test]
fn dirty_result_notice_explicitly_ignores_unsaved_edits() {
    let view = IdeIntelligenceView::loading(1, "symbols", true, true);
    let notice = ide_intelligence_notice(&view);
    assert!(notice.contains("UNSAVED EDITS IGNORED"));
    assert!(notice.contains("saved version"));
}

#[test]
fn workspace_footer_discovers_semantic_commands() {
    let hint = ide_intelligence_command_hint();
    assert!(hint.contains(":status"));
    assert!(hint.contains(":symbols"));
    assert!(hint.contains(":definition"));
    assert!(hint.contains(":diagnostics"));
}

fn hostile_terminal_text(visible: &str, repeated: usize) -> String {
    format!(
        "\u{1b}]0;owned title\u{7}{visible}\u{1b}[31m\n\u{9b}32m\u{202e}{}",
        "界".repeat(repeated)
    )
}

fn assert_terminal_safe_and_bounded(value: &str, max_chars: usize) {
    assert_eq!(
        value,
        crate::system_agents::sanitize_display_text(value, max_chars)
    );
    assert!(
        value.chars().count() <= max_chars,
        "{}",
        value.chars().count()
    );
    assert!(!value.contains("owned title"));
    assert!(!value.contains(['\r', '\n', '\u{202e}']));
}

#[test]
fn symbol_queries_reject_oversized_or_invisible_terminal_input() {
    let oversized = format!(
        "symbols {}",
        "x".repeat(IDE_INTELLIGENCE_QUERY_MAX_CHARS + 1)
    );
    assert!(parse_ide_intelligence_command(&oversized)
        .expect("semantic command")
        .is_err());
    assert!(
        parse_ide_intelligence_command("symbols \u{1b}]0;hidden\u{7}\u{202e}")
            .expect("semantic command")
            .is_err()
    );

    let parsed = parse_ide_intelligence_command(
        "symbols \u{1b}]0;hidden\u{7}Runtime\u{1b}[31m\nRegistry\u{202e}",
    )
    .expect("semantic command")
    .expect("safe query");
    assert_eq!(
        parsed,
        IdeIntelligenceCommand::Symbols {
            query: Some("Runtime Registry".to_owned())
        }
    );
}

#[test]
fn lsp_status_text_is_terminal_safe_and_bounded() {
    let status = a3s_code_core::CodeIntelligenceStatus {
        state: CodeIntelligenceState::Degraded,
        capabilities: CodeIntelligenceCapabilities::default(),
        languages: vec![a3s_code_core::CodeIntelligenceLanguageStatus {
            language: a3s_code_core::LanguageId::new(hostile_terminal_text("rust", 200)),
            state: CodeIntelligenceState::Degraded,
            capabilities: CodeIntelligenceCapabilities::default(),
            message: Some(hostile_terminal_text("language message", 2_000)),
        }],
        message: Some(hostile_terminal_text("workspace message", 2_000)),
    };
    let result = status_result(hostile_terminal_text("status", 2_000), status, false);

    assert_terminal_safe_and_bounded(&result.title, IDE_INTELLIGENCE_TITLE_MAX_CHARS);
    assert_eq!(result.rows.len(), 3);
    for row in result.rows {
        assert_terminal_safe_and_bounded(&row.text, IDE_INTELLIGENCE_ROW_MAX_CHARS);
    }
}

#[test]
fn lsp_rows_sanitize_labels_without_mutating_navigation_targets() {
    let raw_path = "src/\u{1b}]0;owned title\u{7}visible\u{202e}.rs";
    let location = CodeLocation {
        path: WorkspacePath::from_normalized(raw_path),
        range: a3s_code_core::CodeRange::new(CodePosition::new(4, 6), CodePosition::new(4, 10)),
    };
    let symbol_row = workspace_symbol_row(SymbolInformation {
        name: hostile_terminal_text("VisibleSymbol", 2_000),
        kind: CodeSymbolKind::Function,
        location: location.clone(),
        container_name: Some(hostile_terminal_text("VisibleContainer", 2_000)),
    });
    assert_terminal_safe_and_bounded(&symbol_row.text, IDE_INTELLIGENCE_ROW_MAX_CHARS);
    assert!(symbol_row.text.contains("VisibleSymbol"));
    assert!(symbol_row.text.contains("VisibleContainer"));
    assert_eq!(symbol_row.target.as_ref().unwrap().path, raw_path);

    let diagnostic_row = diagnostic_row(a3s_code_core::CodeDiagnostic {
        location,
        severity: Some(CodeDiagnosticSeverity::Error),
        code: Some(hostile_terminal_text("E0308", 500)),
        source: Some(hostile_terminal_text("rust-analyzer", 500)),
        message: hostile_terminal_text("mismatched types", 2_000),
    });
    assert_terminal_safe_and_bounded(&diagnostic_row.text, IDE_INTELLIGENCE_ROW_MAX_CHARS);
    assert!(diagnostic_row.text.contains("mismatched types"));
    assert_eq!(diagnostic_row.target.as_ref().unwrap().path, raw_path);
}

#[test]
fn document_symbol_projection_is_iterative_depth_bounded_and_preorder() {
    fn symbol(name: String, children: Vec<DocumentSymbol>) -> DocumentSymbol {
        let range = a3s_code_core::CodeRange::new(CodePosition::new(0, 0), CodePosition::new(0, 1));
        DocumentSymbol {
            name,
            detail: None,
            kind: CodeSymbolKind::Function,
            range,
            selection_range: range,
            children,
        }
    }

    let mut nested = symbol("leaf".to_owned(), Vec::new());
    for depth in 0..IDE_INTELLIGENCE_MAX_SYMBOL_DEPTH + 4 {
        nested = symbol(format!("parent-{depth}"), vec![nested]);
    }
    let (rows, truncated) = document_symbol_rows(vec![nested], "src/lib.rs");

    assert!(truncated, "display-depth truncation must be explicit");
    assert_eq!(rows.len(), IDE_INTELLIGENCE_MAX_SYMBOL_DEPTH + 5);
    assert!(rows.iter().any(|row| row.text.contains('…')));
    assert!(rows.iter().all(|row| {
        row.text.chars().count() <= IDE_INTELLIGENCE_ROW_MAX_CHARS
            && row.target.as_ref().map(|target| target.path.as_str()) == Some("src/lib.rs")
    }));
}

#[test]
fn result_application_and_panel_defensively_sanitize_future_producers() {
    let mut ide = Ide::workspace(Vec::new());
    ide.intelligence_request_id = 4;
    ide.intelligence = Some(IdeIntelligenceView::loading(4, "loading", false, false));
    let target = IdeIntelligenceTarget {
        path: "src/raw-target.rs".to_owned(),
        position: CodePosition::new(1, 2),
    };
    let rows = (0..=IDE_INTELLIGENCE_MAX_ROWS)
        .map(|_| IdeIntelligenceRow {
            text: hostile_terminal_text("visible result", 2_000),
            target: Some(target.clone()),
        })
        .collect();
    let result = IdeIntelligenceResult {
        title: hostile_terminal_text("visible title", 2_000),
        rows,
        truncated: false,
        saved_version: false,
        dirty_buffer: false,
        stale: false,
        workspace_revision: Some(9),
    };
    assert!(apply_ide_intelligence_result_to_ide(
        &mut ide,
        4,
        Ok(result)
    ));

    let view = ide.intelligence.as_ref().unwrap();
    assert!(view.truncated);
    assert_eq!(view.rows.len(), IDE_INTELLIGENCE_MAX_ROWS);
    assert_terminal_safe_and_bounded(&view.title, IDE_INTELLIGENCE_TITLE_MAX_CHARS);
    assert_terminal_safe_and_bounded(&view.rows[0].text, IDE_INTELLIGENCE_ROW_MAX_CHARS);
    assert_eq!(view.rows[0].target.as_ref(), Some(&target));

    let (title, rendered) = ide_intelligence_panel(&ide, 1, 40).expect("result panel");
    assert!(!title.contains("owned title"));
    let plain = a3s_tui::style::strip_ansi(&rendered[0]);
    assert!(!plain.contains("owned title"));
    assert!(!plain.contains(['\r', '\n', '\u{202e}']));
    assert!(a3s_tui::style::visible_len(&rendered[0]) <= 40);
}

#[tokio::test]
async fn typed_protocol_failures_receive_exactly_one_cancellable_retry() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let attempts = Arc::new(AtomicUsize::new(0));
    let result = execute_code_intelligence_with_protocol_retry(
        "test_query",
        &CancellationToken::new(),
        |_| {
            let attempts = Arc::clone(&attempts);
            async move {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(a3s_code_core::CodeIntelligenceError::Protocol {
                        message: "transient protocol state".to_owned(),
                    })
                } else {
                    Ok("ready")
                }
            }
        },
    )
    .await;

    assert_eq!(result, Ok("ready"));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);

    let cancellation = CancellationToken::new();
    let cancelled_attempts = Arc::new(AtomicUsize::new(0));
    let cancelled =
        execute_code_intelligence_with_protocol_retry("test_query", &cancellation, |_| {
            let cancellation = cancellation.clone();
            let attempts = Arc::clone(&cancelled_attempts);
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                cancellation.cancel();
                Err::<(), _>(a3s_code_core::CodeIntelligenceError::Protocol {
                    message: "protocol state before cancellation".to_owned(),
                })
            }
        })
        .await;
    assert_eq!(
        cancelled,
        Err(a3s_code_core::CodeIntelligenceError::Cancelled)
    );
    assert_eq!(cancelled_attempts.load(Ordering::SeqCst), 1);
}

/// End-to-end through the real TUI adapter and an installed language server.
/// Run with:
/// `cargo test real_rust_analyzer_saved_workspace_roundtrip_when_available --bin a3s -- --ignored --nocapture`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "external rust-analyzer process; run explicitly outside the parallel unit suite"]
async fn real_rust_analyzer_saved_workspace_roundtrip_when_available() {
    let available = std::process::Command::new("rust-analyzer")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !available {
        eprintln!("rust-analyzer is unavailable; real Code Intelligence smoke was skipped");
        return;
    }

    let workspace = tempfile::tempdir().expect("temporary Rust workspace");
    std::fs::create_dir(workspace.path().join("src")).expect("source directory");
    std::fs::write(
        workspace.path().join("Cargo.toml"),
        "[package]\nname = \"a3s-code-intelligence-smoke\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("Cargo manifest");
    std::fs::write(
        workspace.path().join("src/lib.rs"),
        "pub struct Greeter;\n\nimpl Greeter {\n    pub fn greet(name: &str) -> String {\n        format!(\"Hello, {name}\")\n    }\n}\n\npub fn call_greeter() -> String {\n    Greeter::greet(\"A3S\")\n}\n\npub fn broken_value() -> u32 {\n    \"not a number\"\n}\n",
    )
    .expect("Rust source");

    let backend = ManifestWorkspaceBackend::new(workspace.path());
    let manifest = backend.manifest();
    if manifest.snapshot().version == 0 {
        let mut snapshots = manifest.subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let snapshot = snapshots.recv().await.expect("manifest snapshot");
                if snapshot.version > 0
                    && snapshot.files.iter().any(|file| file.path == "src/lib.rs")
                {
                    break;
                }
            }
        })
        .await
        .expect("initial manifest scan");
    }

    let file_system: Arc<dyn a3s_code_core::workspace::WorkspaceFileSystem> = backend;
    let provider = LocalCodeIntelligence::start(
        format!("a3s-tui-real-smoke-{}", std::process::id()),
        manifest,
        Arc::clone(&file_system),
    )
    .await
    .expect("start real Code Intelligence provider");
    let query_provider: Arc<dyn WorkspaceCodeIntelligence> = provider.clone();
    let path = WorkspacePath::from_normalized("src/lib.rs");

    let symbols = execute_ide_intelligence_query(
        Arc::clone(&query_provider),
        Arc::clone(&file_system),
        PreparedIdeIntelligenceQuery {
            title: "Document symbols · src/lib.rs".to_owned(),
            task: IdeIntelligenceTask::DocumentSymbols { path: path.clone() },
            saved_version: true,
            dirty_buffer: false,
        },
        CancellationToken::new(),
    )
    .await;
    let mut workspace_symbols = execute_ide_intelligence_query(
        Arc::clone(&query_provider),
        Arc::clone(&file_system),
        PreparedIdeIntelligenceQuery {
            title: "Workspace symbols · Greeter".to_owned(),
            task: IdeIntelligenceTask::WorkspaceSymbols {
                query: "Greeter".to_owned(),
            },
            saved_version: false,
            dirty_buffer: false,
        },
        CancellationToken::new(),
    )
    .await;
    for _ in 0..20 {
        if workspace_symbols
            .as_ref()
            .is_ok_and(|result| !result.rows.is_empty())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        workspace_symbols = execute_ide_intelligence_query(
            Arc::clone(&query_provider),
            Arc::clone(&file_system),
            PreparedIdeIntelligenceQuery {
                title: "Workspace symbols · Greeter".to_owned(),
                task: IdeIntelligenceTask::WorkspaceSymbols {
                    query: "Greeter".to_owned(),
                },
                saved_version: false,
                dirty_buffer: false,
            },
            CancellationToken::new(),
        )
        .await;
    }
    let definition = execute_ide_intelligence_query(
        Arc::clone(&query_provider),
        Arc::clone(&file_system),
        PreparedIdeIntelligenceQuery {
            title: "Definition · src/lib.rs".to_owned(),
            task: IdeIntelligenceTask::Navigate {
                kind: NavigationKind::Definition,
                path: path.clone(),
                row: 9,
                expanded_col: 15,
            },
            saved_version: true,
            dirty_buffer: false,
        },
        CancellationToken::new(),
    )
    .await;
    let mut diagnostics = execute_ide_intelligence_query(
        Arc::clone(&query_provider),
        Arc::clone(&file_system),
        PreparedIdeIntelligenceQuery {
            title: "Document diagnostics · src/lib.rs".to_owned(),
            task: IdeIntelligenceTask::Diagnostics {
                path: Some(path.clone()),
            },
            saved_version: true,
            dirty_buffer: false,
        },
        CancellationToken::new(),
    )
    .await;
    for _ in 0..20 {
        if diagnostics
            .as_ref()
            .is_ok_and(|result| !result.rows.is_empty())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        diagnostics = execute_ide_intelligence_query(
            Arc::clone(&query_provider),
            Arc::clone(&file_system),
            PreparedIdeIntelligenceQuery {
                title: "Document diagnostics · src/lib.rs".to_owned(),
                task: IdeIntelligenceTask::Diagnostics {
                    path: Some(path.clone()),
                },
                saved_version: true,
                dirty_buffer: false,
            },
            CancellationToken::new(),
        )
        .await;
    }
    let status = execute_ide_intelligence_query(
        query_provider,
        file_system,
        PreparedIdeIntelligenceQuery {
            title: "Code Intelligence status".to_owned(),
            task: IdeIntelligenceTask::Status,
            saved_version: false,
            dirty_buffer: false,
        },
        CancellationToken::new(),
    )
    .await;
    provider.shutdown().await;

    let symbols = symbols.expect("real document symbols");
    assert!(symbols.rows.iter().any(|row| row.text.contains("Greeter")));
    assert!(symbols.rows.iter().any(|row| row.text.contains("greet")));

    let workspace_symbols = workspace_symbols.expect("real workspace symbols");
    assert!(
        workspace_symbols
            .rows
            .iter()
            .any(|row| row.text.contains("Greeter")),
        "{:?}",
        workspace_symbols.rows
    );

    let definition = definition.expect("real definition navigation");
    assert!(
        definition.rows.iter().any(|row| {
            row.target
                .as_ref()
                .is_some_and(|target| target.path == "src/lib.rs" && target.position.line == 3)
        }),
        "{:?}",
        definition.rows
    );

    let diagnostics = diagnostics.expect("real diagnostics");
    assert!(
        diagnostics.rows.iter().any(|row| {
            row.text.starts_with("error")
                && row
                    .target
                    .as_ref()
                    .is_some_and(|target| target.path == "src/lib.rs" && target.position.line == 13)
        }),
        "{:?}",
        diagnostics.rows
    );
    let status = status.expect("real Code Intelligence status");
    assert!(status.rows.iter().any(|row| {
        row.text.starts_with("Workspace: ready") || row.text.starts_with("Workspace: degraded")
    }));
    assert!(status.rows.iter().any(|row| {
        row.text.starts_with("rust: ready") || row.text.starts_with("rust: degraded")
    }));
}
