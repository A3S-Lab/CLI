use std::net::IpAddr;

use a3s_acl::{Block, Value};
use a3s_use_core::{HttpMethod, PlanPolicyDecision, PluginSurfaceKind};

use super::{policy_error, PolicyFilesystemAccess, MAX_POLICY_RULES};
use crate::plugin_manager::PluginManagerResult;

pub(super) fn decision(block: &Block, name: &str) -> PluginManagerResult<PlanPolicyDecision> {
    match optional_string(block, name)?.unwrap_or("ask") {
        "allow" => Ok(PlanPolicyDecision::Allow),
        "ask" => Ok(PlanPolicyDecision::Ask),
        "deny" => Ok(PlanPolicyDecision::Deny),
        value => Err(policy_error(format!(
            "`{name}` must be `allow`, `ask`, or `deny`, not `{value}`"
        ))),
    }
}

pub(super) fn access(block: &Block, name: &str) -> PluginManagerResult<PolicyFilesystemAccess> {
    match optional_string(block, name)?.unwrap_or("none") {
        "none" => Ok(PolicyFilesystemAccess::None),
        "read" => Ok(PolicyFilesystemAccess::Read),
        "read-write" => Ok(PolicyFilesystemAccess::ReadWrite),
        value => Err(policy_error(format!(
            "`{name}` must be `none`, `read`, or `read-write`, not `{value}`"
        ))),
    }
}

pub(super) fn required_access(
    block: &Block,
    name: &str,
) -> PluginManagerResult<PolicyFilesystemAccess> {
    let value = required_string(block, name)?;
    match value {
        "none" => Ok(PolicyFilesystemAccess::None),
        "read" => Ok(PolicyFilesystemAccess::Read),
        "read-write" => Ok(PolicyFilesystemAccess::ReadWrite),
        _ => Err(policy_error(format!(
            "`{name}` must be `none`, `read`, or `read-write`, not `{value}`"
        ))),
    }
}

pub(super) fn boolean(block: &Block, name: &str) -> PluginManagerResult<bool> {
    match block.attributes.get(name) {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(policy_error(format!("`{name}` must be a boolean"))),
    }
}

pub(super) fn unsigned(block: &Block, name: &str, maximum: u64) -> PluginManagerResult<u64> {
    let Some(value) = block.attributes.get(name) else {
        return Ok(0);
    };
    let Value::Number(value) = value else {
        return Err(policy_error(format!("`{name}` must be an integer")));
    };
    if !value.is_finite() || *value < 0.0 || value.fract() != 0.0 || *value > maximum as f64 {
        return Err(policy_error(format!(
            "`{name}` must be an integer between 0 and {maximum}"
        )));
    }
    Ok(*value as u64)
}

pub(super) fn unsigned_u32(block: &Block, name: &str, maximum: u32) -> PluginManagerResult<u32> {
    let value = unsigned(block, name, u64::from(maximum))?;
    u32::try_from(value).map_err(|_| policy_error(format!("`{name}` is too large")))
}

pub(super) fn required_string<'a>(block: &'a Block, name: &str) -> PluginManagerResult<&'a str> {
    optional_string(block, name)?.ok_or_else(|| policy_error(format!("`{name}` is required")))
}

pub(super) fn segment_list(block: &Block, name: &str) -> PluginManagerResult<Vec<String>> {
    string_list(block, name, valid_segment)
}

pub(super) fn machine_id_list(block: &Block, name: &str) -> PluginManagerResult<Vec<String>> {
    string_list(block, name, valid_machine_id)
}

pub(super) fn surface_list(
    block: &Block,
    name: &str,
) -> PluginManagerResult<Vec<PluginSurfaceKind>> {
    let values = raw_string_list(block, name)?;
    let mut output = values
        .into_iter()
        .map(|value| match value.as_str() {
            "flow" => Ok(PluginSurfaceKind::Flow),
            "mcp" => Ok(PluginSurfaceKind::Mcp),
            "okf" => Ok(PluginSurfaceKind::Okf),
            "skill" => Ok(PluginSurfaceKind::Skill),
            "tool" => Ok(PluginSurfaceKind::Tool),
            "ui" => Ok(PluginSurfaceKind::Ui),
            _ => Err(policy_error(format!(
                "`{name}` contains unsupported surface `{value}`"
            ))),
        })
        .collect::<PluginManagerResult<Vec<_>>>()?;
    output.sort();
    reject_duplicates(&output, name)?;
    Ok(output)
}

pub(super) fn http_method_list(block: &Block, name: &str) -> PluginManagerResult<Vec<HttpMethod>> {
    let values = raw_string_list(block, name)?;
    let mut output = values
        .into_iter()
        .map(|value| match value.as_str() {
            "delete" => Ok(HttpMethod::Delete),
            "get" => Ok(HttpMethod::Get),
            "patch" => Ok(HttpMethod::Patch),
            "post" => Ok(HttpMethod::Post),
            "put" => Ok(HttpMethod::Put),
            _ => Err(policy_error(format!(
                "`{name}` contains unsupported HTTP method `{value}`"
            ))),
        })
        .collect::<PluginManagerResult<Vec<_>>>()?;
    output.sort();
    reject_duplicates(&output, name)?;
    Ok(output)
}

