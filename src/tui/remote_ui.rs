//! RemoteUI: surface the `view` (a sized embed widget) that OS's progressive
//! API returns for a task.
//!
//! A `view` is a partial, chrome-less console page meant for a *sized popup*
//! rather than a full browser tab. We can't embed a WebView in the terminal, so
//! we spawn the sibling `a3s-webview` helper — a native window that seeds the OS
//! token into localStorage (from `A3S_OS_TOKEN`, which the TUI exports) and loads
//! the page authenticated. Plain links still go to the user's browser.

use std::process::Command;

/// A `viewUrl` (+ optional size / embeddable hint) extracted from a tool result.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ViewSpec {
    pub url: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The API explicitly marked this view as a sized popup (or returned a size).
    pub embeddable: bool,
}

/// Find a renderable view in a tool's JSON output. Prefers the current `view`
/// object `{ url, width, height }`; falls back to a legacy top-level `viewUrl`
/// (+ optional `viewSize` / `embeddable`). The capabilities API nests it under
/// `data` too, so we walk recursively and take the first match.
///
/// The progressive API returns a RELATIVE `url` (`/admin/…?embed=1`) by the OS's
/// "store relative, complete at the edge" convention — the TUI IS the edge, so we
/// absolutize it against `origin` (the signed-in OS origin). Without this every
/// capabilities view is silently dropped, since the webview needs an absolute URL.
pub(crate) fn find_view_url(output: &str, origin: Option<&str>) -> Option<ViewSpec> {
    // Tool stdout is usually one JSON doc, but a bash block may emit several
    // (e.g. a `list` then an `execute`). Scan every parseable JSON value and take
    // the LAST that carries a view — the freshest result the user just ran. This
    // also tolerates concatenated docs that a single `from_str` would reject.
    serde_json::Deserializer::from_str(output)
        .into_iter::<serde_json::Value>()
        .flatten()
        .filter_map(|v| find_in(&v, origin))
        .last()
}

/// Accept an absolute `http(s)://` url as-is, or complete a root-relative
/// `/path` against `origin`. Anything else (relative with no origin, `mailto:`,
/// …) yields `None` so we never hand the webview a URL it can't open.
fn absolutize(url: &str, origin: Option<&str>) -> Option<String> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(url.to_string())
    } else if url.starts_with('/') {
        origin.map(|o| format!("{}{}", o.trim_end_matches('/'), url))
    } else {
        None
    }
}

fn find_in(value: &serde_json::Value, origin: Option<&str>) -> Option<ViewSpec> {
    match value {
        serde_json::Value::Object(obj) => {
            // Current OS shape: a `view` object `{ url, width, height }` — a
            // focused, chrome-less embed widget at a suggested size.
            if let Some(spec) = obj.get("view").and_then(|v| parse_view_object(v, origin)) {
                return Some(spec);
            }
            // Back-compat: a bare top-level `viewUrl` (+ optional `viewSize` /
            // `embeddable`), the shape the API returned before the `view` object.
            if let Some(spec) = parse_legacy_view_url(obj, origin) {
                return Some(spec);
            }
            obj.values().find_map(|v| find_in(v, origin))
        }
        serde_json::Value::Array(arr) => arr.iter().find_map(|v| find_in(v, origin)),
        _ => None,
    }
}

/// Read a JSON number (int or float) as a pixel dimension.
fn px(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<u32> {
    obj.get(key)
        .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f.round() as u64)))
        .map(|n| n as u32)
}

/// Parse the current `view` object `{ url, width, height }`. The API only emits
/// it for sized popups, so a parsed `view` is always embeddable.
fn parse_view_object(v: &serde_json::Value, origin: Option<&str>) -> Option<ViewSpec> {
    let obj = v.as_object()?;
    let url = obj.get("url").and_then(|u| u.as_str())?;
    Some(ViewSpec {
        url: absolutize(url, origin)?,
        width: px(obj, "width"),
        height: px(obj, "height"),
        embeddable: true,
    })
}

