//! `/packages` reviewed cognitive-package enablement panel.

use super::super::*;
use a3s_tui::components::{MenuItem, MenuPanel};
use a3s_use::plugin_manager::{PluginManagerInstalledPackage, PluginManagerService};
use a3s_use_core::{
    PlanActor, PlanPolicyDecision, PlanScopeKind, PluginDesiredState, PluginHostApplyResult,
    PluginHostEnablementPlanResult, PluginHostEnablementPlanStatus, PluginManagedScope,
    PluginManagerApplyPlanInput, PluginManagerListInstalledInput, PluginManagerPackageScopeInput,
    PluginObservedState, PluginOperationConfirmation, PluginPackageId,
    PLUGIN_OPERATION_CONFIRMATION_SCHEMA,
};
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use a3s::plugin_manager::review::{plan_review_fields, PluginPlanReviewField};

const MAX_PACKAGE_PANEL_ITEMS: usize = 1_000;
const PACKAGE_PANEL_PAGE_LIMIT: u16 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PackagePanelRow {
    component_id: String,
    package_id: String,
    version: String,
    enabled: bool,
    desired: PluginDesiredState,
    observed: PluginObservedState,
}

impl TryFrom<PluginManagerInstalledPackage> for PackagePanelRow {
    type Error = String;

    fn try_from(package: PluginManagerInstalledPackage) -> Result<Self, Self::Error> {
        package
            .state
            .validate()
            .map_err(|error| format!("installed package state is invalid: {error}"))?;
        let package_id = PluginPackageId::parse(package.package_id)
            .map_err(|error| format!("installed package identity is invalid: {error}"))?;
        let version = package.state.version.clone().ok_or_else(|| {
            format!(
                "installed package '{}' omitted its version",
                package_id.as_str()
            )
        })?;
        let desired = package.state.desired;
        Ok(Self {
            component_id: package_id.component_id(),
            package_id: package_id.into_string(),
            version,
            enabled: desired == PluginDesiredState::Enabled,
            desired,
            observed: package.state.observed,
        })
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
    desired_before: PluginDesiredState,
    assignment_generation: u64,
    capabilities_digest: String,
    scope: PluginManagedScope,
    details: Vec<PluginPlanReviewField>,
}

impl PackagePlanReview {
    fn action(&self) -> &'static str {
        if self.enabled {
            "enable"
        } else {
            "disable"
        }
    }

    fn target_state(&self) -> PluginDesiredState {
        if self.enabled {
            PluginDesiredState::Enabled
        } else {
            PluginDesiredState::InstalledDisabled
        }
    }

    fn apply_input(&self) -> PluginManagerApplyPlanInput {
        PluginManagerApplyPlanInput {
            operation_id: self.operation_id.clone(),
            plan_digest: self.plan_digest.clone(),
        }
    }

