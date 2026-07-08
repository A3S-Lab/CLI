use std::path::{Path, PathBuf};
use std::sync::Arc;

use a3s_boot::{BootError, Result as BootResult};
use serde_json::{json, Value};

use super::controller::{KbAddNoteRequest, KbImportRequest, KbSearchRequest};
use crate::api::code_web::state::CodeWebState;
use crate::tui::kbutil::{self, ImportKind, ImportPreview, KbStats, SearchHit};

pub(in crate::api::code_web) struct KnowledgeService {
    state: Arc<CodeWebState>,
}

impl KnowledgeService {
    pub(in crate::api::code_web) fn new(state: Arc<CodeWebState>) -> Self {
        Self { state }
    }

    pub(in crate::api::code_web) async fn kb_home(
        &self,
        workspace: Option<String>,
    ) -> BootResult<Value> {
        let workspace = self.workspace_from_request(workspace);
        self.ensure_workspace(&workspace).await?;
        Ok(kb_home_json(&workspace))
    }

    pub(in crate::api::code_web) async fn ensure(
        &self,
        workspace: Option<String>,
    ) -> BootResult<Value> {
        let workspace = self.workspace_from_request(workspace);
        self.ensure_workspace(&workspace).await?;
        Ok(kb_home_json(&workspace))
    }

    pub(in crate::api::code_web) async fn add_note(
        &self,
        request: KbAddNoteRequest,
    ) -> BootResult<Value> {
        let text = request.text.trim().to_string();
        if text.is_empty() {
            return Err(BootError::BadRequest("note text is required".to_string()));
        }
        let workspace = self.workspace_from_request(request.workspace);
        self.ensure_workspace(&workspace).await?;
        let workspace_text = workspace.display().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let summary = tokio::task::spawn_blocking(move || {
            kbutil::add_text_to_kb(&workspace_text, &text, &now)
        })
        .await
        .map_err(|error| BootError::Internal(format!("failed to add KB note: {error}")))?;
        Ok(kb_action_json(&workspace, summary, "note"))
    }

    pub(in crate::api::code_web) async fn import_preview(
        &self,
        request: KbImportRequest,
    ) -> BootResult<Value> {
        let path = required_path_arg(&request.path)?;
        let workspace = self.workspace_from_request(request.workspace);
        let workspace_text = workspace.display().to_string();
        let preview =
            tokio::task::spawn_blocking(move || kbutil::preview_import(&workspace_text, &path))
                .await
                .map_err(|error| {
                    BootError::Internal(format!("failed to preview KB import: {error}"))
                })?
                .map_err(BootError::BadRequest)?;
        Ok(json!({
            "workspaceRoot": workspace.display().to_string(),
            "kbRoot": kbutil::kb_dir(&workspace.display().to_string()).display().to_string(),
            "preview": import_preview_json(&preview),
        }))
    }

    pub(in crate::api::code_web) async fn import(
        &self,
        request: KbImportRequest,
    ) -> BootResult<Value> {
        let path = required_path_arg(&request.path)?;
        let workspace = self.workspace_from_request(request.workspace);
        self.ensure_workspace(&workspace).await?;
        let workspace_text = workspace.display().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let summary =
            tokio::task::spawn_blocking(move || kbutil::import_to_kb(&workspace_text, &path, &now))
                .await
                .map_err(|error| {
                    BootError::Internal(format!("failed to import into KB: {error}"))
                })?;
        Ok(kb_action_json(&workspace, summary, "import"))
    }

    pub(in crate::api::code_web) async fn search(
        &self,
        request: KbSearchRequest,
    ) -> BootResult<Value> {
        let query = request.query.trim().to_string();
        if query.is_empty() {
            return Err(BootError::BadRequest("query is required".to_string()));
        }
        let workspace = self.workspace_from_request(request.workspace);
        let workspace_text = workspace.display().to_string();
        let query_for_task = query.clone();
        let hits = tokio::task::spawn_blocking(move || {
            kbutil::search_kb(&workspace_text, &query_for_task)
        })
        .await
        .map_err(|error| BootError::Internal(format!("failed to search KB: {error}")))?;
        Ok(json!({
            "workspaceRoot": workspace.display().to_string(),
            "kbRoot": kbutil::kb_dir(&workspace.display().to_string()).display().to_string(),
            "query": query,
            "hits": hits.iter().map(search_hit_json).collect::<Vec<_>>(),
            "total": hits.len(),
        }))
    }

