use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::redirect::Policy;
use serde::de::DeserializeOwned;

use super::auth::SecretValue;
use super::client::{
    IlinkAuth, IlinkClientIdentity, IlinkError, MAX_LONG_POLL_TIMEOUT, MAX_RESPONSE_BYTES,
};
use super::types::{
    CreateQrResponse, GetUpdatesResponse, NotifyResponse, PollQrResponse, SendMessageResponse,
};
#[cfg(test)]
use super::types::{GetConfigResponse, SendTypingResponse};
use super::url_policy::{IlinkHostPolicy, ValidatedBaseUrl};

#[async_trait]
pub(in crate::api::code_web::weixin) trait IlinkLoginTransport:
    Send + Sync
{
    fn qr_base_url(&self) -> ValidatedBaseUrl;

    fn validate_account_base_url(&self, base_url: &str) -> Result<ValidatedBaseUrl, IlinkError>;

    fn validate_redirect_host(&self, redirect_host: &str) -> Result<ValidatedBaseUrl, IlinkError>;

    async fn create_qr(&self) -> Result<CreateQrResponse, IlinkError>;

    async fn poll_qr(
        &self,
        base_url: &ValidatedBaseUrl,
        qrcode: &SecretValue,
        verify_code: Option<&SecretValue>,
    ) -> Result<PollQrResponse, IlinkError>;
}

#[async_trait]
pub(in crate::api::code_web::weixin) trait IlinkMessagingTransport:
    Send + Sync
{
    fn validate_account_base_url(&self, base_url: &str) -> Result<ValidatedBaseUrl, IlinkError>;

    async fn get_updates(
        &self,
        auth: &IlinkAuth,
        update_cursor: &str,
        long_poll_timeout: Duration,
    ) -> Result<GetUpdatesResponse, IlinkError>;

    async fn send_text(
        &self,
        auth: &IlinkAuth,
        recipient: &SecretValue,
        context_token: Option<&SecretValue>,
        client_id: &str,
        run_id: Option<&str>,
        text: &str,
    ) -> Result<SendMessageResponse, IlinkError>;

    #[cfg(test)]
    async fn get_config(
        &self,
        auth: &IlinkAuth,
        owner_id: Option<&SecretValue>,
        context_token: Option<&SecretValue>,
    ) -> Result<GetConfigResponse, IlinkError>;

    #[cfg(test)]
    async fn send_typing(
        &self,
        auth: &IlinkAuth,
        owner_id: &SecretValue,
        typing_ticket: &SecretValue,
        status: i32,
    ) -> Result<SendTypingResponse, IlinkError>;

    async fn notify_start(&self, auth: &IlinkAuth) -> Result<NotifyResponse, IlinkError>;

    async fn notify_stop(&self, auth: &IlinkAuth) -> Result<NotifyResponse, IlinkError>;
}

pub(super) struct TencentIlinkTransport {
    pub(super) http: reqwest::Client,
    pub(super) identity: IlinkClientIdentity,
    pub(super) host_policy: IlinkHostPolicy,
    pub(super) qr_base_url: ValidatedBaseUrl,
    pub(super) max_response_bytes: usize,
}

impl TencentIlinkTransport {
    pub(super) fn new(
        identity: IlinkClientIdentity,
        host_policy: IlinkHostPolicy,
        qr_base_url: &str,
    ) -> Result<Self, IlinkError> {
        let qr_base_url = host_policy.validate(qr_base_url)?;
        let http = reqwest::Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .no_proxy()
            .build()
            .map_err(|_| IlinkError::InvalidConfiguration("HTTP client"))?;
        Ok(Self {
            http,
            identity,
            host_policy,
            qr_base_url,
            max_response_bytes: MAX_RESPONSE_BYTES,
        })
    }

    pub(super) fn validate_account_base_url(
        &self,
        base_url: &str,
    ) -> Result<ValidatedBaseUrl, IlinkError> {
        self.host_policy.validate(base_url).map_err(Into::into)
    }

    pub(super) fn validate_redirect_host(
        &self,
        redirect_host: &str,
    ) -> Result<ValidatedBaseUrl, IlinkError> {
        self.host_policy
            .validate_redirect_host(redirect_host)
            .map_err(Into::into)
    }

    pub(super) async fn get_json<T>(
        &self,
        request: reqwest::RequestBuilder,
        timeout: Duration,
        operation: &'static str,
    ) -> Result<T, IlinkError>
    where
        T: DeserializeOwned,
    {
        self.execute_json(request, timeout, operation).await
    }

