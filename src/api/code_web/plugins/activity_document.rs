use a3s_boot::BootResponse;

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

pub(super) fn response(content: &UseActivityContent) -> BootResponse {
    BootResponse::html(render(content))
        .with_header("cache-control", "no-store")
        .with_header("content-security-policy", CSP)
        .with_header("permissions-policy", PERMISSIONS_POLICY)
        .with_header("referrer-policy", "no-referrer")
        .with_header("x-content-type-options", "nosniff")
        .with_header("x-frame-options", "SAMEORIGIN")
        .with_header("cross-origin-resource-policy", "same-origin")
}

fn render(content: &UseActivityContent) -> String {
    let resource_bytes = content
        .styles
        .iter()
        .chain(content.scripts.iter())
        .map(String::len)
        .sum::<usize>();
    let mut document = String::with_capacity(content.html.len() + resource_bytes + 128);
    document.push_str(&content.html);
    for style in &content.styles {
        document.push_str("\n<style data-a3s-activity-resource=\"style\">\n");
        document.push_str(style);
        document.push_str("\n</style>");
    }
    for script in &content.scripts {
        document.push_str("\n<script data-a3s-activity-resource=\"script\">\n");
        document.push_str(script);
        document.push_str("\n</script>");
    }
    document
}

#[cfg(test)]
mod tests {
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
}
