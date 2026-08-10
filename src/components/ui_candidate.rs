//! Pre-cutover browser readiness for cognitive-package UI candidates.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use a3s_use::plugin_lifecycle::{
    PluginLifecycleEvidence, PluginLifecycleIntent, StaticPluginSurfaceLifecycleHost,
};
use a3s_use_core::{metadata_is_link_or_reparse_point, PlanScope, UseError, UseResult};
use a3s_use_extension::{inspect_ui_surface_files, PluginUiSurface};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{watch, Mutex};

const UI_CANDIDATE_MAX_PENDING: usize = 8;
const UI_CANDIDATE_HTML_MAX_BYTES: u64 = 2 * 1024 * 1024;
const UI_CANDIDATE_RESOURCE_MAX_BYTES: u64 = 2 * 1024 * 1024;
const UI_CANDIDATE_EVIDENCE_SCHEMA: &str = "a3s.code.plugin-ui-candidate-readiness.v1";

/// Browser decision accepted by the trusted Code Web host.
///
/// The package iframe cannot call the decision API because its CSP denies
/// connections and its sandbox has an opaque origin. Code Web translates only
/// bounded host-observed outcomes into this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodePluginUiCandidateDecision {
    Ready,
    LoadFailed,
    ProtocolError,
    NavigationBlocked,
    TimedOut,
}

impl CodePluginUiCandidateDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::LoadFailed => "load-failed",
            Self::ProtocolError => "protocol-error",
            Self::NavigationBlocked => "navigation-blocked",
            Self::TimedOut => "timed-out",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodePluginUiCandidate {
    pub token: String,
    pub scope: PlanScope,
    pub package_id: String,
    pub surface_id: String,
    pub generation: u64,
    pub title: String,
    pub asset_digest: String,
}

/// Fully memory-resident, bounded candidate document.
///
/// No managed filesystem path crosses the API boundary. Assets are rechecked
/// against the immutable package surface before this value is published.
#[derive(Debug, Clone)]
pub struct CodePluginUiCandidateContent {
    pub html: Arc<str>,
    pub styles: Vec<Arc<str>>,
    pub scripts: Vec<Arc<str>>,
}

#[derive(Debug, thiserror::Error)]
pub enum CodePluginUiCandidateError {
    #[error("plugin UI candidate token is invalid")]
    InvalidToken,
    #[error("plugin UI candidate is no longer pending")]
    NotFound,
    #[error("plugin UI candidate already has a terminal browser decision")]
    AlreadyDecided,
}

#[derive(Debug, Clone, Copy)]
enum CandidateReadinessMode {
    StaticOnly,
    BrowserRequired { timeout: Duration },
}

struct CandidateEntry {
    summary: CodePluginUiCandidate,
    content: CodePluginUiCandidateContent,
    decision: watch::Sender<Option<CodePluginUiCandidateDecision>>,
}

struct CandidateBrokerInner {
    mode: CandidateReadinessMode,
    pending: Mutex<BTreeMap<String, Arc<CandidateEntry>>>,
}

/// Process-local rendezvous between the package lifecycle and Code Web.
///
/// Candidate records are intentionally ephemeral. The Use lifecycle journal
/// remains the durable source of truth; after a process restart, exact-plan
/// recovery re-enters the same UI checkpoint and registers the same
/// deterministic token for a fresh browser proof.
#[derive(Clone)]
pub struct CodePluginUiCandidateBroker {
    inner: Arc<CandidateBrokerInner>,
}

impl std::fmt::Debug for CodePluginUiCandidateBroker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodePluginUiCandidateBroker")
            .field("mode", &self.inner.mode)
            .finish_non_exhaustive()
    }
}

impl Default for CodePluginUiCandidateBroker {
    fn default() -> Self {
        Self::static_only()
    }
}

impl CodePluginUiCandidateBroker {
    /// Composition for CLI and hosts that do not own a browser renderer yet.
    ///
    /// This preserves the existing integrity-only static projection contract;
    /// it must not be described as browser readiness.
    pub fn static_only() -> Self {
        Self::new(CandidateReadinessMode::StaticOnly)
    }

    /// Composition for Code Web. Capability publication waits for an exact
    /// candidate document to complete the isolated browser handshake.
    pub fn browser_required(timeout: Duration) -> Self {
        Self::new(CandidateReadinessMode::BrowserRequired { timeout })
    }

    fn new(mode: CandidateReadinessMode) -> Self {
        Self {
            inner: Arc::new(CandidateBrokerInner {
                mode,
                pending: Mutex::new(BTreeMap::new()),
            }),
        }
    }