pub(super) fn port_list(block: &Block, name: &str) -> PluginManagerResult<Vec<u16>> {
    let Some(value) = block.attributes.get(name) else {
        return Err(policy_error(format!("`{name}` is required")));
    };
    let Value::List(values) = value else {
        return Err(policy_error(format!("`{name}` must be an integer list")));
    };
    if values.len() > 16 {
        return Err(policy_error(format!("`{name}` exceeds 16 ports")));
    }
    let mut output = Vec::with_capacity(values.len());
    for value in values {
        let Value::Number(value) = value else {
            return Err(policy_error(format!("`{name}` must be an integer list")));
        };
        if !value.is_finite()
            || value.fract() != 0.0
            || *value < 1.0
            || *value > f64::from(u16::MAX)
        {
            return Err(policy_error(format!(
                "`{name}` ports must be integers between 1 and 65535"
            )));
        }
        output.push(*value as u16);
    }
    output.sort_unstable();
    reject_duplicates(&output, name)?;
    Ok(output)
}

pub(super) fn reject_unknown_attributes(
    block: &Block,
    allowed: &[&str],
    label: &str,
) -> PluginManagerResult<()> {
    let mut unknown = block
        .attributes
        .keys()
        .filter(|name| !allowed.contains(&name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    unknown.sort();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(policy_error(format!(
            "`{label}` contains unsupported attribute(s): {}",
            unknown.join(", ")
        )))
    }
}

pub(super) fn valid_portable_scope_path(value: &str) -> bool {
    value == "."
        || (!value.is_empty()
            && value.len() <= 1024
            && !value.starts_with('/')
            && !value.contains('\\')
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-')
            })
            && value
                .split('/')
                .all(|segment| !matches!(segment, "" | "." | "..")))
}

pub(super) fn valid_dns_name(value: &str) -> bool {
    if value.is_empty() || value.len() > 253 || value.ends_with('.') {
        return false;
    }
    if value.parse::<IpAddr>().is_ok() {
        return true;
    }
    value.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

fn optional_string<'a>(block: &'a Block, name: &str) -> PluginManagerResult<Option<&'a str>> {
    match block.attributes.get(name) {
        None => Ok(None),
        Some(Value::String(value)) if !value.is_empty() && value.trim() == value => Ok(Some(value)),
        Some(_) => Err(policy_error(format!(
            "`{name}` must be a non-empty trimmed string"
        ))),
    }
}

fn string_list(
    block: &Block,
    name: &str,
    validate: fn(&str) -> bool,
) -> PluginManagerResult<Vec<String>> {
    let Some(value) = block.attributes.get(name) else {
        return Ok(Vec::new());
    };
    let Value::List(values) = value else {
        return Err(policy_error(format!("`{name}` must be a string list")));
    };
    if values.len() > MAX_POLICY_RULES {
        return Err(policy_error(format!(
            "`{name}` exceeds {MAX_POLICY_RULES} entries"
        )));
    }
    let mut output = Vec::with_capacity(values.len());
    for value in values {
        let Value::String(value) = value else {
            return Err(policy_error(format!("`{name}` must be a string list")));
        };
        if !validate(value) {
            return Err(policy_error(format!(
                "`{name}` contains invalid value `{value}`"
            )));
        }
        output.push(value.clone());
    }
    output.sort();
    reject_duplicates(&output, name)?;
    Ok(output)
}

fn raw_string_list(block: &Block, name: &str) -> PluginManagerResult<Vec<String>> {
    let Some(value) = block.attributes.get(name) else {
        return Ok(Vec::new());
    };
    let Value::List(values) = value else {
        return Err(policy_error(format!("`{name}` must be a string list")));
    };
    if values.len() > MAX_POLICY_RULES {
        return Err(policy_error(format!(
            "`{name}` exceeds {MAX_POLICY_RULES} entries"
        )));
    }
    values
        .iter()
        .map(|value| match value {
            Value::String(value) if !value.is_empty() && value.trim() == value => Ok(value.clone()),
            _ => Err(policy_error(format!(
                "`{name}` must contain only non-empty trimmed strings"
            ))),
        })
        .collect()
}

fn reject_duplicates<T: Eq>(values: &[T], label: &str) -> PluginManagerResult<()> {
    if values.windows(2).any(|pair| pair[0] == pair[1]) {
        Err(policy_error(format!(
            "`{label}` contains a duplicate entry"
        )))
    } else {
        Ok(())
    }
}

fn valid_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && matches!(value.as_bytes().first(), Some(b'a'..=b'z'))
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_machine_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b':' | b'/' | b'@')
        })
}
