#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResearchReportArtifacts {
    pub(crate) markdown: PathBuf,
    pub(crate) html: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DeepResearchReportArtifactBaseline {
    markdown: Option<ResearchReportFileFingerprint>,
    html: Option<ResearchReportFileFingerprint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResearchReportFileFingerprint {
    len: u64,
    modified: Option<SystemTime>,
    content_hash: u64,
}

impl DeepResearchReportArtifactBaseline {
    fn accepts_current_run_artifacts(&self, artifacts: &ResearchReportArtifacts) -> bool {
        let markdown = research_report_file_fingerprint(&artifacts.markdown);
        let html = research_report_file_fingerprint(&artifacts.html);
        markdown.is_some() && html.is_some() && markdown != self.markdown && html != self.html
    }
}

pub(crate) fn snapshot_deep_research_report_artifacts(
    workspace: &Path,
    query: &str,
) -> DeepResearchReportArtifactBaseline {
    let report_dir = workspace
        .join(".a3s")
        .join("research")
        .join(deep_research_report_slug(query));
    DeepResearchReportArtifactBaseline {
        markdown: research_report_file_fingerprint(&report_dir.join("report.md")),
        html: research_report_file_fingerprint(&report_dir.join("index.html")),
    }
}

fn research_report_file_fingerprint(path: &Path) -> Option<ResearchReportFileFingerprint> {
    const MAX_REPORT_FINGERPRINT_BYTES: u64 = 2 * 1024 * 1024;

    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_REPORT_FINGERPRINT_BYTES
    {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            return None;
        }
    }
    let bytes = std::fs::read(path).ok()?;
    let mut content_hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        content_hash ^= byte as u64;
        content_hash = content_hash.wrapping_mul(0x100000001b3);
    }
    Some(ResearchReportFileFingerprint {
        len: metadata.len(),
        modified: metadata.modified().ok(),
        content_hash,
    })
}

fn ensure_plain_directory(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing symlinked DeepResearch directory {}",
            path.display()
        )),
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(format!(
            "DeepResearch artifact path is not a directory: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::create_dir(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    ensure_plain_directory(path)
                }
                Err(error) => Err(format!("could not create {}: {error}", path.display())),
            }
        }
        Err(error) => Err(format!("could not inspect {}: {error}", path.display())),
    }
}

fn prepare_research_report_directory(
    workspace: &Path,
    slug: &str,
) -> Result<(PathBuf, PathBuf), String> {
    if slug.is_empty() || slug == "." || slug == ".." || slug.contains('/') || slug.contains('\\') {
        return Err("invalid DeepResearch report slug".to_string());
    }

    let root = workspace.canonicalize().map_err(|error| {
        format!(
            "could not resolve workspace {}: {error}",
            workspace.display()
        )
    })?;
    let a3s_dir = root.join(".a3s");
    ensure_plain_directory(&a3s_dir)?;
    let research_dir = a3s_dir.join("research");
    ensure_plain_directory(&research_dir)?;
    let report_dir = research_dir.join(slug);
    ensure_plain_directory(&report_dir)?;

    let canonical_research = research_dir
        .canonicalize()
        .map_err(|error| format!("could not resolve {}: {error}", research_dir.display()))?;
    let canonical_report = report_dir
        .canonicalize()
        .map_err(|error| format!("could not resolve {}: {error}", report_dir.display()))?;
    if canonical_research.parent() != Some(a3s_dir.as_path())
        || canonical_report.parent() != Some(canonical_research.as_path())
        || !canonical_report.starts_with(&root)
    {
        return Err("DeepResearch report directory escaped the workspace".to_string());
    }
    Ok((root, canonical_report))
}

fn validate_research_report_file_target(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!(
                "refusing symlinked DeepResearch artifact {}",
                path.display()
            ));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!(
                "DeepResearch artifact target is not a file: {}",
                path.display()
            ));
        }
        Ok(metadata) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.nlink() > 1 {
                    return Err(format!(
                        "refusing hard-linked DeepResearch artifact {}",
                        path.display()
                    ));
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("could not inspect {}: {error}", path.display())),
    }
    Ok(())
}