/// Back-compat: the older top-level `viewUrl` string with an optional `viewSize`
/// `{width,height}` sibling and `embeddable` flag.
fn parse_legacy_view_url(
    obj: &serde_json::Map<String, serde_json::Value>,
    origin: Option<&str>,
) -> Option<ViewSpec> {
    let url = obj.get("viewUrl").and_then(|u| u.as_str())?;
    let url = absolutize(url, origin)?;
    let size = obj.get("viewSize").and_then(|s| s.as_object());
    let width = size.and_then(|s| px(s, "width"));
    let height = size.and_then(|s| px(s, "height"));
    let embeddable = obj
        .get("embeddable")
        .and_then(|e| e.as_bool())
        .unwrap_or(false)
        || width.is_some();
    Some(ViewSpec {
        url,
        width,
        height,
        embeddable,
    })
}

/// Locate the `a3s-webview` binary: prefer a sibling of the running `a3s`
/// executable (how it ships), else fall back to the bare name on `PATH`.
fn webview_bin() -> std::path::PathBuf {
    let name = if cfg!(windows) {
        "a3s-webview.exe"
    } else {
        "a3s-webview"
    };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(sibling) = exe.parent().map(|d| d.join(name)) {
            if sibling.exists() {
                return sibling;
            }
        }
    }
    std::path::PathBuf::from(name)
}

/// Build the `a3s-webview` argv for a view (url + optional size). Split out from
/// spawning so the spec→argv mapping is unit-testable.
fn webview_args(spec: &ViewSpec) -> Vec<String> {
    let mut args = vec!["--url".to_string(), spec.url.clone()];
    if let Some(w) = spec.width {
        args.push("--width".to_string());
        args.push(w.to_string());
    }
    if let Some(h) = spec.height {
        args.push("--height".to_string());
        args.push(h.to_string());
    }
    args
}

