use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{OriginalUri, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Redirect;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::auth::{pack_client_version, SecretValue};
use super::client::{IlinkAuth, IlinkClientIdentity, IlinkError};
use super::transport::{IlinkLoginTransport, IlinkMessagingTransport, TencentIlinkTransport};
use super::types::{GetUpdatesResponse, PollQrResponse, QrCodeStatus};
use super::updates::validate_updates_response;
use super::url_policy::IlinkHostPolicy;

#[derive(Debug)]
struct CapturedRequest {
    operation: &'static str,
    headers: HeaderMap,
    query: HashMap<String, String>,
    body: Option<Value>,
}

#[derive(Clone)]
struct CaptureState {
    sender: mpsc::UnboundedSender<CapturedRequest>,
}

struct MockServer {
    origin: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn spawn_mock_server(app: Router) -> MockServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock iLink server");
    let origin = format!("http://{}/", listener.local_addr().expect("mock address"));
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve mock iLink requests");
    });
    MockServer { origin, task }
}

fn test_transport(origin: &str) -> TencentIlinkTransport {
    let identity = IlinkClientIdentity::new("a3s-test-app", "a3s", "1.0.11", "A3S/0.9.7")
        .expect("valid test identity");
    let policy = IlinkHostPolicy::for_test_origin(origin).expect("loopback test origin");
    TencentIlinkTransport::new(identity, policy, origin).expect("test transport")
}

fn test_auth(transport: &TencentIlinkTransport, origin: &str) -> IlinkAuth {
    IlinkAuth {
        base_url: transport
            .validate_account_base_url(origin)
            .expect("validated account base URL"),
        bot_token: SecretValue::new("bot-token-canary").expect("bot token"),
    }
}

async fn next_capture(receiver: &mut mpsc::UnboundedReceiver<CapturedRequest>) -> CapturedRequest {
    tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("mock request timeout")
        .expect("mock request channel closed")
}

fn assert_application_headers(headers: &HeaderMap) {
    assert_eq!(
        headers
            .get("ilink-app-id")
            .and_then(|value| value.to_str().ok()),
        Some("a3s-test-app")
    );
    assert_eq!(
        headers
            .get("ilink-app-clientversion")
            .and_then(|value| value.to_str().ok()),
        Some("65547")
    );
}