fn stage_research_report_file(path: &Path, contents: &[u8]) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("DeepResearch artifact has no parent: {}", path.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid DeepResearch artifact name: {}", path.display()))?;
    for _ in 0..8 {
        let staged = parent.join(format!(
            ".{name}.{}.{:016x}.tmp",
            std::process::id(),
            rand::random::<u64>()
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(contents).and_then(|_| file.sync_all()) {
                    let _ = std::fs::remove_file(&staged);
                    return Err(format!("could not stage {}: {error}", path.display()));
                }
                return Ok(staged);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("could not stage {}: {error}", path.display()));
            }
        }
    }
    Err(format!(
        "could not allocate a staging file for {}",
        path.display()
    ))
}

fn replace_staged_research_report_file(staged: &Path, path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|error| format!("could not replace {}: {error}", path.display()))?;
    }
    std::fs::rename(staged, path)
        .map_err(|error| format!("could not publish {}: {error}", path.display()))
}

fn write_research_report_file(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), String> {
    validate_research_report_file_target(path)?;
    let staged = stage_research_report_file(path, contents.as_ref())?;
    let result = replace_staged_research_report_file(&staged, path);
    if result.is_err() {
        let _ = std::fs::remove_file(staged);
    }
    result
}

fn existing_research_report_file(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::read(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("could not preserve {}: {error}", path.display())),
    }
}

fn restore_research_report_file(path: &Path, contents: Option<&[u8]>) -> Result<(), String> {
    match contents {
        Some(contents) => write_research_report_file(path, contents),
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("could not roll back {}: {error}", path.display())),
        },
    }
}

fn write_research_report_pair(
    markdown_path: &Path,
    markdown: impl AsRef<[u8]>,
    html_path: &Path,
    html: impl AsRef<[u8]>,
) -> Result<(), String> {
    // Validate both destinations before touching either current generation.
    validate_research_report_file_target(markdown_path)?;
    validate_research_report_file_target(html_path)?;
    let previous_markdown = existing_research_report_file(markdown_path)?;
    let previous_html = existing_research_report_file(html_path)?;
    let staged_markdown = stage_research_report_file(markdown_path, markdown.as_ref())?;
    let staged_html = match stage_research_report_file(html_path, html.as_ref()) {
        Ok(staged) => staged,
        Err(error) => {
            let _ = std::fs::remove_file(staged_markdown);
            return Err(error);
        }
    };

    if let Err(error) = replace_staged_research_report_file(&staged_markdown, markdown_path) {
        let _ = std::fs::remove_file(staged_markdown);
        let _ = std::fs::remove_file(staged_html);
        let _ = restore_research_report_file(markdown_path, previous_markdown.as_deref());
        return Err(error);
    }
    if let Err(error) = replace_staged_research_report_file(&staged_html, html_path) {
        let _ = std::fs::remove_file(staged_html);
        let markdown_rollback =
            restore_research_report_file(markdown_path, previous_markdown.as_deref());
        let html_rollback = restore_research_report_file(html_path, previous_html.as_deref());
        return match (markdown_rollback, html_rollback) {
            (Ok(()), Ok(())) => Err(error),
            (markdown, html) => Err(format!(
                "{error}; report pair rollback failed: markdown={markdown:?}, html={html:?}"
            )),
        };
    }
    Ok(())
}

pub(crate) fn research_report_artifacts_from_output(
    output: &str,
    workspace: &Path,
) -> Option<ResearchReportArtifacts> {
    research_report_artifacts_from_output_with_slug(output, workspace, None)
}

pub(crate) fn research_report_artifacts_from_output_for_query(
    output: &str,
    workspace: &Path,
    query: &str,
) -> Option<ResearchReportArtifacts> {
    let expected_slug = deep_research_report_slug(query);
    research_report_artifacts_from_output_with_slug(output, workspace, Some(&expected_slug))
}

pub(crate) fn research_report_artifacts_from_output_for_current_run(
    output: &str,
    workspace: &Path,
    query: &str,
    baseline: &DeepResearchReportArtifactBaseline,
) -> Option<ResearchReportArtifacts> {
    let artifacts = research_report_artifacts_from_output_for_query(output, workspace, query)?;
    baseline
        .accepts_current_run_artifacts(&artifacts)
        .then_some(artifacts)
}

