use super::auth::SecretValue;
use super::client::{ensure_api_success, IlinkAuth, IlinkError, DEFAULT_API_TIMEOUT};
use super::transport::TencentIlinkTransport;
use super::types::{
    MessageItem, OutboundWeixinMessage, SendMessageRequest, SendMessageResponse, TextItem,
    MESSAGE_ITEM_TYPE_TEXT, MESSAGE_STATE_FINISH, MESSAGE_TYPE_BOT,
};

const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_CLIENT_ID_BYTES: usize = 128;
const MAX_RUN_ID_BYTES: usize = 128;

impl TencentIlinkTransport {
    pub(super) async fn send_text_request(
        &self,
        auth: &IlinkAuth,
        recipient: &SecretValue,
        context_token: Option<&SecretValue>,
        client_id: &str,
        run_id: Option<&str>,
        text: &str,
    ) -> Result<SendMessageResponse, IlinkError> {
        validate_bounded_text(text, "message text", MAX_TEXT_BYTES)?;
        validate_bounded_text(client_id, "client id", MAX_CLIENT_ID_BYTES)?;
        if let Some(run_id) = run_id {
            validate_bounded_text(run_id, "run id", MAX_RUN_ID_BYTES)?;
        }
        let request = self.authenticated_post(auth, "ilink/bot/sendmessage")?;
        let response: SendMessageResponse = self
            .post_json(
                request,
                &SendMessageRequest {
                    msg: OutboundWeixinMessage {
                        from_user_id: String::new(),
                        to_user_id: recipient.clone(),
                        client_id: client_id.to_string(),
                        message_type: MESSAGE_TYPE_BOT,
                        message_state: MESSAGE_STATE_FINISH,
                        item_list: vec![MessageItem {
                            item_type: Some(MESSAGE_ITEM_TYPE_TEXT),
                            text_item: Some(TextItem {
                                text: Some(text.to_string()),
                            }),
                            ..MessageItem::default()
                        }],
                        context_token: context_token.cloned(),
                        run_id: run_id.map(str::to_string),
                    },
                    base_info: self.identity.base_info(),
                },
                DEFAULT_API_TIMEOUT,
                "send_message",
            )
            .await?;
        ensure_api_success("send_message", response.ret, None)?;
        Ok(response)
    }
}

fn validate_bounded_text(
    value: &str,
    field: &'static str,
    max_bytes: usize,
) -> Result<(), IlinkError> {
    if value.is_empty() || value.len() > max_bytes || value.contains('\0') {
        return Err(IlinkError::InvalidConfiguration(field));
    }
    Ok(())
}
