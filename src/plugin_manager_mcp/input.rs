use std::collections::BTreeSet;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::PluginToolError;

const CURRENT_SCOPE_ID: &str = "current";

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum SurfaceKind {
    Mcp,
    Skill,
    Tool,
    Ui,
}

impl SurfaceKind {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Skill => "skill",
            Self::Tool => "tool",
            Self::Ui => "ui",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Channel {
    Stable,
    Beta,
    Nightly,
}

impl Channel {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Nightly => "nightly",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ScopeKind {
    User,
    Workspace,
}

impl ScopeKind {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Workspace => "workspace",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SearchInput {
    pub query: String,
    pub kind: Option<SurfaceKind>,
    pub channel: Option<Channel>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

impl SearchInput {
    pub(super) fn validate(&mut self) -> Result<(), PluginToolError> {
        self.query = bounded_nonempty(&self.query, "query", 256)?;
        validate_cursor(self.cursor.as_deref())?;
        bounded_limit(self.limit.unwrap_or(20), 50, "search limit")?;
        Ok(())
    }

    pub(super) fn limit(&self) -> usize {
        self.limit.unwrap_or(20)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct InspectInput {
    pub package_id: String,
    pub version: Option<String>,
    pub channel: Option<Channel>,
}

impl InspectInput {
    pub(super) fn validate(&self) -> Result<(), PluginToolError> {
        validate_package_id(&self.package_id)?;
        if let Some(version) = &self.version {
            validate_exact_version(version, "version")?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ListInput {
    pub scope_kind: ScopeKind,
    pub scope_id: String,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

impl ListInput {
    pub(super) fn validate(&self) -> Result<(), PluginToolError> {
        validate_scope(self.scope_kind, &self.scope_id)?;
        validate_cursor(self.cursor.as_deref())?;
        bounded_limit(self.limit.unwrap_or(50), 100, "installed list limit")?;
        Ok(())
    }

    pub(super) fn limit(&self) -> usize {
        self.limit.unwrap_or(50)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PackageScopeInput {
    pub package_id: String,
    pub scope_kind: ScopeKind,
    pub scope_id: String,
}

impl PackageScopeInput {
    pub(super) fn validate(&self) -> Result<(), PluginToolError> {
        validate_package_id(&self.package_id)?;
        validate_scope(self.scope_kind, &self.scope_id)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SelectedSurface {
    kind: SurfaceKind,
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PlanInput {
    pub package_id: String,
    pub version_requirement: Option<String>,
    pub channel: Option<Channel>,
    pub surfaces: Option<Vec<SelectedSurface>>,
    pub scope_kind: ScopeKind,
    pub scope_id: String,
}

impl PlanInput {
    pub(super) fn validate(&self) -> Result<(), PluginToolError> {
        validate_package_id(&self.package_id)?;
        validate_scope(self.scope_kind, &self.scope_id)?;
        if let Some(requirement) = &self.version_requirement {
            exact_version_requirement(requirement)?;
        }
        if let Some(surfaces) = &self.surfaces {
            if surfaces.is_empty() || surfaces.len() > 256 {
                return Err(PluginToolError::invalid(
                    "surfaces must contain from 1 to 256 entries",
                ));
            }
            let mut identities = BTreeSet::new();
            for surface in surfaces {
                validate_segment(&surface.id, "surface ID")?;
                if !identities.insert((surface.kind.as_str(), surface.id.as_str())) {
                    return Err(PluginToolError::invalid(
                        "surfaces must not contain duplicate kind and ID pairs",
                    ));
                }
            }
            return Err(PluginToolError::new(
                "plugin.surface_selection_unsupported",
                "This host release plans complete plugin packages only; omit surfaces or install a narrower package.",
                false,
            ));
        }
        Ok(())
    }

    pub(super) fn exact_version(&self) -> Result<Option<String>, PluginToolError> {
        self.version_requirement
            .as_deref()
            .map(exact_version_requirement)
            .transpose()
    }
}

pub(super) fn parse<T: DeserializeOwned>(
    arguments: Option<Map<String, Value>>,
) -> Result<T, PluginToolError> {
    serde_json::from_value(Value::Object(arguments.unwrap_or_default())).map_err(|error| {
        PluginToolError::invalid(format!(
            "tool arguments do not match the frozen schema: {error}"
        ))
    })
}

pub(super) fn scope_value(kind: ScopeKind, id: &str) -> Value {
    serde_json::json!({
        "kind": kind.as_str(),
        "id": id,
    })
}

fn validate_scope(kind: ScopeKind, id: &str) -> Result<(), PluginToolError> {
    validate_machine_id(id, "scopeId")?;
    if !matches!(kind, ScopeKind::User) || id != CURRENT_SCOPE_ID {
        return Err(PluginToolError::new(
            "plugin.scope_unsupported",
            "This host release supports only scopeKind `user` with scopeId `current`.",
            false,
        ));
    }
    Ok(())
}

fn validate_package_id(value: &str) -> Result<(), PluginToolError> {
    let segments = value.split('/').collect::<Vec<_>>();
    if segments.len() != 2 {
        return Err(PluginToolError::invalid(
            "packageId must use canonical publisher/name syntax",
        ));
    }
    validate_segment(segments[0], "publisher")?;
    validate_segment(segments[1], "package name")
}

fn validate_segment(value: &str, label: &str) -> Result<(), PluginToolError> {
    let mut characters = value.chars();
    let valid = value.len() <= 63
        && matches!(characters.next(), Some(first) if first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        });
    if valid {
        Ok(())
    } else {
        Err(PluginToolError::invalid(format!(
            "{label} is not a canonical lowercase identifier"
        )))
    }
}

fn validate_machine_id(value: &str, label: &str) -> Result<(), PluginToolError> {
    let mut characters = value.chars();
    let valid = value.len() <= 256
        && matches!(characters.next(), Some(first) if first.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '.' | '_' | ':' | '/' | '@' | '-')
        });
    if valid {
        Ok(())
    } else {
        Err(PluginToolError::invalid(format!("{label} is invalid")))
    }
}

fn exact_version_requirement(value: &str) -> Result<String, PluginToolError> {
    let value = bounded_nonempty(value, "versionRequirement", 64)?;
    let exact = value.strip_prefix('=').unwrap_or(&value);
    validate_exact_version(exact, "versionRequirement")?;
    Ok(exact.to_string())
}

fn validate_exact_version(value: &str, label: &str) -> Result<(), PluginToolError> {
    let parsed = semver::Version::parse(value).map_err(|_| {
        PluginToolError::new(
            "plugin.version_requirement_unsupported",
            format!(
                "{label} must be one exact canonical semantic version; omit it to select the latest compatible channel release"
            ),
            false,
        )
    })?;
    if parsed.to_string() != value {
        return Err(PluginToolError::new(
            "plugin.version_requirement_unsupported",
            format!("{label} must use canonical semantic version syntax"),
            false,
        ));
    }
    Ok(())
}

fn validate_cursor(value: Option<&str>) -> Result<(), PluginToolError> {
    if let Some(value) = value {
        bounded_nonempty(value, "cursor", 512)?;
    }
    Ok(())
}

fn bounded_nonempty(
    value: &str,
    label: &str,
    max_characters: usize,
) -> Result<String, PluginToolError> {
    let value = value.trim();
    let characters = value.chars().count();
    if characters == 0 || characters > max_characters {
        return Err(PluginToolError::invalid(format!(
            "{label} must contain from 1 to {max_characters} characters"
        )));
    }
    Ok(value.to_string())
}

fn bounded_limit(value: usize, maximum: usize, label: &str) -> Result<(), PluginToolError> {
    if value == 0 || value > maximum {
        return Err(PluginToolError::invalid(format!(
            "{label} must be from 1 to {maximum}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_reject_unknown_fields_and_unbound_scope() {
        let unknown = serde_json::from_value::<SearchInput>(serde_json::json!({
            "query": "science",
            "registryUrl": "https://example.invalid"
        }));
        assert!(unknown.is_err());

        let workspace = PackageScopeInput {
            package_id: "a3s/science".to_string(),
            scope_kind: ScopeKind::Workspace,
            scope_id: "current".to_string(),
        };
        assert_eq!(
            workspace.validate().unwrap_err().code,
            "plugin.scope_unsupported"
        );
    }

    #[test]
    fn plan_accepts_only_exact_versions_and_complete_packages() {
        let exact = PlanInput {
            package_id: "a3s/science".to_string(),
            version_requirement: Some("=1.2.3".to_string()),
            channel: Some(Channel::Stable),
            surfaces: None,
            scope_kind: ScopeKind::User,
            scope_id: "current".to_string(),
        };
        exact.validate().unwrap();
        assert_eq!(exact.exact_version().unwrap().as_deref(), Some("1.2.3"));

        let mut ranged = exact;
        ranged.version_requirement = Some("^1.2".to_string());
        assert_eq!(
            ranged.validate().unwrap_err().code,
            "plugin.version_requirement_unsupported"
        );
    }
}