    pub async fn pending(&self) -> Vec<CodePluginUiCandidate> {
        self.inner
            .pending
            .lock()
            .await
            .values()
            .map(|entry| entry.summary.clone())
            .collect()
    }

    pub async fn content(
        &self,
        token: &str,
    ) -> Result<CodePluginUiCandidateContent, CodePluginUiCandidateError> {
        validate_token(token)?;
        self.inner
            .pending
            .lock()
            .await
            .get(token)
            .map(|entry| entry.content.clone())
            .ok_or(CodePluginUiCandidateError::NotFound)
    }

    pub async fn decide(
        &self,
        token: &str,
        decision: CodePluginUiCandidateDecision,
    ) -> Result<(), CodePluginUiCandidateError> {
        validate_token(token)?;
        let pending = self.inner.pending.lock().await;
        let entry = pending
            .get(token)
            .ok_or(CodePluginUiCandidateError::NotFound)?;
        if entry.decision.borrow().is_some() {
            return Err(CodePluginUiCandidateError::AlreadyDecided);
        }
        entry.decision.send_replace(Some(decision));
        Ok(())
    }

    pub(crate) async fn prove_ready(
        &self,
        static_host: &StaticPluginSurfaceLifecycleHost,
        static_evidence: PluginLifecycleEvidence,
        intent: &PluginLifecycleIntent,
        surface: &PluginUiSurface,
        idempotency_key: &str,
    ) -> UseResult<PluginLifecycleEvidence> {
        let CandidateReadinessMode::BrowserRequired { timeout } = self.inner.mode else {
            return Ok(static_evidence);
        };
        let (content, asset_digest) =
            load_candidate_content(static_host.package_root(), surface).await?;
        let token = candidate_token(
            intent,
            surface,
            idempotency_key,
            static_evidence.digest(),
            &asset_digest,
        );
        let summary = CodePluginUiCandidate {
            token: token.clone(),
            scope: intent.scope.clone(),
            package_id: intent.package_id.clone(),
            surface_id: surface.id.clone(),
            generation: intent.generation,
            title: surface.title.clone(),
            asset_digest,
        };
        let entry = self.register(summary, content).await?;
        let mut receiver = entry.decision.subscribe();
        let decision = tokio::time::timeout(timeout, async {
            loop {
                if let Some(decision) = *receiver.borrow_and_update() {
                    return Ok(decision);
                }
                receiver.changed().await.map_err(|_| {
                    candidate_error(
                        "use.plugin.ui_candidate_readiness_lost",
                        "The Code Web UI readiness host stopped before deciding the candidate.",
                        intent,
                        surface,
                    )
                })?;
            }
        })
        .await;
        self.remove_if_same(&token, &entry).await;
        let decision = decision.map_err(|_| {
            candidate_error(
                "use.plugin.ui_candidate_readiness_timeout",
                "The candidate UI did not become ready before the host deadline.",
                intent,
                surface,
            )
        })??;
        if decision != CodePluginUiCandidateDecision::Ready {
            return Err(candidate_error(
                "use.plugin.ui_candidate_not_ready",
                "The isolated Code Web candidate UI failed before capability cutover.",
                intent,
                surface,
            )
            .with_detail("readinessOutcome", serde_json::json!(decision.as_str())));
        }
        readiness_evidence(&static_evidence, &token, intent, surface, idempotency_key)
    }

    async fn register(
        &self,
        summary: CodePluginUiCandidate,
        content: CodePluginUiCandidateContent,
    ) -> UseResult<Arc<CandidateEntry>> {
        let mut pending = self.inner.pending.lock().await;
        if let Some(existing) = pending.get(&summary.token) {
            if existing.summary != summary {
                return Err(UseError::new(
                    "use.plugin.ui_candidate_conflict",
                    "A UI readiness token was reused for different candidate evidence.",
                ));
            }
            return Ok(existing.clone());
        }
        if pending.len() >= UI_CANDIDATE_MAX_PENDING {
            return Err(UseError::new(
                "use.plugin.ui_candidate_capacity",
                "The Code Web UI readiness queue is full.",
            ));
        }
        let (decision, _) = watch::channel(None);
        let entry = Arc::new(CandidateEntry {
            summary: summary.clone(),
            content,
            decision,
        });
        pending.insert(summary.token, entry.clone());
        Ok(entry)
    }

    async fn remove_if_same(&self, token: &str, expected: &Arc<CandidateEntry>) {
        let mut pending = self.inner.pending.lock().await;
        if pending
            .get(token)
            .is_some_and(|current| Arc::ptr_eq(current, expected))
        {
            pending.remove(token);
        }
    }
}