/// Open a view's url in the native `a3s-webview` window (detached). Inherits the
/// process env so the helper reads `A3S_OS_TOKEN` for auth. Returns Err if the
/// helper binary isn't present/launchable (caller surfaces a hint).
pub(crate) fn open_window(spec: &ViewSpec) -> std::io::Result<()> {
    Command::new(webview_bin())
        .args(webview_args(spec))
        .spawn()
        .map(|_child| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_top_level_view_url() {
        let out = r#"{"success":true,"viewUrl":"https://os.x/p","data":{"items":[]}}"#;
        let s = find_view_url(out, None).unwrap();
        assert_eq!(s.url, "https://os.x/p");
        assert!(!s.embeddable); // no size / flag
    }

    #[test]
    fn finds_nested_view_url_with_size_marks_embeddable() {
        let out =
            r#"{"data":{"viewUrl":"https://os.x/embed","viewSize":{"width":720,"height":520}}}"#;
        let s = find_view_url(out, None).unwrap();
        assert_eq!((s.width, s.height), (Some(720), Some(520)));
        assert!(s.embeddable); // size present ⇒ embeddable
    }

    #[test]
    fn embeddable_flag_without_size() {
        let out = r#"{"viewUrl":"https://os.x/p","embeddable":true}"#;
        assert!(find_view_url(out, None).unwrap().embeddable);
    }

    #[test]
    fn finds_view_object_marks_embeddable() {
        let out = r#"{"success":true,"view":{"url":"https://os.x/p?embed=1","width":720,"height":520},"modules":[]}"#;
        let s = find_view_url(out, None).unwrap();
        assert_eq!(s.url, "https://os.x/p?embed=1");
        assert_eq!((s.width, s.height), (Some(720), Some(520)));
        assert!(s.embeddable); // a `view` object is always a sized popup
    }

    #[test]
    fn finds_nested_view_object() {
        let out = r#"{"data":{"view":{"url":"https://os.x/embed","width":400,"height":300}}}"#;
        assert_eq!(find_view_url(out, None).unwrap().width, Some(400));
    }

    #[test]
    fn view_object_takes_precedence_over_legacy_url() {
        let out = r#"{"viewUrl":"https://old/x","view":{"url":"https://new/y","width":300,"height":200}}"#;
        assert_eq!(find_view_url(out, None).unwrap().url, "https://new/y");
    }

    #[test]
    fn relative_view_url_is_absolutized_against_origin() {
        // The OS progressive API returns a RELATIVE url; the TUI (the edge)
        // completes it. This is the common real-world shape.
        let out = r#"{"success":true,"view":{"url":"/admin/kernel/assets?embed=1","width":1440,"height":900}}"#;
        let s = find_view_url(out, Some("https://os.example.com/")).unwrap();
        assert_eq!(s.url, "https://os.example.com/admin/kernel/assets?embed=1"); // trailing / trimmed
        assert!(s.embeddable);
    }

    #[test]
    fn last_view_wins_across_concatenated_json_docs() {
        // A bash block that ran `list` then `execute` emits two JSON docs; the
        // freshest (execute, with the view) must win.
        let out = r#"{"success":true,"modules":[]}
{"success":true,"view":{"url":"/admin/assets/a1?embed=1","width":1024,"height":768}}"#;
        let s = find_view_url(out, Some("https://os.x")).unwrap();
        assert_eq!(s.url, "https://os.x/admin/assets/a1?embed=1");
    }

    #[test]
    fn relative_view_url_without_origin_is_dropped() {
        // No signed-in origin ⇒ we can't complete it; better none than a broken url.
        let out = r#"{"view":{"url":"/admin/kernel/assets?embed=1","width":10,"height":10}}"#;
        assert!(find_view_url(out, None).is_none());
    }

    #[test]
    fn ignores_non_http_and_absent() {
        assert!(find_view_url(r#"{"viewUrl":"file:///x"}"#, None).is_none());
        assert!(find_view_url(
            r#"{"view":{"url":"file:///x","width":10,"height":10}}"#,
            None
        )
        .is_none());
        assert!(find_view_url(r#"{"data":{"items":[1,2]}}"#, None).is_none());
        assert!(find_view_url("not json", None).is_none());
    }

    #[test]
    fn webview_args_pass_url_and_size() {
        let spec = ViewSpec {
            url: "https://os.x/p?embed=1".into(),
            width: Some(720),
            height: Some(520),
            embeddable: true,
        };
        assert_eq!(
            webview_args(&spec),
            vec![
                "--url",
                "https://os.x/p?embed=1",
                "--width",
                "720",
                "--height",
                "520"
            ]
        );
        let no_size = ViewSpec {
            url: "https://os.x/p".into(),
            width: None,
            height: None,
            embeddable: false,
        };
        assert_eq!(webview_args(&no_size), vec!["--url", "https://os.x/p"]);
    }

    /// End-to-end: a progressive-API `execute` response carrying a `view` object
    /// parses into a ViewSpec whose url + size reach the a3s-webview argv — i.e.
    /// the view's url is what gets opened in the webview.
    #[test]
    fn progressive_api_view_flows_to_webview_args() {
        let resp = r#"{"success":true,
            "view":{"url":"/admin/kernel/assets?embed=1","width":900,"height":680},
            "data":{"items":[]}}"#;
        let spec =
            find_view_url(resp, Some("https://os.example.com")).expect("view object should parse");
        assert!(spec.embeddable); // a `view` is always a sized popup → auto-opens
        let args = webview_args(&spec);
        assert_eq!(args[0], "--url");
        assert_eq!(
            args[1],
            "https://os.example.com/admin/kernel/assets?embed=1"
        );
        assert!(args.contains(&"900".to_string()) && args.contains(&"680".to_string()));
    }
}