pub(crate) fn deep_research_report_artifacts_from_output_for_query(
    output: &str,
    workspace: &Path,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Option<ResearchReportArtifacts> {
    let artifacts = research_report_artifacts_from_output_for_query(output, workspace, query)?;
    deep_research_report_sources_trace_workflow(
        &artifacts,
        query,
        workflow_output,
        workflow_metadata,
    )
    .then_some(artifacts)
}

pub(crate) fn deep_research_report_artifacts_from_output_for_current_run(
    output: &str,
    workspace: &Path,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
    baseline: &DeepResearchReportArtifactBaseline,
) -> Option<ResearchReportArtifacts> {
    let artifacts =
        research_report_artifacts_from_output_for_current_run(output, workspace, query, baseline)?;
    deep_research_report_sources_trace_workflow(
        &artifacts,
        query,
        workflow_output,
        workflow_metadata,
    )
    .then_some(artifacts)
}

pub(crate) fn clean_deep_research_final_text_from_artifacts(
    artifacts: &ResearchReportArtifacts,
    workspace: &Path,
) -> Option<String> {
    let markdown = read_small_utf8_file(&artifacts.markdown)?;
    if deep_research_output_has_internal_leak(&markdown) {
        return None;
    }
    let root = workspace.canonicalize().ok()?;
    let rel_html = artifacts.html.strip_prefix(&root).ok()?.to_string_lossy();
    let rel_html = rel_html.replace('\\', "/");
    let body = markdown.trim();
    if body.is_empty() {
        return None;
    }
    Some(format!("{body}\n\n{RESEARCH_VIEW_MARKER} {rel_html}"))
}

pub(crate) fn materialize_deep_research_completed_report_from_markdown(
    workspace: &Path,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Option<ResearchReportArtifacts> {
    let slug = deep_research_report_slug(query);
    let (root, report_dir) = prepare_research_report_directory(workspace, &slug).ok()?;
    let markdown_path = report_dir.join("report.md");
    let markdown = read_small_utf8_file(&markdown_path)?;
    let html = deep_research_completed_report_html(query, &markdown);
    if validate_deep_research_completed_report_content(
        &markdown,
        &html,
        query,
        workflow_output,
        workflow_metadata,
    )
    .is_err()
    {
        return None;
    }
    write_research_report_file(&report_dir.join("index.html"), html).ok()?;

    let rel_html = format!(".a3s/research/{slug}/index.html");
    let artifacts = trusted_research_report_artifact_paths(&rel_html, &root)?;
    completed_research_report_artifacts(&artifacts).then_some(artifacts)
}

pub(crate) fn materialize_deep_research_completed_report_from_answer_text(
    workspace: &Path,
    query: &str,
    answer_text: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Option<ResearchReportArtifacts> {
    let markdown = completed_report_markdown_from_answer_text(query, answer_text)?;
    let html = deep_research_completed_report_html(query, &markdown);
    if validate_deep_research_completed_report_content(
        &markdown,
        &html,
        query,
        workflow_output,
        workflow_metadata,
    )
    .is_err()
    {
        return None;
    }

    let slug = deep_research_report_slug(query);
    let (root, report_dir) = prepare_research_report_directory(workspace, &slug).ok()?;
    write_research_report_pair(
        &report_dir.join("report.md"),
        markdown,
        &report_dir.join("index.html"),
        html,
    )
    .ok()?;

    let rel_html = format!(".a3s/research/{slug}/index.html");
    let artifacts = trusted_research_report_artifact_paths(&rel_html, &root)?;
    completed_research_report_artifacts(&artifacts).then_some(artifacts)
}

#[cfg(test)]
pub(crate) fn materialize_deep_research_completed_report_from_workflow_evidence(
    workspace: &Path,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Option<ResearchReportArtifacts> {
    let workflow = serde_json::from_str::<serde_json::Value>(workflow_output.trim()).ok()?;
    if deep_research_collection_status(&workflow) != "completed" {
        return None;
    }
    let evidence =
        deep_research_structured_evidence_from_workflow(workflow_output, workflow_metadata);
    if evidence.is_empty()
        || !evidence.iter().any(|item| {
            item.sources
                .iter()
                .any(|source| normalize_research_source_anchor(&source.url_or_path).is_some())
        })
    {
        return None;
    }

    let finalized_checker = workflow.get("checker").filter(|checker| {
        checker.get("decision").and_then(serde_json::Value::as_str) == Some("finalize")
    });
    let report_title = workflow
        .pointer("/plan/report_title")
        .and_then(serde_json::Value::as_str);
    let verified_summary = finalized_checker
        .and_then(|checker| {
            checker
                .get("report_summary")
                .or_else(|| checker.get("coverage_summary"))
        })
        .and_then(serde_json::Value::as_str);
    let verified_findings = finalized_checker
        .and_then(|checker| checker.get("verified_findings"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let mut verified_caveats = ["unresolved_gaps", "contradictions"]
        .into_iter()
        .flat_map(|field| {
            finalized_checker
                .and_then(|checker| checker.get(field))
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if workflow
        .pointer("/verification/status")
        .and_then(serde_json::Value::as_str)
        == Some("degraded")
    {
        verified_caveats.insert(
            0,
            if query
                .chars()
                .any(|ch| ('\u{3400}'..='\u{9fff}').contains(&ch))
            {
                "本次运行未完成独立核验；结论直接来自所列可追溯证据，应视为有待复核的阶段性判断。"
                    .to_string()
            } else {
                "Independent verification did not complete in this run; conclusions are derived directly from the cited traceable evidence and remain provisional."
                    .to_string()
            },
        );
    }
    let markdown = completed_report_markdown_with_verified_context(
        query,
        &evidence,
        report_title,
        verified_summary,
        &verified_findings,
        &verified_caveats,
    )?;
    if deep_research_output_has_internal_leak(&markdown) {
        return None;
    }
    let html = deep_research_completed_report_html(query, &markdown);
    if !has_research_report_substance(&markdown, &html) {
        return None;
    }

    let slug = deep_research_report_slug(query);
    let (root, report_dir) = prepare_research_report_directory(workspace, &slug).ok()?;
    write_research_report_pair(
        &report_dir.join("report.md"),
        markdown,
        &report_dir.join("index.html"),
        html,
    )
    .ok()?;

    let rel_html = format!(".a3s/research/{slug}/index.html");
    let artifacts = trusted_research_report_artifact_paths(&rel_html, &root)?;
    // Sources are emitted exclusively from verified structured evidence.
    Some(artifacts)
}

pub(crate) fn materialize_deep_research_recovery_report(
    workspace: &Path,
    query: &str,
    answer_text: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Result<ResearchReportArtifacts, String> {
    let completed_slug = deep_research_report_slug(query);
    let completed_dir = workspace
        .join(".a3s")
        .join("research")
        .join(&completed_slug);
    let completed_artifacts = ResearchReportArtifacts {
        markdown: completed_dir.join("report.md"),
        html: completed_dir.join("index.html"),
    };
    let slug = if completed_research_report_artifacts(&completed_artifacts) {
        format!("{completed_slug}-recovery")
    } else {
        completed_slug
    };
    let rel_html = format!(".a3s/research/{slug}/index.html");
    let (root, report_dir) = prepare_research_report_directory(workspace, &slug)?;

    let result = demote_recovery_result_title(&deep_research_recovery_result_text(
        answer_text,
        workflow_output,
    ));
    let evidence_status =
        deep_research_recovery_evidence_status(workflow_output, workflow_metadata);
    let sources = deep_research_recovery_sources(workflow_output, workflow_metadata, &slug);
    let query_markdown = markdown_plain_text(query);
    let markdown = format!(
        "# DeepResearch Recovery Report\n\n\
         ## Query\n\n{query_markdown}\n\n\
         ## Findings\n\n{result}\n\n\
         ## Sources And Evidence\n\n{sources}\n\n\
         ## Evidence Status\n\n{evidence_status}\n\n\
         ## Confidence And Limits\n\n\
         Confidence is low when the evidence workflow stops before returning enough \
         traceable sources. This report is the final run artifact for the user request, \
         but it avoids presenting unsupported claims as established facts. Treat any \
         domain conclusion as provisional until the cited sources above are available \
         and independently checked.\n\n\
         ## Next Actions\n\n\
         - Retry the DeepResearch query after reducing scope or increasing the evidence budget.\n\
         - Prefer official or primary sources first, then add independent analysis.\n\
         - Use this run artifact at `.a3s/research/{slug}/report.md` as the local record of \
         why the original run could not produce a fully source-backed answer.\n"
    );
    let html = deep_research_completed_report_html(query, &markdown);
    write_research_report_pair(
        &report_dir.join("report.md"),
        markdown,
        &report_dir.join("index.html"),
        html,
    )?;

    trusted_research_report_artifact_paths(&rel_html, &root)
        .ok_or_else(|| "recovery report artifacts failed validation".to_string())
        .and_then(|artifacts| {
            recovery_research_report_artifacts(&artifacts)
                .then_some(artifacts)
                .ok_or_else(|| "recovery report artifacts failed validation".to_string())
        })
}

#[cfg(test)]
pub(crate) fn deep_research_workflow_needs_recovery_report(workflow_output: &str) -> bool {
    deep_research_workflow_needs_recovery_report_with_metadata(workflow_output, None)
}

pub(crate) fn deep_research_workflow_needs_recovery_report_with_metadata(
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> bool {
    let canonical_output =
        deep_research_canonical_workflow_output(workflow_output, workflow_metadata);
    let trimmed = canonical_output.trim();
    if trimmed.is_empty() {
        return true;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return true;
    };
    if deep_research_collection_status(&value) == "completed" {
        return false;
    }

    // An independent checker may deliberately close a bounded run as
    // `degrade`: the evidence is useful and traceable, but one or more planned
    // gaps remain. That package should produce a qualified reader-facing
    // report, not the generic Recovery artifact. Unreviewed partial evidence
    // still takes the recovery path.
    let checker_degraded = value
        .pointer("/checker/decision")
        .and_then(serde_json::Value::as_str)
        == Some("degrade");
    let has_traceable_evidence =
        !deep_research_workflow_source_anchors(&canonical_output, workflow_metadata).is_empty();
    !(checker_degraded && has_traceable_evidence)
}

fn completed_report_markdown_from_answer_text(query: &str, answer_text: &str) -> Option<String> {
    let mut body = answer_text.trim().to_string();
    if body.is_empty()
        || looks_like_deep_research_fallback_draft(&body)
        || looks_like_deep_research_recovery_report(&body)
        || is_deep_research_model_failure_text(&body)
        || deep_research_output_has_internal_leak(&body)
        || visible_char_count(&body) < 120
    {
        return None;
    }

    if let Some(marker_at) = body.find(RESEARCH_VIEW_MARKER) {
        body.truncate(marker_at);
        body = body.trim().to_string();
    }
    if body.is_empty() || visible_char_count(&body) < 120 {
        return None;
    }
    if body
        .lines()
        .next()
        .is_some_and(|line| line.trim().starts_with("# "))
    {
        Some(body)
    } else {
        Some(format!("# {}\n\n{body}", markdown_plain_text(query)))
    }
}

fn validate_deep_research_completed_report_content(
    markdown: &str,
    html: &str,
    query: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Result<(), String> {
    // Evidence readiness (Outlining) is not report completion. Inquiry-backed
    // content is publishable only after the section drafts and citation audit
    // have been committed to the replayable event log. Legacy checked-loop
    // outputs return `Ok(None)` and retain their existing validation path.
    deep_research_inquiry_publication_outcome(workflow_output, workflow_metadata)?;
    if looks_like_deep_research_fallback_draft(markdown) {
        return Err("content rejected: the response is an incomplete fallback draft".to_string());
    }
    if looks_like_deep_research_recovery_report(markdown) {
        return Err(
            "content rejected: a recovery report cannot be published as a completed report"
                .to_string(),
        );
    }
    if is_deep_research_model_failure_text(markdown) {
        return Err(
            "content rejected: the response contains a model or tool failure message".to_string(),
        );
    }
    if deep_research_output_has_internal_leak(markdown)
        || deep_research_output_has_internal_leak(html)
    {
        return Err(
            "content rejected: the report contains internal workflow or tool-status text"
                .to_string(),
        );
    }
    if visible_char_count(markdown.trim()) < 120 {
        return Err(
            "content rejected: the report is shorter than 120 visible characters".to_string(),
        );
    }
    if !complete_html_document(html) {
        return Err(
            "content rejected: the rendered HTML document is incomplete or unsafe".to_string(),
        );
    }
    if !has_research_report_substance(markdown, html) {
        return Err(
            "content rejected: the report lacks substantive findings, explicit sources, or confidence and limitations"
                .to_string(),
        );
    }
    deep_research_report_source_trace_diagnostic(
        markdown,
        html,
        query,
        workflow_output,
        workflow_metadata,
    )
}

pub(crate) fn deep_research_report_rejection_diagnostic_from_answer_text(
    query: &str,
    answer_text: &str,
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> Option<String> {
    let answer = answer_text.trim();
    if answer.is_empty() {
        return Some("content rejected: the model returned an empty report".to_string());
    }
    if looks_like_deep_research_fallback_draft(answer) {
        return Some("content rejected: the response is an incomplete fallback draft".to_string());
    }
    if looks_like_deep_research_recovery_report(answer) {
        return Some(
            "content rejected: a recovery report cannot be published as a completed report"
                .to_string(),
        );
    }
    if is_deep_research_model_failure_text(answer) {
        return Some(
            "content rejected: the response contains a model or tool failure message".to_string(),
        );
    }
    if deep_research_output_has_internal_leak(answer) {
        return Some(
            "content rejected: the report contains internal workflow or tool-status text"
                .to_string(),
        );
    }
    let markdown = completed_report_markdown_from_answer_text(query, answer)?;
    let html = deep_research_completed_report_html(query, &markdown);
    validate_deep_research_completed_report_content(
        &markdown,
        &html,
        query,
        workflow_output,
        workflow_metadata,
    )
    .err()
}

fn deep_research_recovery_result_text(answer_text: &str, workflow_output: &str) -> String {
    let answer = answer_text.trim();
    let answer_lower = answer.to_ascii_lowercase();
    if answer_lower.starts_with("content rejected:")
        || answer_lower.starts_with("structured report")
        || answer_lower.starts_with("report plan rejected:")
    {
        return format!(
            "Host publication validation rejected the generated report: {}",
            deep_research_sanitize_evidence_text(answer)
        );
    }
    if visible_char_count(answer) >= 120
        && !is_deep_research_model_failure_text(answer)
        && !deep_research_output_has_internal_leak(answer)
        && has_report_source_anchor(answer)
        && (answer_lower.contains("## sources")
            || answer_lower.contains("\nsources:")
            || answer.contains("## 来源")
            || answer.contains("来源："))
    {
        return truncate_recovery_text(&deep_research_sanitize_evidence_text(answer), 20_000);
    }

    workflow_evidence_summary(workflow_output).unwrap_or_else(|| {
        "DeepResearch could not produce a reliable final synthesis because evidence collection ended before usable source-backed material was available. The run should be treated as incomplete for domain conclusions, but this report records the failure mode and the next recovery steps.".to_string()
    })
}

fn demote_recovery_result_title(markdown: &str) -> String {
    let mut in_fence = false;
    let mut output = Vec::new();
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            output.push(line.to_string());
            continue;
        }
        if !in_fence && trimmed.starts_with("# ") {
            let indent = &line[..line.len() - trimmed.len()];
            output.push(format!("{indent}#{trimmed}"));
        } else {
            output.push(line.to_string());
        }
    }
    let mut demoted = output.join("\n");
    if markdown.ends_with('\n') {
        demoted.push('\n');
    }
    demoted
}

fn truncate_recovery_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated = text
        .chars()
        .take(max_chars.saturating_sub(16))
        .collect::<String>();
    truncated.push_str("\n\n[truncated]");
    truncated
}

fn deep_research_recovery_evidence_status(
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
) -> String {
    let trimmed = workflow_output.trim();
    let metadata_sources = deep_research_workflow_source_anchors("", workflow_metadata).len();
    let metadata_note = (metadata_sources > 0).then(|| {
        format!(
            "recovery metadata preserved {metadata_sources} traceable source{} from successful research tools.",
            if metadata_sources == 1 { "" } else { "s" }
        )
    });
    if trimmed.is_empty() {
        if let Some(metadata_note) = metadata_note.as_deref() {
            return format!("No final workflow evidence output was captured, but {metadata_note}");
        }
        return "No workflow evidence output was captured before recovery report materialization."
            .to_string();
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let status = deep_research_collection_status(&value);
        let mode = value
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let mut summary = format!(
            "The evidence workflow ended with `{status}` collection status in `{mode}` mode. \
             The report above preserves only clean, user-facing evidence and omits internal workflow logs."
        );
        if let Some(coverage) = deep_research_recovery_coverage_summary(&value) {
            summary.push(' ');
            summary.push_str(&coverage);
        }
        if let Some(metadata_note) = metadata_note.as_deref() {
            summary.push(' ');
            summary.push_str(metadata_note);
        }
        return summary;
    }
    if deep_research_output_has_internal_leak(trimmed) {
        let mut summary = "The evidence workflow returned internal tool or workflow logs. Those logs were withheld from the user-facing report.".to_string();
        if let Some(metadata_note) = metadata_note.as_deref() {
            summary.push(' ');
            summary.push_str(metadata_note);
            summary.push_str(" Domain conclusions remain provisional.");
        } else {
            summary.push_str(" No source-backed conclusion is presented here.");
        }
        return summary;
    }
    let mut summary = "The evidence workflow ended without structured evidence. Raw provider/tool error text was withheld from the report; consult the terminal diagnostics for the bounded failure summary.".to_string();
    if let Some(metadata_note) = metadata_note.as_deref() {
        summary.push(' ');
        summary.push_str(metadata_note);
    }
    summary
}

fn deep_research_recovery_coverage_summary(value: &serde_json::Value) -> Option<String> {
    let research = value.get("research")?;
    let metadata = research.get("metadata");
    let successful_results = research
        .get("results")
        .and_then(serde_json::Value::as_array)
        .map(|results| {
            results
                .iter()
                .filter(|result| {
                    result.get("success").and_then(serde_json::Value::as_bool) != Some(false)
                        && result.get("structured").is_some()
                })
                .count() as u64
        })
        .unwrap_or(0);
    let warning_failures = research
        .pointer("/warnings/failed_tasks")
        .and_then(serde_json::Value::as_array)
        .map_or(0, |items| items.len() as u64)
        .saturating_add(
            research
                .pointer("/warnings/failed_rounds")
                .and_then(serde_json::Value::as_array)
                .map_or(0, |items| items.len() as u64),
        );
    let success_count = metadata
        .and_then(|item| item.get("success_count"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(successful_results);
    let failed_count = metadata
        .and_then(|item| item.get("failed_count"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(warning_failures);
    let task_count = metadata
        .and_then(|item| item.get("task_count"))
        .or_else(|| metadata.and_then(|item| item.get("result_count")))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_else(|| success_count.saturating_add(failed_count));
    let incomplete_count = failed_count.max(task_count.saturating_sub(success_count));
    let status = research
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if incomplete_count == 0 && status.eq_ignore_ascii_case("success") {
        return None;
    }
    if task_count > 0 {
        return Some(format!(
            "Coverage failure: only {success_count} of {task_count} planned research tasks produced validated evidence; {incomplete_count} failed, were interrupted, or did not pass validation."
        ));
    }
    Some(
        "Coverage failure: the collection did not complete every planned research task."
            .to_string(),
    )
}

fn deep_research_recovery_sources(
    workflow_output: &str,
    workflow_metadata: Option<&serde_json::Value>,
    slug: &str,
) -> String {
    let anchors = deep_research_workflow_source_anchors(workflow_output, workflow_metadata);
    let evidence_omitted =
        deep_research_workflow_evidence_omitted_count(workflow_output, workflow_metadata);
    if anchors.is_empty() {
        let mut lines = vec![
            "- No traceable external or local research source was captured before the workflow stopped."
                .to_string(),
        ];
        if evidence_omitted > 0 {
            lines.push(format!(
                "- At least {evidence_omitted} additional evidence item{} omitted from this bounded recovery view.",
                if evidence_omitted == 1 { " was" } else { "s were" }
            ));
        }
        lines.push(format!(
            "- Local run artifact: `.a3s/research/{slug}/report.md`."
        ));
        return lines.join("\n");
    }

    let mut lines = anchors
        .iter()
        .take(12)
        .map(|anchor| format!("- {anchor}"))
        .collect::<Vec<_>>();
    let omitted = anchors.len().saturating_sub(lines.len()).saturating_add(
        deep_research_workflow_source_omitted_count(workflow_output, workflow_metadata),
    );
    if omitted > 0 {
        lines.push(format!(
            "- At least {omitted} additional captured source entr{} omitted from this bounded recovery view.",
            if omitted == 1 { "y was" } else { "ies were" }
        ));
    }
    if evidence_omitted > 0 {
        lines.push(format!(
            "- At least {evidence_omitted} additional evidence item{} omitted from this bounded recovery view.",
            if evidence_omitted == 1 { " was" } else { "s were" }
        ));
    }
    lines.join("\n")
}