async fn load_candidate_content(
    package_root: &Path,
    surface: &PluginUiSurface,
) -> UseResult<(CodePluginUiCandidateContent, String)> {
    let before = inspect_ui_surface_files(surface, package_root).await?;
    let root = canonical_owned_root(package_root).await?;
    let html =
        read_candidate_asset(&root, &surface.entry, UI_CANDIDATE_HTML_MAX_BYTES, "entry").await?;
    let mut styles = Vec::with_capacity(surface.styles.len());
    for path in &surface.styles {
        styles.push(
            read_candidate_asset(&root, path, UI_CANDIDATE_RESOURCE_MAX_BYTES, "style").await?,
        );
    }
    let mut scripts = Vec::with_capacity(surface.scripts.len());
    for path in &surface.scripts {
        scripts.push(
            read_candidate_asset(&root, path, UI_CANDIDATE_RESOURCE_MAX_BYTES, "script").await?,
        );
    }
    let after = inspect_ui_surface_files(surface, package_root).await?;
    if before != after {
        return Err(UseError::new(
            "use.plugin.ui_candidate_changed",
            "The immutable UI candidate changed while Code Web prepared its readiness document.",
        ));
    }
    Ok((
        CodePluginUiCandidateContent {
            html,
            styles,
            scripts,
        },
        before.digest().to_string(),
    ))
}

async fn canonical_owned_root(package_root: &Path) -> UseResult<PathBuf> {
    if !package_root.is_absolute() {
        return Err(candidate_asset_error("package root"));
    }
    let metadata = tokio::fs::symlink_metadata(package_root)
        .await
        .map_err(|_| candidate_asset_error("package root"))?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
        return Err(candidate_asset_error("package root"));
    }
    tokio::fs::canonicalize(package_root)
        .await
        .map_err(|_| candidate_asset_error("package root"))
}

async fn read_candidate_asset(
    root: &Path,
    relative: &Path,
    max_bytes: u64,
    label: &str,
) -> UseResult<Arc<str>> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(candidate_asset_error(label));
    }
    let mut path = root.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        path.push(component.as_os_str());
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|_| candidate_asset_error(label))?;
        if metadata_is_link_or_reparse_point(&metadata)
            || (index + 1 == components.len() && !metadata.is_file())
            || (index + 1 < components.len() && !metadata.is_dir())
        {
            return Err(candidate_asset_error(label));
        }
        if index + 1 == components.len() && (metadata.len() == 0 || metadata.len() > max_bytes) {
            return Err(candidate_asset_error(label));
        }
    }
    let canonical = tokio::fs::canonicalize(&path)
        .await
        .map_err(|_| candidate_asset_error(label))?;
    if !canonical.starts_with(root) {
        return Err(candidate_asset_error(label));
    }
    let bytes = tokio::fs::read(canonical)
        .await
        .map_err(|_| candidate_asset_error(label))?;
    if bytes.is_empty() || bytes.len() as u64 > max_bytes {
        return Err(candidate_asset_error(label));
    }
    String::from_utf8(bytes)
        .map(Arc::from)
        .map_err(|_| candidate_asset_error(label))
}

fn candidate_token(
    intent: &PluginLifecycleIntent,
    surface: &PluginUiSurface,
    idempotency_key: &str,
    static_evidence_digest: &str,
    asset_digest: &str,
) -> String {
    let identity = format!(
        "{UI_CANDIDATE_EVIDENCE_SCHEMA}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        intent.operation_id,
        intent.plan_digest,
        intent.scope.kind.as_str(),
        intent.scope.id,
        intent.package_id,
        intent.generation,
        surface.id,
        idempotency_key,
        static_evidence_digest,
    );
    format!(
        "{:x}",
        Sha256::digest(format!("{identity}\n{asset_digest}").as_bytes())
    )
}

fn readiness_evidence(
    static_evidence: &PluginLifecycleEvidence,
    token: &str,
    intent: &PluginLifecycleIntent,
    surface: &PluginUiSurface,
    idempotency_key: &str,
) -> UseResult<PluginLifecycleEvidence> {
    let evidence = format!(
        "{UI_CANDIDATE_EVIDENCE_SCHEMA}\nready\n{}\n{token}\n{}\n{}\n{}\n{}",
        static_evidence.digest(),
        intent.package_id,
        intent.generation,
        surface.id,
        idempotency_key,
    );
    PluginLifecycleEvidence::new(format!("sha256:{:x}", Sha256::digest(evidence.as_bytes())))
}

