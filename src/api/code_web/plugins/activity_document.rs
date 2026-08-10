use a3s_boot::BootResponse;

use a3s::components::CodePluginUiCandidateContent;

use crate::use_registry::UseActivityContent;

const CSP: &str = "sandbox allow-scripts; default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src data:; media-src 'none'; connect-src 'none'; worker-src 'none'; frame-src 'none'; child-src 'none'; object-src 'none'; manifest-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'self'; navigate-to 'none'";
const PERMISSIONS_POLICY: &str = "accelerometer=(), ambient-light-sensor=(), attribution-reporting=(), autoplay=(), battery=(), bluetooth=(), browsing-topics=(), camera=(), clipboard-read=(), clipboard-write=(), compute-pressure=(), display-capture=(), encrypted-media=(), fullscreen=(), gamepad=(), geolocation=(), gyroscope=(), hid=(), identity-credentials-get=(), idle-detection=(), local-fonts=(), magnetometer=(), microphone=(), midi=(), otp-credentials=(), payment=(), picture-in-picture=(), publickey-credentials-create=(), publickey-credentials-get=(), screen-wake-lock=(), serial=(), speaker-selection=(), storage-access=(), usb=(), web-share=(), window-management=(), xr-spatial-tracking=()";

pub(super) fn valid_registry_revision(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub(super) fn url(key: &str, generation: u64, revision: &str) -> String {
    let encoded_key = key.replace(':', "%3A");
    format!(
        "/api/v1/plugins/activities/{encoded_key}/document?generation={generation}&revision={revision}"
    )
}

pub(super) fn candidate_url(token: &str) -> String {
    format!("/api/v1/plugins/activities/candidates/{token}/document")
}

pub(super) fn response(content: &UseActivityContent) -> BootResponse {
    secured_response(render(
        &content.html,
        content.styles.iter().map(String::as_str),
        content.scripts.iter().map(String::as_str),
    ))
}

pub(super) fn candidate_response(content: &CodePluginUiCandidateContent) -> BootResponse {
    secured_response(render(
        &content.html,
        content.styles.iter().map(|value| value.as_ref()),
        content.scripts.iter().map(|value| value.as_ref()),
    ))
}

fn secured_response(document: String) -> BootResponse {
    BootResponse::html(document)
        .with_header("cache-control", "no-store")
        .with_header("content-security-policy", CSP)
        .with_header("permissions-policy", PERMISSIONS_POLICY)
        .with_header("referrer-policy", "no-referrer")
        .with_header("x-content-type-options", "nosniff")
        .with_header("x-frame-options", "SAMEORIGIN")
        .with_header("cross-origin-resource-policy", "same-origin")
}

fn render<'a>(
    html: &str,
    styles: impl Iterator<Item = &'a str>,
    scripts: impl Iterator<Item = &'a str>,
) -> String {
    let styles = styles.collect::<Vec<_>>();
    let scripts = scripts.collect::<Vec<_>>();
    let resource_bytes = styles
        .iter()
        .chain(scripts.iter())
        .map(|resource| resource.len())
        .sum::<usize>();
    let mut document = String::with_capacity(html.len() + resource_bytes + 128);
    document.push_str(html);
    for style in styles {
        document.push_str("\n<style data-a3s-activity-resource=\"style\">\n");
        document.push_str(style);
        document.push_str("\n</style>");
    }
    for script in scripts {
        document.push_str("\n<script data-a3s-activity-resource=\"script\">\n");
        document.push_str(script);
        document.push_str("\n</script>");
    }
    document
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn registry_revision_must_be_canonical_lowercase_sha256() {
        assert!(valid_registry_revision(&"a".repeat(64)));
        assert!(!valid_registry_revision(&"A".repeat(64)));
        assert!(!valid_registry_revision(&"g".repeat(64)));
        assert!(!valid_registry_revision(&"a".repeat(63)));
    }

    #[test]
    fn response_inlines_verified_assets_behind_the_sandbox_headers() {
        let content = UseActivityContent {
            key: "report:reports".to_string(),
            package_id: "use/acme/report".to_string(),
            skill: Some("report".to_string()),
            registry_revision: "1".repeat(64),
            sha256: "2".repeat(64),
            media_type: "text/html".to_string(),
            html: "<!doctype html><main>Report</main>".to_string(),
            styles: vec!["main { color: rebeccapurple; }".to_string()],
            scripts: vec!["document.querySelector('main').dataset.ready = 'true';".to_string()],
        };

        let response = response(&content);

        assert_eq!(response.status, 200);
        assert_eq!(
            response.headers.get("content-type").map(String::as_str),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(
            response.headers.get("cache-control").map(String::as_str),
            Some("no-store")
        );
        let csp = response
            .headers
            .get("content-security-policy")
            .expect("CSP header");
        assert!(csp.contains("sandbox allow-scripts"));
        assert!(csp.contains("connect-src 'none'"));
        assert!(!csp.contains("allow-same-origin"));
        let body = String::from_utf8(response.body).expect("UTF-8 document");
        assert!(body.contains("<main>Report</main>"));
        assert!(body.contains("main { color: rebeccapurple; }"));
        assert!(body.contains("document.querySelector('main')"));
        assert!(!body.contains("use/acme/report"));
    }

    #[test]
    fn candidate_response_uses_the_same_authority_free_document_boundary() {
        let response = candidate_response(&CodePluginUiCandidateContent {
            html: Arc::from("<!doctype html><main>Candidate</main>"),
            styles: vec![Arc::from("main { color: green; }")],
            scripts: vec![Arc::from(
                "port.postMessage({ protocol: 'a3s.activity.v3', type: 'activity.ready' });",
            )],
        });

        assert_eq!(response.status, 200);
        assert_eq!(
            response.headers.get("cache-control").map(String::as_str),
            Some("no-store")
        );
        assert_eq!(
            response
                .headers
                .get("cross-origin-resource-policy")
                .map(String::as_str),
            Some("same-origin")
        );
        assert_eq!(
            response.headers.get("referrer-policy").map(String::as_str),
            Some("no-referrer")
        );
        assert_eq!(
            response
                .headers
                .get("x-content-type-options")
                .map(String::as_str),
            Some("nosniff")
        );
        let csp = response
            .headers
            .get("content-security-policy")
            .expect("candidate CSP header");
        assert!(csp.contains("sandbox allow-scripts"));
        assert!(csp.contains("connect-src 'none'"));
        assert!(csp.contains("form-action 'none'"));
        assert!(csp.contains("navigate-to 'none'"));
        assert!(!csp.contains("allow-same-origin"));
    }
}