    pub(super) async fn post_json<B, T>(
        &self,
        request: reqwest::RequestBuilder,
        body: &B,
        timeout: Duration,
        operation: &'static str,
    ) -> Result<T, IlinkError>
    where
        B: serde::Serialize + Sync,
        T: DeserializeOwned,
    {
        self.execute_json(request.json(body), timeout, operation)
            .await
    }

    async fn execute_json<T>(
        &self,
        request: reqwest::RequestBuilder,
        timeout: Duration,
        operation: &'static str,
    ) -> Result<T, IlinkError>
    where
        T: DeserializeOwned,
    {
        let response = request
            .timeout(timeout)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        if !response.status().is_success() {
            return Err(IlinkError::HttpStatus(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > self.max_response_bytes as u64)
        {
            return Err(IlinkError::ResponseTooLarge);
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(map_reqwest_error)?;
            if bytes.len().saturating_add(chunk.len()) > self.max_response_bytes {
                return Err(IlinkError::ResponseTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| IlinkError::InvalidResponse(operation))
    }

    pub(super) fn authenticated_post(
        &self,
        auth: &IlinkAuth,
        endpoint: &str,
    ) -> Result<reqwest::RequestBuilder, IlinkError> {
        let url = auth.base_url.join(endpoint)?;
        Ok(self
            .http
            .post(url)
            .headers(self.identity.post_headers(Some(&auth.bot_token))?))
    }

    pub(super) fn bounded_long_poll_timeout(timeout: Duration) -> Result<Duration, IlinkError> {
        if timeout.is_zero() || timeout > MAX_LONG_POLL_TIMEOUT {
            return Err(IlinkError::InvalidConfiguration("long poll timeout"));
        }
        Ok(timeout.saturating_add(Duration::from_secs(5)))
    }
}

fn map_reqwest_error(error: reqwest::Error) -> IlinkError {
    if error.is_timeout() {
        IlinkError::Timeout
    } else {
        IlinkError::Transport
    }
}

#[async_trait]
impl IlinkLoginTransport for TencentIlinkTransport {
    fn qr_base_url(&self) -> ValidatedBaseUrl {
        self.qr_base_url.clone()
    }

    fn validate_account_base_url(&self, base_url: &str) -> Result<ValidatedBaseUrl, IlinkError> {
        self.validate_account_base_url(base_url)
    }

    fn validate_redirect_host(&self, redirect_host: &str) -> Result<ValidatedBaseUrl, IlinkError> {
        self.validate_redirect_host(redirect_host)
    }

    async fn create_qr(&self) -> Result<CreateQrResponse, IlinkError> {
        self.create_qr_request().await
    }

    async fn poll_qr(
        &self,
        base_url: &ValidatedBaseUrl,
        qrcode: &SecretValue,
        verify_code: Option<&SecretValue>,
    ) -> Result<PollQrResponse, IlinkError> {
        self.poll_qr_request(base_url, qrcode, verify_code).await
    }
}

#[async_trait]
impl IlinkMessagingTransport for TencentIlinkTransport {
    fn validate_account_base_url(&self, base_url: &str) -> Result<ValidatedBaseUrl, IlinkError> {
        self.validate_account_base_url(base_url)
    }

    async fn get_updates(
        &self,
        auth: &IlinkAuth,
        update_cursor: &str,
        long_poll_timeout: Duration,
    ) -> Result<GetUpdatesResponse, IlinkError> {
        self.get_updates_request(auth, update_cursor, long_poll_timeout)
            .await
    }

    async fn send_text(
        &self,
        auth: &IlinkAuth,
        recipient: &SecretValue,
        context_token: Option<&SecretValue>,
        client_id: &str,
        run_id: Option<&str>,
        text: &str,
    ) -> Result<SendMessageResponse, IlinkError> {
        self.send_text_request(auth, recipient, context_token, client_id, run_id, text)
            .await
    }

    #[cfg(test)]
    async fn get_config(
        &self,
        auth: &IlinkAuth,
        owner_id: Option<&SecretValue>,
        context_token: Option<&SecretValue>,
    ) -> Result<GetConfigResponse, IlinkError> {
        self.get_config_request(auth, owner_id, context_token).await
    }

    #[cfg(test)]
    async fn send_typing(
        &self,
        auth: &IlinkAuth,
        owner_id: &SecretValue,
        typing_ticket: &SecretValue,
        status: i32,
    ) -> Result<SendTypingResponse, IlinkError> {
        self.send_typing_request(auth, owner_id, typing_ticket, status)
            .await
    }

    async fn notify_start(&self, auth: &IlinkAuth) -> Result<NotifyResponse, IlinkError> {
        self.notify_request(auth, true).await
    }

    async fn notify_stop(&self, auth: &IlinkAuth) -> Result<NotifyResponse, IlinkError> {
        self.notify_request(auth, false).await
    }
}
