//! Memory bounds for structured tool payloads retained by the terminal UI.
//!
//! Tool output already has a byte ceiling. Authoritative arguments and
//! `ToolEnd.metadata` need the same guarantee because the transcript caches and
//! re-renders them after resize. Small payloads remain byte-for-byte identical;
//! oversized JSON is projected with bounded depth, fan-out, nodes, keys, and
//! strings so specialized renderers can still read ordinary semantic fields.

use std::io::{self, Write};

use serde_json::{Map, Value};

pub(crate) const MAX_STORED_TOOL_JSON_BYTES: usize = 1024 * 1024;
const MAX_JSON_DEPTH: usize = 12;
const MAX_JSON_NODES: usize = 1024;
const MAX_ARRAY_ITEMS: usize = 256;
const MAX_OBJECT_FIELDS: usize = 256;
const MAX_KEY_BYTES: usize = 128;
const MAX_STRING_BYTES: usize = 64 * 1024;
const TOTAL_STRING_BYTES: usize = 512 * 1024;
const TRUNCATED_STRING_SUFFIX: &str = "… [truncated by A3S Code]";

pub(crate) fn bounded_tool_args(value: Value) -> Value {
    bounded_json_payload(value, "_a3s_args_truncated")
}

pub(crate) fn bounded_tool_args_ref(value: &Value) -> Value {
    bounded_json_payload_ref(value, "_a3s_args_truncated")
}

pub(crate) fn bounded_tool_metadata(value: Option<Value>) -> Option<Value> {
    value.map(|value| bounded_json_payload(value, "_a3s_metadata_truncated"))
}

fn bounded_json_payload(value: Value, marker_key: &str) -> Value {
    let original_bytes = encoded_len(&value);
    if original_bytes <= MAX_STORED_TOOL_JSON_BYTES {
        return value;
    }

    project_oversized_payload(&value, marker_key, original_bytes)
}

fn bounded_json_payload_ref(value: &Value, marker_key: &str) -> Value {
    let original_bytes = encoded_len(value);
    if original_bytes <= MAX_STORED_TOOL_JSON_BYTES {
        return value.clone();
    }

    project_oversized_payload(value, marker_key, original_bytes)
}

fn project_oversized_payload(value: &Value, marker_key: &str, original_bytes: usize) -> Value {
    let mut budget = ProjectionBudget::default();
    let projected = project_value(value, 0, &mut budget);
    let projected = with_truncation_marker(projected, marker_key, original_bytes);
    if encoded_len(&projected) <= MAX_STORED_TOOL_JSON_BYTES {
        return projected;
    }

    serde_json::json!({
        marker_key: true,
        "_a3s_original_bytes": original_bytes,
        "_a3s_projection": "structured payload exceeded the terminal storage budget"
    })
}

#[derive(Debug)]
struct ProjectionBudget {
    remaining_nodes: usize,
    remaining_string_bytes: usize,
}

impl Default for ProjectionBudget {
    fn default() -> Self {
        Self {
            remaining_nodes: MAX_JSON_NODES,
            remaining_string_bytes: TOTAL_STRING_BYTES,
        }
    }
}

fn project_value(value: &Value, depth: usize, budget: &mut ProjectionBudget) -> Value {
    if budget.remaining_nodes == 0 {
        return Value::String("[JSON node budget exhausted]".to_string());
    }
    budget.remaining_nodes -= 1;

    match value {
        Value::String(value) => Value::String(project_string(value, budget)),
        Value::Array(values) => {
            if depth >= MAX_JSON_DEPTH {
                return Value::String("[nested array omitted]".to_string());
            }
            let original_len = values.len();
            let mut projected = Vec::with_capacity(original_len.min(MAX_ARRAY_ITEMS));
            for value in values.iter().take(MAX_ARRAY_ITEMS) {
                if budget.remaining_nodes == 0 {
                    break;
                }
                projected.push(project_value(value, depth + 1, budget));
            }
            let omitted = original_len.saturating_sub(projected.len());
            if omitted > 0 {
                projected.push(serde_json::json!({ "_a3s_omitted_items": omitted }));
            }
            Value::Array(projected)
        }
        Value::Object(values) => {
            if depth >= MAX_JSON_DEPTH {
                return Value::String("[nested object omitted]".to_string());
            }
            let original_len = values.len();
            let mut projected = Map::new();
            for (index, (key, value)) in values.iter().take(MAX_OBJECT_FIELDS).enumerate() {
                if budget.remaining_nodes == 0 {
                    break;
                }
                let key = project_key(key, index);
                projected.insert(key, project_value(value, depth + 1, budget));
            }
            let omitted = original_len.saturating_sub(projected.len());
            if omitted > 0 {
                projected.insert(
                    "_a3s_omitted_fields".to_string(),
                    Value::Number(omitted.into()),
                );
            }
            Value::Object(projected)
        }
        scalar => scalar.clone(),
    }
}

