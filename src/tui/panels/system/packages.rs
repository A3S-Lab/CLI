//! `/packages` reviewed cognitive-package enablement panel.

use super::super::*;
use a3s::plugin_manager::{
    PluginEnablementApplyRequest, PluginEnablementPlanRequest, PluginInstallationSnapshot,
    PluginInstalledPackage, PluginPackageReadiness,
};
use a3s_tui::components::{MenuItem, MenuPanel};
use serde_json::Value;

const PACKAGE_PLAN_RESULT_SCHEMA: &str = "a3s.cli.plugin-enablement-plan-result.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
struct PackagePanelRow {
    component_id: String,
    package_id: String,
    version: String,
    enabled: bool,
    callable: bool,
    readiness: PluginPackageReadiness,
}

impl From<PluginInstalledPackage> for PackagePanelRow {
    fn from(package: PluginInstalledPackage) -> Self {
        Self {
            component_id: package.component_id,
            package_id: package.package_id,
            version: package.version,
            enabled: package.enabled,
            callable: package.callable,
            readiness: package.readiness,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PackagePlanReview {
    component_id: String,
    package_id: String,
    enabled: bool,
    expected_package_generation: u64,
    operation_id: String,
    plan_digest: String,
    expires_at_ms: u64,
    desired_before: String,
}

impl PackagePlanReview {
    fn action(&self) -> &'static str {
        if self.enabled {
            "enable"
        } else {
            "disable"
        }
    }

    fn target_state(&self) -> &'static str {
        if self.enabled {
            "enabled"
        } else {
            "installed-disabled"
        }
    }

    fn apply_request(&self) -> PluginEnablementApplyRequest {
        PluginEnablementApplyRequest {
            operation_id: self.operation_id.clone(),
            plan_digest: self.plan_digest.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PackagePlanOutcome {
    NoChange,
    Planned(PackagePlanReview),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PackageApplyOutcome {
    generation: u64,
    replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PackagePanelPhase {
    Loading,
    Ready,
    Planning { component_id: String, enabled: bool },
    Review(PackagePlanReview),
    Applying(PackagePlanReview),
    Unavailable(String),
}

#[derive(Debug, Clone)]
pub(crate) struct PackagePanel {
    request_id: u64,
    selected: usize,
    rows: Vec<PackagePanelRow>,
    phase: PackagePanelPhase,
    note: Option<String>,
}

impl PackagePanel {
    fn loading(
        request_id: u64,
        selected: usize,
        rows: Vec<PackagePanelRow>,
        note: Option<String>,
    ) -> Self {
        Self {
            request_id,
            selected,
            rows,
            phase: PackagePanelPhase::Loading,
            note,
        }
    }

    fn selected_row(&self) -> Option<&PackagePanelRow> {
        self.rows
            .get(self.selected.min(self.rows.len().saturating_sub(1)))
    }

    fn apply_snapshot(&mut self, snapshot: PluginInstallationSnapshot) {
        if !snapshot.available {
            self.phase = PackagePanelPhase::Unavailable(
                snapshot
                    .error
                    .unwrap_or_else(|| "A3S Use installation state is unavailable".to_string()),
            );
            self.rows.clear();
            self.selected = 0;
            return;
        }

        let selected_package = self.selected_row().map(|row| row.package_id.clone());
        let mut rows = snapshot
            .items
            .into_iter()
            .map(PackagePanelRow::from)
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.package_id.cmp(&right.package_id));
        self.selected = selected_package
            .as_deref()
            .and_then(|package_id| rows.iter().position(|row| row.package_id == package_id))
            .unwrap_or_else(|| self.selected.min(rows.len().saturating_sub(1)));
        self.rows = rows;
        self.phase = PackagePanelPhase::Ready;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackagePanelKeyAction {
    MoveUp,
    MoveDown,
    Plan,
    Apply,
    Back,
    Refresh,
    Close,
    Ignore,
}

fn key_action(phase: &PackagePanelPhase, key: &KeyEvent) -> PackagePanelKeyAction {
    match phase {
        PackagePanelPhase::Ready => match key.code {
            KeyCode::Up => PackagePanelKeyAction::MoveUp,
            KeyCode::Down => PackagePanelKeyAction::MoveDown,
            KeyCode::Enter | KeyCode::Char(' ') => PackagePanelKeyAction::Plan,
            KeyCode::Char('r' | 'R') => PackagePanelKeyAction::Refresh,
            KeyCode::Esc => PackagePanelKeyAction::Close,
            _ => PackagePanelKeyAction::Ignore,
        },
        PackagePanelPhase::Review(_) => match key.code {
            KeyCode::Enter if key.modifiers == KeyModifiers::NONE => PackagePanelKeyAction::Apply,
            KeyCode::Char('y') if key.modifiers == KeyModifiers::NONE => {
                PackagePanelKeyAction::Apply
            }
            KeyCode::Char('Y')
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                PackagePanelKeyAction::Apply
            }
            KeyCode::Esc | KeyCode::Char('n' | 'N') => PackagePanelKeyAction::Back,
            _ => PackagePanelKeyAction::Ignore,
        },
        PackagePanelPhase::Unavailable(_) => match key.code {
            KeyCode::Char('r' | 'R') => PackagePanelKeyAction::Refresh,
            KeyCode::Esc => PackagePanelKeyAction::Close,
            _ => PackagePanelKeyAction::Ignore,
        },
        PackagePanelPhase::Loading | PackagePanelPhase::Planning { .. } => {
            if key.code == KeyCode::Esc {
                PackagePanelKeyAction::Close
            } else {
                PackagePanelKeyAction::Ignore
            }
        }
        PackagePanelPhase::Applying(_) => PackagePanelKeyAction::Ignore,
    }
}

fn parse_plan(
    value: &Value,
    expected_component_id: &str,
    expected_enabled: bool,
) -> Result<PackagePlanOutcome, String> {
    if required_string(value, "/schema")? != PACKAGE_PLAN_RESULT_SCHEMA {
        return Err("reviewed enablement returned an unsupported plan schema".to_string());
    }
    let component_id = required_string(value, "/componentId")?;
    let enabled = value
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| "reviewed enablement plan omitted enabled".to_string())?;
    if component_id != expected_component_id || enabled != expected_enabled {
        return Err("reviewed enablement plan changed its requested target".to_string());
    }
    let package_id = required_string(value, "/packageId")?.to_string();
    if component_id != format!("use/{package_id}") {
        return Err("reviewed enablement plan changed its package identity".to_string());
    }
    let expected_package_generation = value
        .get("expectedPackageGeneration")
        .and_then(Value::as_u64)
        .filter(|generation| *generation > 0)
        .ok_or_else(|| {
            "reviewed enablement plan omitted its expected package generation".to_string()
        })?;
    if value
        .pointer("/state/packageGeneration")
        .and_then(Value::as_u64)
        != Some(expected_package_generation)
    {
        return Err("reviewed enablement plan changed its package generation".to_string());
    }
    let desired_before = required_string(value, "/state/desired")?.to_string();
    let target_state = if enabled {
        "enabled"
    } else {
        "installed-disabled"
    };
    let status = required_string(value, "/status")?;
    if status == "no-change" {
        if value.get("operationId").is_some()
            || value.get("canonicalPlanDigest").is_some()
            || value.get("plan").is_some()
        {
            return Err("NoChange carried synthetic mutation identity".to_string());
        }
        if desired_before != target_state {
            return Err("NoChange did not match the requested package state".to_string());
        }
        return Ok(PackagePlanOutcome::NoChange);
    }
    if status != "planned" {
        return Err(format!(
            "reviewed enablement returned unsupported status '{status}'"
        ));
    }
    if desired_before == target_state {
        return Err("planned enablement did not describe a state transition".to_string());
    }

    let operation_id = required_string(value, "/operationId")?.to_string();
    a3s_use_core::PluginOperationPlan::validate_operation_id(&operation_id)
        .map_err(|_| "reviewed enablement returned an invalid operation ID".to_string())?;
    let plan_digest = required_string(value, "/canonicalPlanDigest")?.to_string();
    if !valid_sha256(&plan_digest) {
        return Err("reviewed enablement returned an invalid canonical digest".to_string());
    }
    let nested_operation_id = required_string(value, "/plan/plan/operationId")?;
    let nested_digest = required_string(value, "/plan/planDigest")?;
    let nested_component_id = required_string(value, "/plan/plan/componentId")?;
    let nested_package_id = required_string(value, "/plan/plan/packageId")?;
    let action = required_string(value, "/plan/plan/action")?;
    let actor = required_string(value, "/plan/plan/authority/actor")?;
    let expires_at_ms = value
        .pointer("/plan/plan/expiresAtMs")
        .and_then(Value::as_u64)
        .filter(|expires_at_ms| *expires_at_ms > 0)
        .ok_or_else(|| "reviewed enablement plan omitted its expiry".to_string())?;
    let expected_action = if expected_enabled {
        "enable"
    } else {
        "disable"
    };
    if operation_id != nested_operation_id
        || plan_digest != nested_digest
        || component_id != nested_component_id
        || package_id != nested_package_id
        || action != expected_action
        || actor != "user"
    {
        return Err("reviewed enablement plan identity or authority drifted".to_string());
    }
    Ok(PackagePlanOutcome::Planned(PackagePlanReview {
        component_id: component_id.to_string(),
        package_id,
        enabled,
        expected_package_generation,
        operation_id,
        plan_digest,
        expires_at_ms,
        desired_before,
    }))
}

fn parse_apply_result(
    value: &Value,
    review: &PackagePlanReview,
) -> Result<PackageApplyOutcome, String> {
    if required_string(value, "/schema")?
        != a3s_use::cognitive_package::COGNITIVE_PACKAGE_ENABLEMENT_RESULT_SCHEMA
    {
        return Err("reviewed enablement apply returned an unsupported result schema".to_string());
    }
    let generation = value
        .pointer("/state/packageGeneration")
        .and_then(Value::as_u64)
        .filter(|generation| *generation > review.expected_package_generation)
        .ok_or_else(|| {
            "reviewed enablement apply did not advance the package generation".to_string()
        })?;
    let replayed = value
        .get("replayed")
        .and_then(Value::as_bool)
        .ok_or_else(|| "reviewed enablement apply omitted replay state".to_string())?;
    let changed = value
        .get("changed")
        .and_then(Value::as_bool)
        .ok_or_else(|| "reviewed enablement apply omitted changed state".to_string())?;
    let durable = value
        .get("durableEnablement")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let result_digest = required_string(value, "/operationResultDigest")?;
    if required_string(value, "/componentId")? != review.component_id
        || required_string(value, "/packageId")? != review.package_id
        || required_string(value, "/operationId")? != review.operation_id
        || required_string(value, "/canonicalPlanDigest")? != review.plan_digest
        || required_string(value, "/state/desired")? != review.target_state()
        || !changed
        || !durable
        || !valid_sha256(result_digest)
    {
        return Err("reviewed enablement apply result drifted from the confirmed plan".to_string());
    }
    Ok(PackageApplyOutcome {
        generation,
        replayed,
    })
}

fn required_string<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("reviewed enablement response omitted {pointer}"))
}

fn valid_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn package_panel_max_items(height: usize, note: bool) -> usize {
    height
        .saturating_sub(if note { 10 } else { 9 })
        .clamp(2, 10)
}

fn list_lines(panel: &PackagePanel, width: usize, height: usize) -> Vec<String> {
    if panel.rows.is_empty() {
        return message_lines(
            "Cognitive packages",
            "No installed cognitive packages.",
            "r refresh · Esc close",
            panel.note.as_deref(),
            width,
        );
    }
    let selected = panel.selected.min(panel.rows.len() - 1);
    let max_items = package_panel_max_items(height, panel.note.is_some());
    let scroll = selected.saturating_add(1).saturating_sub(max_items);
    let enabled = panel.rows.iter().filter(|row| row.enabled).count();
    let items = panel
        .rows
        .iter()
        .map(|row| {
            MenuItem::new(row.package_id.clone())
                .description(format!(
                    "desired={} · callable={} · {} · v{}",
                    if row.enabled { "enabled" } else { "disabled" },
                    row.callable,
                    readiness_name(row.readiness),
                    row.version,
                ))
                .checked(row.enabled)
                .color(if row.enabled { TN_CYAN } else { TN_GRAY })
        })
        .collect::<Vec<_>>();
    let max_label_width = width.saturating_sub(44).clamp(10, 28);
    let label_width = panel
        .rows
        .iter()
        .map(|row| a3s_tui::style::visible_len(&row.package_id))
        .max()
        .unwrap_or(10)
        .clamp(10, max_label_width);
    let menu = MenuPanel::new(format!(
        "Cognitive packages ({enabled}/{} enabled) — Enter review toggle · r refresh · Esc",
        panel.rows.len()
    ))
    .items(items)
    .selected(selected)
    .scroll(scroll)
    .max_items(max_items)
    .label_width(label_width)
    .show_scroll(panel.rows.len() > max_items)
    .indent(2)
    .marker("▸")
    .title_color(ACCENT)
    .text_color(TN_GRAY)
    .muted_color(TN_GRAY)
    .checked_color(TN_GREEN)
    .selected_colors(TN_FG, SURFACE_SELECTED);
    let mut lines = menu
        .view(
            width.min(u16::MAX as usize) as u16,
            max_items.saturating_add(3),
        )
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if let Some(note) = panel.note.as_deref() {
        lines.extend(wrapped_field("status", note, width));
    }
    lines
}

fn review_lines(review: &PackagePlanReview, note: Option<&str>, width: usize) -> Vec<String> {
    let mut lines = vec![bounded(
        &format!("Review {} — {}", review.action(), review.package_id),
        width,
    )];
    lines.extend(wrapped_field(
        "state",
        &format!("{} -> {}", review.desired_before, review.target_state()),
        width,
    ));
    lines.extend(wrapped_field(
        "generation",
        &review.expected_package_generation.to_string(),
        width,
    ));
    lines.extend(wrapped_field("operation", &review.operation_id, width));
    lines.extend(wrapped_field("digest", &review.plan_digest, width));
    lines.extend(wrapped_field(
        "expiresAtMs",
        &review.expires_at_ms.to_string(),
        width,
    ));
    if let Some(note) = note {
        lines.extend(wrapped_field("status", note, width));
    }
    lines.push(bounded(
        "Enter/y apply this exact plan · Esc/n cancel",
        width,
    ));
    lines
}

fn message_lines(
    title: &str,
    detail: &str,
    hint: &str,
    note: Option<&str>,
    width: usize,
) -> Vec<String> {
    let mut lines = vec![bounded(title, width)];
    lines.extend(wrapped_field("status", detail, width));
    if let Some(note) = note {
        lines.extend(wrapped_field("note", note, width));
    }
    lines.push(bounded(hint, width));
    lines
}

fn wrapped_field(label: &str, value: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let prefix = format!("{label}: ");
    if prefix.chars().count() >= width {
        let mut lines = vec![bounded(&prefix, width)];
        lines.extend(wrap_chunks(value, width, ""));
        return lines;
    }
    let available = width - prefix.chars().count();
    wrap_chunks(value, available, &prefix)
}

fn wrap_chunks(value: &str, chunk_width: usize, prefix: &str) -> Vec<String> {
    let chunk_width = chunk_width.max(1);
    let characters = value.chars().collect::<Vec<_>>();
    if characters.is_empty() {
        return vec![prefix.to_string()];
    }
    let indent = " ".repeat(prefix.chars().count());
    characters
        .chunks(chunk_width)
        .enumerate()
        .map(|(index, chunk)| {
            let start = if index == 0 { prefix } else { indent.as_str() };
            format!("{start}{}", chunk.iter().collect::<String>())
        })
        .collect()
}

fn bounded(value: &str, width: usize) -> String {
    a3s_tui::style::truncate_visible(value, width)
}

fn readiness_name(readiness: PluginPackageReadiness) -> &'static str {
    match readiness {
        PluginPackageReadiness::Ready => "ready",
        PluginPackageReadiness::Missing => "missing",
        PluginPackageReadiness::Broken => "broken",
        PluginPackageReadiness::Unknown => "unknown",
    }
}

fn panel_lines(panel: &PackagePanel, width: usize, height: usize) -> Vec<String> {
    match &panel.phase {
        PackagePanelPhase::Loading => message_lines(
            "Cognitive packages",
            "loading installed package state...",
            "Esc close",
            panel.note.as_deref(),
            width,
        ),
        PackagePanelPhase::Ready => list_lines(panel, width, height),
        PackagePanelPhase::Planning {
            component_id,
            enabled,
        } => message_lines(
            "Cognitive packages",
            &format!(
                "planning {} for {component_id}...",
                if *enabled { "enable" } else { "disable" }
            ),
            "Esc close",
            panel.note.as_deref(),
            width,
        ),
        PackagePanelPhase::Review(review) => review_lines(review, panel.note.as_deref(), width),
        PackagePanelPhase::Applying(review) => message_lines(
            &format!("Applying {} — {}", review.action(), review.package_id),
            "durable apply in progress; the panel remains locked until completion",
            "waiting for exact result...",
            panel.note.as_deref(),
            width,
        ),
        PackagePanelPhase::Unavailable(error) => message_lines(
            "Cognitive packages unavailable",
            error,
            "r retry · Esc close",
            panel.note.as_deref(),
            width,
        ),
    }
}

impl App {
    fn next_package_panel_request_id(&mut self) -> u64 {
        self.package_panel_seq = self.package_panel_seq.wrapping_add(1).max(1);
        self.package_panel_seq
    }

    pub(crate) fn open_package_panel(&mut self) -> Option<Cmd<Msg>> {
        self.request_package_snapshot(None)
    }

    fn request_package_snapshot(&mut self, note: Option<String>) -> Option<Cmd<Msg>> {
        let request_id = self.next_package_panel_request_id();
        let (selected, rows) = self
            .package_panel
            .as_ref()
            .map_or((0, Vec::new()), |panel| {
                (panel.selected, panel.rows.clone())
            });
        self.package_panel = Some(PackagePanel::loading(request_id, selected, rows, note));
        let manager = self.plugin_manager.clone();
        let unavailable = self.plugin_manager_error.clone().unwrap_or_else(|| {
            "the shared Plugin Manager was not initialized for this session".to_string()
        });
        Some(cmd::cmd(move || async move {
            let result = match manager {
                Some(manager) => Ok(manager.installation_snapshot().await),
                None => Err(unavailable),
            };
            Msg::PackagePanelLoaded {
                request_id,
                result: Box::new(result),
            }
        }))
    }

    pub(crate) fn apply_package_panel_snapshot(
        &mut self,
        request_id: u64,
        result: Result<PluginInstallationSnapshot, String>,
    ) {
        let Some(panel) = self.package_panel.as_mut() else {
            return;
        };
        if panel.request_id != request_id || panel.phase != PackagePanelPhase::Loading {
            return;
        }
        match result {
            Ok(snapshot) => panel.apply_snapshot(snapshot),
            Err(error) => panel.phase = PackagePanelPhase::Unavailable(error),
        }
    }

    fn begin_package_enablement_plan(&mut self) -> Option<Cmd<Msg>> {
        let row = self.package_panel.as_ref()?.selected_row()?.clone();
        let enabled = !row.enabled;
        let request_id = self.next_package_panel_request_id();
        let panel = self.package_panel.as_mut()?;
        panel.request_id = request_id;
        panel.note = None;
        panel.phase = PackagePanelPhase::Planning {
            component_id: row.component_id.clone(),
            enabled,
        };
        let manager = self.plugin_manager.clone();
        let unavailable = self.plugin_manager_error.clone().unwrap_or_else(|| {
            "the shared Plugin Manager was not initialized for this session".to_string()
        });
        Some(cmd::cmd(move || async move {
            let result = match manager {
                Some(manager) => manager
                    .plan_package_enablement(&PluginEnablementPlanRequest {
                        component_id: row.component_id.clone(),
                        enabled,
                        expected_package_generation: None,
                    })
                    .await
                    .map_err(|error| error.to_string()),
                None => Err(unavailable),
            };
            Msg::PackageEnablementPlanned {
                request_id,
                component_id: row.component_id,
                enabled,
                result: Box::new(result),
            }
        }))
    }

    pub(crate) fn apply_package_enablement_plan(
        &mut self,
        request_id: u64,
        component_id: String,
        enabled: bool,
        result: Result<Value, String>,
    ) -> Option<Cmd<Msg>> {
        let panel = self.package_panel.as_mut()?;
        if panel.request_id != request_id
            || !matches!(
                &panel.phase,
                PackagePanelPhase::Planning {
                    component_id: expected_component_id,
                    enabled: expected_enabled,
                } if expected_component_id == &component_id && *expected_enabled == enabled
            )
        {
            return None;
        }
        match result.and_then(|value| parse_plan(&value, &component_id, enabled)) {
            Ok(PackagePlanOutcome::NoChange) => self.request_package_snapshot(Some(format!(
                "No change: {} is already {}.",
                component_id,
                if enabled { "enabled" } else { "disabled" }
            ))),
            Ok(PackagePlanOutcome::Planned(review)) => {
                panel.phase = PackagePanelPhase::Review(review);
                panel.note = None;
                None
            }
            Err(error) => {
                panel.phase = PackagePanelPhase::Ready;
                panel.note = Some(format!("Plan failed: {error}"));
                None
            }
        }
    }

    fn begin_package_enablement_apply(&mut self) -> Option<Cmd<Msg>> {
        let review = match self.package_panel.as_ref()?.phase.clone() {
            PackagePanelPhase::Review(review) => review,
            _ => return None,
        };
        let request_id = self.next_package_panel_request_id();
        let panel = self.package_panel.as_mut()?;
        panel.request_id = request_id;
        panel.note = None;
        panel.phase = PackagePanelPhase::Applying(review.clone());
        let request = review.apply_request();
        let operation_id = request.operation_id.clone();
        let manager = self.plugin_manager.clone();
        let unavailable = self.plugin_manager_error.clone().unwrap_or_else(|| {
            "the shared Plugin Manager was not initialized for this session".to_string()
        });
        Some(cmd::cmd(move || async move {
            let result = match manager {
                Some(manager) => manager
                    .apply_confirmed_package_enablement(&request)
                    .await
                    .map_err(|error| error.to_string()),
                None => Err(unavailable),
            };
            Msg::PackageEnablementApplied {
                request_id,
                operation_id,
                result: Box::new(result),
            }
        }))
    }

    pub(crate) fn apply_package_enablement_result(
        &mut self,
        request_id: u64,
        operation_id: String,
        result: Result<Value, String>,
    ) -> Option<Cmd<Msg>> {
        let panel = self.package_panel.as_mut()?;
        let review = match &panel.phase {
            PackagePanelPhase::Applying(review)
                if panel.request_id == request_id && review.operation_id == operation_id =>
            {
                review.clone()
            }
            _ => return None,
        };
        match result.and_then(|value| parse_apply_result(&value, &review)) {
            Ok(outcome) => self.request_package_snapshot(Some(format!(
                "{} {} · generation {}{}.",
                review.package_id,
                review.target_state(),
                outcome.generation,
                if outcome.replayed { " · replayed" } else { "" }
            ))),
            Err(error) => {
                panel.phase = PackagePanelPhase::Ready;
                panel.note = Some(format!(
                    "Apply failed: {error}. Select the package to create a fresh plan."
                ));
                None
            }
        }
    }

    pub(crate) fn handle_package_panel_key(&mut self, key: &KeyEvent) -> Option<Cmd<Msg>> {
        let action = self
            .package_panel
            .as_ref()
            .map_or(PackagePanelKeyAction::Ignore, |panel| {
                key_action(&panel.phase, key)
            });
        match action {
            PackagePanelKeyAction::MoveUp => {
                if let Some(panel) = self.package_panel.as_mut() {
                    panel.selected = panel.selected.saturating_sub(1);
                }
                None
            }
            PackagePanelKeyAction::MoveDown => {
                if let Some(panel) = self.package_panel.as_mut() {
                    panel.selected = (panel.selected + 1).min(panel.rows.len().saturating_sub(1));
                }
                None
            }
            PackagePanelKeyAction::Plan => self.begin_package_enablement_plan(),
            PackagePanelKeyAction::Apply => self.begin_package_enablement_apply(),
            PackagePanelKeyAction::Back => {
                if let Some(panel) = self.package_panel.as_mut() {
                    panel.phase = PackagePanelPhase::Ready;
                    panel.note =
                        Some("Reviewed plan cancelled; no mutation was applied.".to_string());
                }
                None
            }
            PackagePanelKeyAction::Refresh => self.request_package_snapshot(None),
            PackagePanelKeyAction::Close => {
                self.package_panel = None;
                None
            }
            PackagePanelKeyAction::Ignore => None,
        }
    }

    pub(crate) fn overlay_packages(&self, composed: String) -> String {
        let Some(panel) = self.package_panel.as_ref() else {
            return composed;
        };
        let lines = panel_lines(panel, self.width as usize, self.height as usize);
        self.overlay_list(composed, &lines)
    }
}

#[cfg(test)]
#[path = "packages/tests.rs"]
mod tests;