fn assert_post_headers(headers: &HeaderMap, authenticated: bool) {
    assert_application_headers(headers);
    assert_eq!(
        headers
            .get("authorizationtype")
            .and_then(|value| value.to_str().ok()),
        Some("ilink_bot_token")
    );
    assert_eq!(
        headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let encoded_uin = headers
        .get("x-wechat-uin")
        .and_then(|value| value.to_str().ok())
        .expect("random Weixin UIN header");
    let decoded_uin = base64::engine::general_purpose::STANDARD
        .decode(encoded_uin)
        .expect("base64 Weixin UIN");
    std::str::from_utf8(&decoded_uin)
        .expect("UTF-8 Weixin UIN")
        .parse::<u32>()
        .expect("decimal uint32 Weixin UIN");
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    if authenticated {
        assert_eq!(authorization, Some("Bearer bot-token-canary"));
    } else {
        assert_eq!(authorization, None);
    }
}

async fn capture_create_qr(
    State(state): State<CaptureState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Json(body): Json<Value>,
) -> Json<Value> {
    state
        .sender
        .send(CapturedRequest {
            operation: "create_qr",
            headers,
            query,
            body: Some(body),
        })
        .expect("capture create QR request");
    Json(json!({
        "qrcode": "qr-canary",
        "qrcode_img_content": "data:image/png;base64,cXItY2FuYXJ5"
    }))
}

async fn capture_poll_qr(
    State(state): State<CaptureState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Value> {
    state
        .sender
        .send(CapturedRequest {
            operation: "poll_qr",
            headers,
            query,
            body: None,
        })
        .expect("capture poll QR request");
    Json(json!({ "status": "wait" }))
}

async fn capture_get_updates(
    State(state): State<CaptureState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    state
        .sender
        .send(CapturedRequest {
            operation: "get_updates",
            headers,
            query: HashMap::new(),
            body: Some(body),
        })
        .expect("capture get updates request");
    Json(json!({
        "ret": 0,
        "msgs": [],
        "get_updates_buf": "next-cursor-canary",
        "longpolling_timeout_ms": 35_000
    }))
}

async fn capture_send_message(
    State(state): State<CaptureState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    state
        .sender
        .send(CapturedRequest {
            operation: "send_message",
            headers,
            query: HashMap::new(),
            body: Some(body),
        })
        .expect("capture send message request");
    Json(json!({ "ret": 0 }))
}

async fn capture_control_request(
    State(state): State<CaptureState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    let operation = match uri.path() {
        "/ilink/bot/getconfig" => "get_config",
        "/ilink/bot/sendtyping" => "send_typing",
        "/ilink/bot/msg/notifystart" => "notify_start",
        "/ilink/bot/msg/notifystop" => "notify_stop",
        path => panic!("unexpected control path: {path}"),
    };
    state
        .sender
        .send(CapturedRequest {
            operation,
            headers,
            query: HashMap::new(),
            body: Some(body),
        })
        .expect("capture control request");
    Json(json!({ "ret": 0, "typing_ticket": "typing-ticket-canary" }))
}

#[test]
fn weixin_ilink_contract_packs_client_versions() {
    assert_eq!(pack_client_version("1.0.11").unwrap(), 0x0001_000b);
    assert_eq!(pack_client_version("2.4.6").unwrap(), 0x0002_0406);
    assert!(pack_client_version("1.2").is_err());
    assert!(pack_client_version("1.256.0").is_err());
    assert!(pack_client_version("not-a-version").is_err());
}

#[test]
fn weixin_ilink_contract_redacts_secret_debug_output() {
    let secret = SecretValue::new("canary-bot-token").unwrap();

    let rendered = format!("{secret:?}");

    assert_eq!(rendered, "SecretValue([REDACTED])");
    assert!(!rendered.contains("canary-bot-token"));
    assert_eq!(secret.expose(), "canary-bot-token");
}

#[test]
fn weixin_ilink_contract_accepts_known_qr_states_and_contains_unknown_states() {
    let redirected: PollQrResponse = serde_json::from_value(serde_json::json!({
        "status": "scaned_but_redirect",
        "redirect_host": "https://ilinkai.weixin.qq.com"
    }))
    .unwrap();
    assert_eq!(redirected.status, QrCodeStatus::ScanedButRedirect);

    let unknown: PollQrResponse = serde_json::from_value(serde_json::json!({
        "status": "future_state"
    }))
    .unwrap();
    assert_eq!(unknown.status, QrCodeStatus::Unknown);
}

#[test]
fn weixin_ilink_contract_deserializes_text_updates_without_exposing_tokens_in_debug() {
    let response: GetUpdatesResponse = serde_json::from_value(serde_json::json!({
        "ret": 0,
        "msgs": [{
            "seq": 7,
            "message_id": 42,
            "from_user_id": "owner-canary",
            "message_type": 1,
            "item_list": [{
                "type": 1,
                "text_item": { "text": "进度" }
            }],
            "context_token": "context-canary"
        }],
        "get_updates_buf": "cursor-canary",
        "longpolling_timeout_ms": 35000
    }))
    .unwrap();

    assert_eq!(response.messages.len(), 1);
    assert_eq!(response.messages[0].message_id, Some(42));
    assert_eq!(response.messages[0].text(), Some("进度"));
    assert_eq!(response.long_polling_timeout_ms, Some(35_000));
    let rendered = format!("{response:?}");
    assert!(!rendered.contains("owner-canary"));
    assert!(!rendered.contains("context-canary"));
    assert!(!rendered.contains("cursor-canary"));
}

#[test]
fn weixin_ilink_contract_enforces_production_base_url_policy() {
    let policy = IlinkHostPolicy::production(["ilinkai.weixin.qq.com"]).unwrap();

    assert!(policy.validate("https://ilinkai.weixin.qq.com").is_ok());
    assert!(policy
        .validate("https://ilinkai.weixin.qq.com:443/region/")
        .is_ok());
    for rejected in [
        "http://ilinkai.weixin.qq.com",
        "https://127.0.0.1",
        "https://user@ilinkai.weixin.qq.com",
        "https://ilinkai.weixin.qq.com:8443",
        "https://ilinkai.weixin.qq.com.attacker.example",
        "https://attacker.example/?next=ilinkai.weixin.qq.com",
        "https://ilinkai.weixin.qq.com/#fragment",
    ] {
        assert!(
            policy.validate(rejected).is_err(),
            "unexpectedly accepted {rejected}"
        );
    }
    assert!(policy
        .validate_redirect_host("ilinkai.weixin.qq.com")
        .is_ok());
    for rejected in [
        "https://ilinkai.weixin.qq.com",
        "user@ilinkai.weixin.qq.com",
        "ilinkai.weixin.qq.com:8443",
        "ilinkai.weixin.qq.com.attacker.example",
        "127.0.0.1",
    ] {
        assert!(
            policy.validate_redirect_host(rejected).is_err(),
            "unexpectedly accepted redirect host {rejected}"
        );
    }
}

#[test]
fn weixin_ilink_contract_bounds_update_arrays_text_and_server_timeout() {
    let too_many_messages: GetUpdatesResponse = serde_json::from_value(json!({
        "ret": 0,
        "msgs": (0..257).map(|index| json!({ "message_id": index })).collect::<Vec<_>>(),
        "get_updates_buf": "cursor-canary"
    }))
    .unwrap();
    assert_eq!(
        validate_updates_response(&too_many_messages),
        Err(IlinkError::InvalidResponse("get_updates"))
    );

    let too_many_items: GetUpdatesResponse = serde_json::from_value(json!({
        "ret": 0,
        "msgs": [{
            "message_id": 1,
            "item_list": (0..33).map(|_| json!({ "type": 0 })).collect::<Vec<_>>()
        }],
        "get_updates_buf": "cursor-canary"
    }))
    .unwrap();
    assert_eq!(
        validate_updates_response(&too_many_items),
        Err(IlinkError::InvalidResponse("get_updates"))
    );

    let oversized_text: GetUpdatesResponse = serde_json::from_value(json!({
        "ret": 0,
        "msgs": [{
            "message_id": 1,
            "item_list": [{
                "type": 1,
                "text_item": { "text": "x".repeat(16 * 1024 + 1) }
            }]
        }],
        "get_updates_buf": "cursor-canary"
    }))
    .unwrap();
    assert_eq!(
        validate_updates_response(&oversized_text),
        Err(IlinkError::InvalidResponse("get_updates"))
    );

    let unbounded_timeout: GetUpdatesResponse = serde_json::from_value(json!({
        "ret": 0,
        "msgs": [],
        "get_updates_buf": "cursor-canary",
        "longpolling_timeout_ms": 60_001
    }))
    .unwrap();
    assert_eq!(
        validate_updates_response(&unbounded_timeout),
        Err(IlinkError::InvalidResponse("get_updates"))
    );
}

#[tokio::test]
async fn weixin_ilink_contract_sends_qr_update_and_text_requests() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let app = Router::new()
        .route("/ilink/bot/get_bot_qrcode", post(capture_create_qr))
        .route("/ilink/bot/get_qrcode_status", get(capture_poll_qr))
        .route("/ilink/bot/getupdates", post(capture_get_updates))
        .route("/ilink/bot/sendmessage", post(capture_send_message))
        .route("/ilink/bot/getconfig", post(capture_control_request))
        .route("/ilink/bot/sendtyping", post(capture_control_request))
        .route("/ilink/bot/msg/notifystart", post(capture_control_request))
        .route("/ilink/bot/msg/notifystop", post(capture_control_request))
        .with_state(CaptureState { sender });
    let server = spawn_mock_server(app).await;
    let transport = test_transport(&server.origin);
    let auth = test_auth(&transport, &server.origin);

    let created = transport.create_qr().await.expect("create QR response");
    assert_eq!(created.qrcode.expose(), "qr-canary");
    let qr_base_url = transport
        .validate_account_base_url(&server.origin)
        .expect("QR polling base URL");
    let qr_code = SecretValue::new("qr-canary").expect("QR code");
    let verify_code = SecretValue::new("123456").expect("verification code");
    let polled = transport
        .poll_qr(&qr_base_url, &qr_code, Some(&verify_code))
        .await
        .expect("poll QR response");
    assert_eq!(polled.status, QrCodeStatus::Wait);
    let updates = transport
        .get_updates(&auth, "cursor-canary", Duration::from_secs(35))
        .await
        .expect("get updates response");
    assert_eq!(
        updates.update_cursor.as_ref().map(SecretValue::expose),
        Some("next-cursor-canary")
    );
    let recipient = SecretValue::new("owner-canary").expect("owner ID");
    let context_token = SecretValue::new("context-canary").expect("context token");
    transport
        .send_text(
            &auth,
            &recipient,
            Some(&context_token),
            "client-id-canary",
            Some("run-id-canary"),
            "任务仍在执行",
        )
        .await
        .expect("send message response");
    let config = transport
        .get_config(&auth, Some(&recipient), Some(&context_token))
        .await
        .expect("get config response");
    assert_eq!(
        config.typing_ticket.as_ref().map(SecretValue::expose),
        Some("typing-ticket-canary")
    );
    let typing_ticket = SecretValue::new("typing-ticket-canary").expect("typing ticket");
    transport
        .send_typing(&auth, &recipient, &typing_ticket, 1)
        .await
        .expect("send typing response");
    transport
        .notify_start(&auth)
        .await
        .expect("notify start response");
    transport
        .notify_stop(&auth)
        .await
        .expect("notify stop response");

    let create = next_capture(&mut receiver).await;
    assert_eq!(create.operation, "create_qr");
    assert_post_headers(&create.headers, false);
    assert_eq!(
        create.query.get("bot_type").map(String::as_str),
        Some("a3s")
    );
    assert_eq!(create.body, Some(json!({ "local_token_list": [] })));

    let poll = next_capture(&mut receiver).await;
    assert_eq!(poll.operation, "poll_qr");
    assert_application_headers(&poll.headers);
    for absent in [
        "authorizationtype",
        "authorization",
        "content-type",
        "x-wechat-uin",
    ] {
        assert!(!poll.headers.contains_key(absent));
    }
    assert_eq!(
        poll.query.get("qrcode").map(String::as_str),
        Some("qr-canary")
    );
    assert_eq!(
        poll.query.get("verify_code").map(String::as_str),
        Some("123456")
    );
    assert_eq!(poll.body, None);

    let get_updates = next_capture(&mut receiver).await;
    assert_eq!(get_updates.operation, "get_updates");
    assert_post_headers(&get_updates.headers, true);
    assert_eq!(
        get_updates.body,
        Some(json!({
            "get_updates_buf": "cursor-canary",
            "base_info": {
                "channel_version": "1.0.11",
                "bot_agent": "A3S/0.9.7"
            }
        }))
    );

    let send_message = next_capture(&mut receiver).await;
    assert_eq!(send_message.operation, "send_message");
    assert_post_headers(&send_message.headers, true);
    assert_eq!(
        send_message.body,
        Some(json!({
            "msg": {
                "from_user_id": "",
                "to_user_id": "owner-canary",
                "client_id": "client-id-canary",
                "message_type": 2,
                "message_state": 2,
                "item_list": [{
                    "type": 1,
                    "text_item": { "text": "任务仍在执行" }
                }],
                "context_token": "context-canary",
                "run_id": "run-id-canary"
            },
            "base_info": {
                "channel_version": "1.0.11",
                "bot_agent": "A3S/0.9.7"
            }
        }))
    );

    let get_config = next_capture(&mut receiver).await;
    assert_eq!(get_config.operation, "get_config");
    assert_post_headers(&get_config.headers, true);
    assert_eq!(
        get_config.body,
        Some(json!({
            "base_info": {
                "channel_version": "1.0.11",
                "bot_agent": "A3S/0.9.7"
            },
            "ilink_user_id": "owner-canary",
            "context_token": "context-canary"
        }))
    );

    let send_typing = next_capture(&mut receiver).await;
    assert_eq!(send_typing.operation, "send_typing");
    assert_post_headers(&send_typing.headers, true);
    assert_eq!(
        send_typing.body,
        Some(json!({
            "ilink_user_id": "owner-canary",
            "typing_ticket": "typing-ticket-canary",
            "status": 1,
            "base_info": {
                "channel_version": "1.0.11",
                "bot_agent": "A3S/0.9.7"
            }
        }))
    );

    for operation in ["notify_start", "notify_stop"] {
        let notify = next_capture(&mut receiver).await;
        assert_eq!(notify.operation, operation);
        assert_post_headers(&notify.headers, true);
        assert_eq!(
            notify.body,
            Some(json!({
                "base_info": {
                    "channel_version": "1.0.11",
                    "bot_agent": "A3S/0.9.7"
                }
            }))
        );
    }
}

#[tokio::test]
async fn weixin_ilink_contract_fails_closed_on_unknown_qr_state() {
    let app = Router::new().route(
        "/ilink/bot/get_qrcode_status",
        get(|| async { Json(json!({ "status": "future_state" })) }),
    );
    let server = spawn_mock_server(app).await;
    let transport = test_transport(&server.origin);
    let qr_base_url = transport
        .validate_account_base_url(&server.origin)
        .expect("QR base URL");
    let qr_code = SecretValue::new("qr-canary").expect("QR code");

    let error = transport
        .poll_qr(&qr_base_url, &qr_code, None)
        .await
        .expect_err("unknown QR state must fail closed");

    assert_eq!(error, IlinkError::InvalidResponse("poll_qr"));
}

#[tokio::test]
async fn weixin_ilink_contract_maps_stale_credentials() {
    let app = Router::new().route(
        "/ilink/bot/getupdates",
        post(|| async { Json(json!({ "errcode": -14, "msgs": [] })) }),
    );
    let server = spawn_mock_server(app).await;
    let transport = test_transport(&server.origin);
    let auth = test_auth(&transport, &server.origin);

    let error = transport
        .get_updates(&auth, "cursor-canary", Duration::from_secs(35))
        .await
        .expect_err("stale credential must fail closed");

    assert_eq!(error, IlinkError::StaleCredential);
}

#[tokio::test]
async fn weixin_ilink_contract_rejects_redirect_status_oversize_and_timeout() {
    let app = Router::new()
        .route(
            "/redirect",
            get(|| async { Redirect::temporary("/target") }),
        )
        .route("/status", get(|| async { StatusCode::SERVICE_UNAVAILABLE }))
        .route(
            "/large",
            get(|| async { Json(json!({ "payload": "too large" })) }),
        )
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Json(json!({}))
            }),
        );
    let server = spawn_mock_server(app).await;
    let mut transport = test_transport(&server.origin);

    let redirect_url = transport
        .qr_base_url
        .join("redirect")
        .expect("redirect URL");
    let redirect_error = transport
        .get_json::<Value>(
            transport.http.get(redirect_url),
            Duration::from_secs(1),
            "redirect",
        )
        .await
        .expect_err("redirect must not be followed");
    assert_eq!(redirect_error, IlinkError::HttpStatus(307));

    let status_url = transport.qr_base_url.join("status").expect("status URL");
    let status_error = transport
        .get_json::<Value>(
            transport.http.get(status_url),
            Duration::from_secs(1),
            "status",
        )
        .await
        .expect_err("non-success status must fail");
    assert_eq!(status_error, IlinkError::HttpStatus(503));

    transport.max_response_bytes = 8;
    let large_url = transport.qr_base_url.join("large").expect("large URL");
    let large_error = transport
        .get_json::<Value>(
            transport.http.get(large_url),
            Duration::from_secs(1),
            "large",
        )
        .await
        .expect_err("oversized response must fail");
    assert_eq!(large_error, IlinkError::ResponseTooLarge);

    let slow_url = transport.qr_base_url.join("slow").expect("slow URL");
    let timeout_error = transport
        .get_json::<Value>(
            transport.http.get(slow_url),
            Duration::from_millis(10),
            "slow",
        )
        .await
        .expect_err("slow response must time out");
    assert_eq!(timeout_error, IlinkError::Timeout);
}