    fn confirmation(&self) -> Result<PluginOperationConfirmation, String> {
        let confirmation = PluginOperationConfirmation {
            schema: PLUGIN_OPERATION_CONFIRMATION_SCHEMA.to_string(),
            operation_id: self.operation_id.clone(),
            plan_digest: self.plan_digest.clone(),
            confirmed_by: PlanActor::User,
            confirmed_at_ms: unix_time_millis()?,
        };
        confirmation
            .validate()
            .map_err(|error| format!("could not bind exact user confirmation: {error}"))?;
        Ok(confirmation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PackagePlanOutcome {
    NoChange,
    Planned(Box<PackagePlanReview>),
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
    review_scroll: usize,
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
            review_scroll: 0,
            rows,
            phase: PackagePanelPhase::Loading,
            note,
        }
    }

    fn selected_row(&self) -> Option<&PackagePanelRow> {
        self.rows
            .get(self.selected.min(self.rows.len().saturating_sub(1)))
    }

    fn apply_snapshot(
        &mut self,
        packages: Vec<PluginManagerInstalledPackage>,
    ) -> Result<(), String> {
        let selected_package = self.selected_row().map(|row| row.package_id.clone());
        let mut rows = packages
            .into_iter()
            .map(PackagePanelRow::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        rows.sort_by(|left, right| left.package_id.cmp(&right.package_id));
        self.selected = selected_package
            .as_deref()
            .and_then(|package_id| rows.iter().position(|row| row.package_id == package_id))
            .unwrap_or_else(|| self.selected.min(rows.len().saturating_sub(1)));
        self.rows = rows;
        self.review_scroll = 0;
        self.phase = PackagePanelPhase::Ready;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackagePanelKeyAction {
    MoveUp,
    MoveDown,
    Plan,
    Apply,
    ReviewUp,
    ReviewDown,
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
            KeyCode::Up => PackagePanelKeyAction::ReviewUp,
            KeyCode::Down => PackagePanelKeyAction::ReviewDown,
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

fn review_plan(
    result: PluginHostEnablementPlanResult,
    expected_component_id: &str,
    expected_enabled: bool,
) -> Result<PackagePlanOutcome, String> {
    result
        .validate()
        .map_err(|error| format!("reviewed enablement plan is invalid: {error}"))?;
    let component_id = result.package_id.component_id();
    if component_id != expected_component_id || result.enabled != expected_enabled {
        return Err("reviewed enablement plan changed its requested target".to_string());
    }
    if result.status == PluginHostEnablementPlanStatus::NoChange {
        return Ok(PackagePlanOutcome::NoChange);
    }
    let envelope = result.plan.as_ref().ok_or_else(|| {
        "reviewed enablement planned a mutation without an immutable plan".to_string()
    })?;
    if envelope.plan.component_id != component_id
        || envelope.plan.authority.actor != PlanActor::User
        || envelope.plan.authority.decision != PlanPolicyDecision::Ask
        || !envelope.plan.authority.confirmation_required
    {
        return Err("reviewed enablement plan identity or authority drifted".to_string());
    }
    let desired_before = result.state.desired;
    let details = plan_review_fields(envelope)
        .map_err(|error| format!("could not project the exact reviewed plan: {error}"))?;
    Ok(PackagePlanOutcome::Planned(Box::new(PackagePlanReview {
        component_id,
        package_id: result.package_id.into_string(),
        enabled: result.enabled,
        expected_package_generation: result.expected_package_generation,
        operation_id: envelope.plan.operation_id.clone(),
        plan_digest: envelope.plan_digest.clone(),
        expires_at_ms: envelope.plan.expires_at_ms,
        desired_before,
        assignment_generation: result.assignment_generation,
        capabilities_digest: result.capabilities_digest,
        scope: result.scope,
        details,
    })))
}

fn review_apply_result(
    result: PluginHostApplyResult,
    review: &PackagePlanReview,
) -> Result<PackageApplyOutcome, String> {
    result
        .validate()
        .map_err(|error| format!("reviewed enablement apply result is invalid: {error}"))?;
    let generation = result
        .state
        .package_generation
        .filter(|generation| *generation > review.expected_package_generation)
        .ok_or_else(|| {
            "reviewed enablement apply did not advance the package generation".to_string()
        })?;
    if result.package_id.as_str() != review.package_id
        || result.operation_id != review.operation_id
        || result.plan_digest != review.plan_digest
        || result.assignment_generation != review.assignment_generation
        || result.capabilities_digest != review.capabilities_digest
        || result.scope != review.scope
        || result.state.desired != review.target_state()
    {
        return Err("reviewed enablement apply result drifted from the confirmed plan".to_string());
    }
    Ok(PackageApplyOutcome {
        generation,
        replayed: result.replayed,
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
                    "desired={} · observed={} · v{}",
                    desired_state_name(row.desired),
                    observed_state_name(row.observed),
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

fn review_detail_lines(review: &PackagePlanReview, width: usize) -> Vec<String> {
    let mut lines = wrapped_field(
        "state",
        &format!(
            "{} -> {}",
            desired_state_name(review.desired_before),
            desired_state_name(review.target_state())
        ),
        width,
    );
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
    for field in &review.details {
        lines.extend(wrapped_field(&field.label, &field.value, width));
    }
    lines
}

fn review_viewport_height(height: usize, note: bool) -> usize {
    height.saturating_sub(if note { 6 } else { 5 }).clamp(3, 18)
}

fn review_max_scroll(
    review: &PackagePlanReview,
    note: Option<&str>,
    width: usize,
    height: usize,
) -> usize {
    review_detail_lines(review, width)
        .len()
        .saturating_sub(review_viewport_height(height, note.is_some()))
}

fn review_lines(
    review: &PackagePlanReview,
    note: Option<&str>,
    width: usize,
    height: usize,
    scroll: usize,
) -> Vec<String> {
    let mut lines = vec![bounded(
        &format!("Review {} — {}", review.action(), review.package_id),
        width,
    )];
    let details = review_detail_lines(review, width);
    let viewport = review_viewport_height(height, note.is_some());
    let scroll = scroll.min(details.len().saturating_sub(viewport));
    let end = scroll.saturating_add(viewport).min(details.len());
    lines.extend(details[scroll..end].iter().cloned());
    lines.push(bounded(
        &format!(
            "details {}-{} of {}",
            scroll.saturating_add(1).min(details.len()),
            end,
            details.len()
        ),
        width,
    ));
    if let Some(note) = note {
        lines.extend(wrapped_field("status", note, width));
    }
    let full_hint = "↑/↓ review · Enter/y apply exact plan · Esc/n cancel";
    let compact_hint = "↑↓ review · y apply exact · n cancel";
    lines.push(bounded(
        if a3s_tui::style::visible_len(full_hint) <= width {
            full_hint
        } else {
            compact_hint
        },
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

fn desired_state_name(state: PluginDesiredState) -> &'static str {
    match state {
        PluginDesiredState::Absent => "absent",
        PluginDesiredState::InstalledDisabled => "installed-disabled",
        PluginDesiredState::Enabled => "enabled",
    }
}

fn observed_state_name(state: PluginObservedState) -> &'static str {
    match state {
        PluginObservedState::Installed => "installed",
        PluginObservedState::Reconciling => "reconciling",
        PluginObservedState::Ready => "ready",
        PluginObservedState::Degraded => "degraded",
        PluginObservedState::Broken => "broken",
        PluginObservedState::Incompatible => "incompatible",
        PluginObservedState::Draining => "draining",
        PluginObservedState::Removed => "removed",
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
        PackagePanelPhase::Review(review) => review_lines(
            review,
            panel.note.as_deref(),
            width,
            height,
            panel.review_scroll,
        ),
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

async fn installed_packages(
    service: &PluginManagerService,
) -> Result<Vec<PluginManagerInstalledPackage>, String> {
    let mut cursor = None;
    let mut snapshot_digest = None;
    let mut identities = BTreeSet::new();
    let mut packages = Vec::new();
    loop {
        let page = service
            .list_installed(PluginManagerListInstalledInput {
                scope_kind: PlanScopeKind::User,
                scope_id: a3s_use::COGNITIVE_PACKAGE_DEFAULT_SCOPE.to_string(),
                cursor,
                limit: Some(PACKAGE_PANEL_PAGE_LIMIT),
            })
            .await
            .map_err(|error| error.to_string())?;
        if snapshot_digest
            .as_deref()
            .is_some_and(|digest| digest != page.snapshot_digest)
        {
            return Err("installed package state changed while the panel was loading".to_string());
        }
        snapshot_digest = Some(page.snapshot_digest);
        for package in page.packages {
            if !identities.insert(package.package_id.clone()) {
                return Err("installed package list contained a duplicate identity".to_string());
            }
            packages.push(package);
            if packages.len() > MAX_PACKAGE_PANEL_ITEMS {
                return Err(format!(
                    "installed package list exceeds the supported {MAX_PACKAGE_PANEL_ITEMS} items"
                ));
            }
        }
        let Some(next_cursor) = page.next_cursor else {
            return Ok(packages);
        };
        cursor = Some(next_cursor);
    }
}

fn unix_time_millis() -> Result<u64, String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_millis();
    u64::try_from(millis)
        .map_err(|_| "system time exceeds the supported millisecond range".to_string())
        .and_then(|millis| {
            (millis > 0)
                .then_some(millis)
                .ok_or_else(|| "system time must be positive".to_string())
        })
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
        let service = self.plugin_manager_service.clone();
        let unavailable = self.plugin_manager_error.clone().unwrap_or_else(|| {
            "the shared Plugin Manager was not initialized for this session".to_string()
        });
        Some(cmd::cmd(move || async move {
            let result = match service {
                Some(service) => installed_packages(&service).await,
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
        result: Result<Vec<PluginManagerInstalledPackage>, String>,
    ) {
        let Some(panel) = self.package_panel.as_mut() else {
            return;
        };
        if panel.request_id != request_id || panel.phase != PackagePanelPhase::Loading {
            return;
        }
        match result {
            Ok(packages) => {
                if let Err(error) = panel.apply_snapshot(packages) {
                    panel.phase = PackagePanelPhase::Unavailable(error);
                    panel.rows.clear();
                    panel.selected = 0;
                }
            }
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
        let service = self.plugin_manager_service.clone();
        let unavailable = self.plugin_manager_error.clone().unwrap_or_else(|| {
            "the shared Plugin Manager was not initialized for this session".to_string()
        });
        Some(cmd::cmd(move || async move {
            let result = match service {
                Some(service) => match PluginPackageId::parse(row.package_id.clone()) {
                    Ok(package_id) => {
                        let input = PluginManagerPackageScopeInput {
                            package_id,
                            scope_kind: PlanScopeKind::User,
                            scope_id: a3s_use::COGNITIVE_PACKAGE_DEFAULT_SCOPE.to_string(),
                        };
                        if enabled {
                            service.plan_enable(input).await
                        } else {
                            service.plan_disable(input).await
                        }
                        .map_err(|error| error.to_string())
                    }
                    Err(error) => Err(error.to_string()),
                },
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
        result: Result<PluginHostEnablementPlanResult, String>,
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
        match result.and_then(|result| review_plan(result, &component_id, enabled)) {
            Ok(PackagePlanOutcome::NoChange) => self.request_package_snapshot(Some(format!(
                "No change: {} is already {}.",
                component_id,
                if enabled { "enabled" } else { "disabled" }
            ))),
            Ok(PackagePlanOutcome::Planned(review)) => {
                panel.phase = PackagePanelPhase::Review(*review);
                panel.note = None;
                panel.review_scroll = 0;
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
        let request = review.apply_input();
        let operation_id = request.operation_id.clone();
        let service = self.plugin_manager_service.clone();
        let unavailable = self.plugin_manager_error.clone().unwrap_or_else(|| {
            "the shared Plugin Manager was not initialized for this session".to_string()
        });
        Some(cmd::cmd(move || async move {
            let result = match service {
                Some(service) => match review.confirmation() {
                    Ok(confirmation) => service
                        .apply_plan(request, Some(confirmation))
                        .await
                        .map_err(|error| error.to_string()),
                    Err(error) => Err(error),
                },
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
        result: Result<PluginHostApplyResult, String>,
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
        match result.and_then(|result| review_apply_result(result, &review)) {
            Ok(outcome) => self.request_package_snapshot(Some(format!(
                "{} {} · generation {}{}.",
                review.package_id,
                desired_state_name(review.target_state()),
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
            PackagePanelKeyAction::ReviewUp => {
                if let Some(panel) = self.package_panel.as_mut() {
                    panel.review_scroll = panel.review_scroll.saturating_sub(1);
                }
                None
            }
            PackagePanelKeyAction::ReviewDown => {
                let max_scroll = self.package_panel.as_ref().and_then(|panel| {
                    let PackagePanelPhase::Review(review) = &panel.phase else {
                        return None;
                    };
                    Some(review_max_scroll(
                        review,
                        panel.note.as_deref(),
                        self.width as usize,
                        self.height as usize,
                    ))
                });
                if let (Some(panel), Some(max_scroll)) = (self.package_panel.as_mut(), max_scroll) {
                    panel.review_scroll = panel.review_scroll.saturating_add(1).min(max_scroll);
                }
                None
            }
            PackagePanelKeyAction::Back => {
                if let Some(panel) = self.package_panel.as_mut() {
                    panel.phase = PackagePanelPhase::Ready;
                    panel.review_scroll = 0;
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