fn candidate_error(
    code: &'static str,
    message: &'static str,
    intent: &PluginLifecycleIntent,
    surface: &PluginUiSurface,
) -> UseError {
    UseError::new(code, message)
        .with_detail("packageId", serde_json::json!(intent.package_id))
        .with_detail("surfaceId", serde_json::json!(surface.id))
        .with_detail("generation", serde_json::json!(intent.generation))
}

fn candidate_asset_error(label: &str) -> UseError {
    UseError::new(
        "use.plugin.ui_candidate_asset_invalid",
        format!(
            "The Code Web UI candidate {label} is missing, unsafe, oversized, or invalid UTF-8."
        ),
    )
}

fn validate_token(token: &str) -> Result<(), CodePluginUiCandidateError> {
    if token.len() == 64
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(CodePluginUiCandidateError::InvalidToken)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use a3s_use::plugin_lifecycle::{
        PluginLifecycleAction, PluginLifecycleIntentSpec, PluginUiLifecycleHost,
    };
    use a3s_use_core::PlanScopeKind;
    use a3s_use_extension::ExtensionManifest;

    use super::*;

    #[tokio::test]
    async fn browser_ready_candidate_returns_bound_evidence_and_disappears() {
        let fixture = fixture();
        let broker = CodePluginUiCandidateBroker::browser_required(Duration::from_secs(2));
        let host = StaticPluginSurfaceLifecycleHost::new(&fixture.package_root);
        let intent = fixture.intent();
        let surface = &fixture.manifest.ui[0];
        let static_evidence = host
            .prepare_ui(&intent, surface, "candidate-ready")
            .await
            .unwrap();
        let expected_static = static_evidence.clone();
        let broker_for_task = broker.clone();
        let intent_for_task = intent.clone();
        let surface_for_task = surface.clone();
        let task = tokio::spawn(async move {
            broker_for_task
                .prove_ready(
                    &host,
                    static_evidence,
                    &intent_for_task,
                    &surface_for_task,
                    "candidate-ready",
                )
                .await
        });

        let candidate = wait_for_candidate(&broker).await;
        assert_eq!(candidate.package_id, intent.package_id);
        assert_eq!(candidate.surface_id, surface.id);
        assert_eq!(candidate.generation, intent.generation);
        let content = broker.content(&candidate.token).await.unwrap();
        assert!(content.html.contains("Candidate Activity"));
        assert!(content.scripts[0].contains("activity.ready"));
        broker
            .decide(&candidate.token, CodePluginUiCandidateDecision::Ready)
            .await
            .unwrap();

        let evidence = task.await.unwrap().unwrap();
        assert_ne!(evidence, expected_static);
        assert!(broker.pending().await.is_empty());
        assert!(matches!(
            broker.content(&candidate.token).await,
            Err(CodePluginUiCandidateError::NotFound)
        ));
        assert!(matches!(
            broker
                .decide(&candidate.token, CodePluginUiCandidateDecision::Ready)
                .await,
            Err(CodePluginUiCandidateError::NotFound)
        ));
    }

    #[tokio::test]
    async fn candidate_accepts_only_one_terminal_decision() {
        let fixture = fixture();
        let broker = CodePluginUiCandidateBroker::browser_required(Duration::from_secs(2));
        let host = StaticPluginSurfaceLifecycleHost::new(&fixture.package_root);
        let intent = fixture.intent();
        let surface = fixture.manifest.ui[0].clone();
        let static_evidence = host
            .prepare_ui(&intent, &surface, "candidate-one-decision")
            .await
            .unwrap();
        let broker_for_task = broker.clone();
        let intent_for_task = intent.clone();
        let task = tokio::spawn(async move {
            broker_for_task
                .prove_ready(
                    &host,
                    static_evidence,
                    &intent_for_task,
                    &surface,
                    "candidate-one-decision",
                )
                .await
        });
        let candidate = wait_for_candidate(&broker).await;

        broker
            .decide(&candidate.token, CodePluginUiCandidateDecision::Ready)
            .await
            .unwrap();
        assert!(matches!(
            broker
                .decide(
                    &candidate.token,
                    CodePluginUiCandidateDecision::ProtocolError,
                )
                .await,
            Err(CodePluginUiCandidateError::AlreadyDecided | CodePluginUiCandidateError::NotFound)
        ));
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn failed_and_timed_out_candidates_fail_before_cutover() {
        let fixture = fixture();
        let broker = CodePluginUiCandidateBroker::browser_required(Duration::from_secs(2));
        let host = StaticPluginSurfaceLifecycleHost::new(&fixture.package_root);
        let intent = fixture.intent();
        let surface = fixture.manifest.ui[0].clone();
        let static_evidence = host
            .prepare_ui(&intent, &surface, "candidate-failed")
            .await
            .unwrap();
        let broker_for_task = broker.clone();
        let intent_for_task = intent.clone();
        let task = tokio::spawn(async move {
            broker_for_task
                .prove_ready(
                    &host,
                    static_evidence,
                    &intent_for_task,
                    &surface,
                    "candidate-failed",
                )
                .await
        });
        let candidate = wait_for_candidate(&broker).await;
        broker
            .decide(
                &candidate.token,
                CodePluginUiCandidateDecision::ProtocolError,
            )
            .await
            .unwrap();
        assert_eq!(
            task.await.unwrap().unwrap_err().code,
            "use.plugin.ui_candidate_not_ready"
        );
        assert!(broker.pending().await.is_empty());

        let broker = CodePluginUiCandidateBroker::browser_required(Duration::from_millis(20));
        let host = StaticPluginSurfaceLifecycleHost::new(&fixture.package_root);
        let surface = &fixture.manifest.ui[0];
        let static_evidence = host
            .prepare_ui(&intent, surface, "candidate-timeout")
            .await
            .unwrap();
        let error = broker
            .prove_ready(
                &host,
                static_evidence,
                &intent,
                surface,
                "candidate-timeout",
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "use.plugin.ui_candidate_readiness_timeout");
        assert!(broker.pending().await.is_empty());
    }

    #[tokio::test]
    async fn static_only_composition_does_not_publish_a_browser_claim() {
        let fixture = fixture();
        let broker = CodePluginUiCandidateBroker::static_only();
        let host = StaticPluginSurfaceLifecycleHost::new(&fixture.package_root);
        let intent = fixture.intent();
        let surface = &fixture.manifest.ui[0];
        let static_evidence = host
            .prepare_ui(&intent, surface, "static-only")
            .await
            .unwrap();
        let result = broker
            .prove_ready(
                &host,
                static_evidence.clone(),
                &intent,
                surface,
                "static-only",
            )
            .await
            .unwrap();
        assert_eq!(result, static_evidence);
        assert!(broker.pending().await.is_empty());
    }

    async fn wait_for_candidate(broker: &CodePluginUiCandidateBroker) -> CodePluginUiCandidate {
        for _ in 0..100 {
            if let Some(candidate) = broker.pending().await.into_iter().next() {
                return candidate;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("candidate did not become pending");
    }

    struct Fixture {
        _temporary: tempfile::TempDir,
        package_root: PathBuf,
        manifest: ExtensionManifest,
    }

    impl Fixture {
        fn intent(&self) -> PluginLifecycleIntent {
            PluginLifecycleIntent::from_manifest(
                PluginLifecycleIntentSpec {
                    operation_id: "ui-candidate-upgrade".to_string(),
                    plan_digest: format!("sha256:{}", "1".repeat(64)),
                    scope: PlanScope {
                        kind: PlanScopeKind::User,
                        id: "user/current".to_string(),
                    },
                    package_id: self.manifest.package_id.clone(),
                    package_digest: format!("sha256:{}", "2".repeat(64)),
                    manifest_digest: format!("sha256:{}", "3".repeat(64)),
                    generation: 8,
                    action: PluginLifecycleAction::Upgrade,
                    retained_ui_state_surfaces: vec!["review".to_string()],
                },
                &self.manifest,
            )
            .unwrap()
        }
    }

    fn fixture() -> Fixture {
        let temporary = tempfile::tempdir().unwrap();
        let package_root = temporary.path().join("package");
        std::fs::create_dir_all(package_root.join("ui")).unwrap();
        std::fs::write(
            package_root.join("ui/review.html"),
            "<!doctype html><main>Candidate Activity</main>",
        )
        .unwrap();
        std::fs::write(
            package_root.join("ui/review.js"),
            "port.postMessage({ protocol: 'a3s.activity.v3', type: 'activity.ready' });",
        )
        .unwrap();
        let manifest = ExtensionManifest::parse_acl(
            r#"
extension "acme/research" {
  schema_version = 3
  version = "2.0.0"
  route = "research"
  requires_use = ">=0.3.0, <0.4.0"
  actions = ["read"]

  repository {
    url = "https://github.com/acme/research"
    revision = "0123456789abcdef0123456789abcdef01234567"
  }

  ui "review" {
    entry = "ui/review.html"
    styles = []
    scripts = ["ui/review.js"]
    bind_tool = []
    bind_mcp = []
    bind_flow = []
    optional = false
  }
}
"#,
        )
        .unwrap();
        assert!(manifest.ui.iter().all(|surface| surface.id == "review"));
        Fixture {
            _temporary: temporary,
            package_root,
            manifest,
        }
    }
}