    async fn ensure_workspace(&self, workspace: &Path) -> BootResult<()> {
        let kb_root = kbutil::kb_dir(&workspace.display().to_string());
        tokio::fs::create_dir_all(kb_root.join("sources"))
            .await
            .map_err(fs_error)?;
        tokio::fs::create_dir_all(kb_root.join("wiki"))
            .await
            .map_err(fs_error)?;
        Ok(())
    }

    fn workspace_from_request(&self, workspace: Option<String>) -> PathBuf {
        workspace
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.state.default_workspace.clone())
    }
}

fn kb_home_json(workspace: &Path) -> Value {
    let workspace_text = workspace.display().to_string();
    let stats = kbutil::kb_stats(&workspace_text);
    json!({
        "workspaceRoot": workspace_text,
        "kbRoot": kbutil::kb_dir(&workspace.display().to_string()).display().to_string(),
        "stats": kb_stats_json(&stats),
        "recent": kbutil::recent_sources(&workspace.display().to_string(), 8),
    })
}

fn kb_action_json(workspace: &Path, summary: String, action: &str) -> Value {
    let home = kb_home_json(workspace);
    json!({
        "action": action,
        "success": !summary.trim_start().starts_with('\u{2717}'),
        "summary": summary,
        "workspaceRoot": home["workspaceRoot"].clone(),
        "kbRoot": home["kbRoot"].clone(),
        "stats": home["stats"].clone(),
        "recent": home["recent"].clone(),
    })
}

fn kb_stats_json(stats: &KbStats) -> Value {
    json!({
        "sources": stats.sources,
        "concepts": stats.concepts,
        "imports": stats.imports,
        "bytes": stats.bytes,
    })
}

fn import_preview_json(preview: &ImportPreview) -> Value {
    json!({
        "arg": preview.arg,
        "path": preview.path.display().to_string(),
        "kind": match preview.kind {
            ImportKind::File => "file",
            ImportKind::Folder => "folder",
        },
        "addable": preview.addable,
        "skipped": preview.skipped,
        "capped": preview.capped,
        "bytes": preview.bytes,
    })
}

fn search_hit_json(hit: &SearchHit) -> Value {
    json!({
        "path": hit.path,
        "line": hit.line,
        "snippet": hit.snippet,
    })
}

fn required_path_arg(path: &str) -> BootResult<String> {
    let path = path.trim();
    if path.is_empty() {
        return Err(BootError::BadRequest("path is required".to_string()));
    }
    Ok(path.to_string())
}

fn fs_error(error: std::io::Error) -> BootError {
    BootError::Internal(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kb_stats_json_matches_tui_kb_stats_fields() {
        let stats = KbStats {
            sources: 2,
            concepts: 3,
            imports: 1,
            bytes: 128,
        };
        assert_eq!(
            kb_stats_json(&stats),
            json!({
                "sources": 2,
                "concepts": 3,
                "imports": 1,
                "bytes": 128,
            })
        );
    }

    #[test]
    fn import_preview_json_keeps_tui_preview_contract() {
        let preview = ImportPreview {
            arg: "notes".to_string(),
            path: PathBuf::from("/tmp/notes"),
            kind: ImportKind::Folder,
            addable: 4,
            skipped: 1,
            capped: true,
            bytes: 4096,
        };
        let json = import_preview_json(&preview);
        assert_eq!(json["kind"], "folder");
        assert_eq!(json["addable"], 4);
        assert_eq!(json["skipped"], 1);
        assert_eq!(json["capped"], true);
        assert_eq!(json["bytes"], 4096);
    }
}