fn project_string(value: &str, budget: &mut ProjectionBudget) -> String {
    let allowed = MAX_STRING_BYTES.min(budget.remaining_string_bytes);
    if value.len() <= allowed {
        budget.remaining_string_bytes -= value.len();
        return value.to_string();
    }

    let content_budget = allowed.saturating_sub(TRUNCATED_STRING_SUFFIX.len());
    let end = utf8_prefix_len(value, content_budget);
    budget.remaining_string_bytes = budget
        .remaining_string_bytes
        .saturating_sub(end + TRUNCATED_STRING_SUFFIX.len());
    let mut projected = String::with_capacity(end + TRUNCATED_STRING_SUFFIX.len());
    projected.push_str(&value[..end]);
    projected.push_str(TRUNCATED_STRING_SUFFIX);
    projected
}

fn project_key(key: &str, index: usize) -> String {
    if key.len() <= MAX_KEY_BYTES {
        return key.to_string();
    }
    let suffix = format!("…#{index}");
    let content_budget = MAX_KEY_BYTES.saturating_sub(suffix.len());
    let end = utf8_prefix_len(key, content_budget);
    format!("{}{suffix}", &key[..end])
}

fn with_truncation_marker(value: Value, marker_key: &str, original_bytes: usize) -> Value {
    match value {
        Value::Object(mut object) => {
            object.insert(marker_key.to_string(), Value::Bool(true));
            object.insert(
                "_a3s_original_bytes".to_string(),
                Value::Number(original_bytes.into()),
            );
            Value::Object(object)
        }
        value => serde_json::json!({
            marker_key: true,
            "_a3s_original_bytes": original_bytes,
            "value": value,
        }),
    }
}

fn utf8_prefix_len(value: &str, max_bytes: usize) -> usize {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    end
}

fn encoded_len(value: &Value) -> usize {
    let mut counter = CountingWriter::default();
    serde_json::to_writer(&mut counter, value).map_or(usize::MAX, |()| counter.bytes)
}

#[derive(Default)]
struct CountingWriter {
    bytes: usize,
}

impl Write for CountingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.bytes = self.bytes.saturating_add(buffer.len());
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_payloads_are_preserved_exactly() {
        let value = serde_json::json!({
            "status": "complete",
            "items": [1, 2, 3],
            "nested": { "ok": true }
        });

        assert_eq!(bounded_tool_args(value.clone()), value);
        assert_eq!(bounded_tool_metadata(Some(value.clone())), Some(value));
    }

    #[test]
    fn oversized_metadata_keeps_semantic_fields_with_an_exact_memory_bound() {
        let value = serde_json::json!({
            "status": "complete",
            "dynamic_workflow": {
                "run_id": "research-42",
                "snapshot": {
                    "steps": {
                        "collect": {
                            "status": "completed",
                            "output": "证".repeat(MAX_STORED_TOOL_JSON_BYTES)
                        }
                    }
                }
            }
        });

        let projected = bounded_tool_metadata(Some(value)).expect("metadata");

        assert_eq!(projected["status"], "complete");
        assert_eq!(projected["dynamic_workflow"]["run_id"], "research-42");
        assert_eq!(
            projected["dynamic_workflow"]["snapshot"]["steps"]["collect"]["status"],
            "completed"
        );
        assert_eq!(projected["_a3s_metadata_truncated"], true);
        assert!(projected["_a3s_original_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes > MAX_STORED_TOOL_JSON_BYTES as u64));
        assert!(encoded_len(&projected) <= MAX_STORED_TOOL_JSON_BYTES);
        assert!(serde_json::to_string(&projected).is_ok());
    }

    #[test]
    fn oversized_argument_arrays_are_structurally_bounded() {
        let values = (0..10_000)
            .map(|index| {
                serde_json::json!({
                    "index": index,
                    "content": "x".repeat(1024)
                })
            })
            .collect::<Vec<_>>();
        let projected = bounded_tool_args(serde_json::json!({ "items": values }));

        assert_eq!(projected["_a3s_args_truncated"], true);
        assert!(projected["items"].as_array().is_some_and(|items| {
            items.len() <= MAX_ARRAY_ITEMS + 1
                && items.last().is_some_and(|item| {
                    item.get("_a3s_omitted_items")
                        .and_then(Value::as_u64)
                        .is_some()
                })
        }));
        assert!(encoded_len(&projected) <= MAX_STORED_TOOL_JSON_BYTES);
    }
}
