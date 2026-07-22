use std::fmt;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use thiserror::Error;

use super::auth::{pack_client_version, random_wechat_uin, ClientVersionError, SecretValue};
use super::url_policy::{IlinkUrlError, ValidatedBaseUrl};

pub(super) const FIXED_QR_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
pub(super) const STALE_TOKEN_ERROR_CODE: i64 = -14;
pub(super) const DEFAULT_API_TIMEOUT: Duration = Duration::from_secs(15);
pub(super) const DEFAULT_CONFIG_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const DEFAULT_QR_POLL_TIMEOUT: Duration = Duration::from_secs(35);
pub(super) const MAX_LONG_POLL_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

const AUTHORIZATION_TYPE: HeaderName = HeaderName::from_static("authorizationtype");
const WECHAT_UIN: HeaderName = HeaderName::from_static("x-wechat-uin");
const ILINK_APP_ID: HeaderName = HeaderName::from_static("ilink-app-id");
const ILINK_APP_CLIENT_VERSION: HeaderName = HeaderName::from_static("ilink-app-clientversion");

#[derive(Clone, Debug)]
pub(super) struct IlinkClientIdentity {
    app_id: String,
    packed_client_version: u32,
    pub(super) bot_type: String,
    pub(super) channel_version: String,
    pub(super) bot_agent: String,
}

impl IlinkClientIdentity {
    pub(super) fn new(
        app_id: impl Into<String>,
        bot_type: impl Into<String>,
        client_version: &str,
        bot_agent: impl Into<String>,
    ) -> Result<Self, IlinkError> {
        let app_id = bounded_ascii(app_id.into(), "app id", 128)?;
        let bot_type = bounded_ascii(bot_type.into(), "bot type", 16)?;
        let channel_version = bounded_ascii(client_version.to_string(), "client version", 32)?;
        let bot_agent = bounded_ascii(bot_agent.into(), "bot agent", 256)?;
        let packed_client_version = pack_client_version(client_version)?;
        Ok(Self {
            app_id,
            packed_client_version,
            bot_type,
            channel_version,
            bot_agent,
        })
    }

    pub(super) fn base_info(&self) -> super::types::BaseInfo {
        super::types::BaseInfo {
            channel_version: Some(self.channel_version.clone()),
            bot_agent: Some(self.bot_agent.clone()),
        }
    }

    pub(super) fn application_headers(&self) -> Result<HeaderMap, IlinkError> {
        let mut headers = HeaderMap::new();
        headers.insert(ILINK_APP_ID, safe_header_value(&self.app_id, "app id")?);
        headers.insert(
            ILINK_APP_CLIENT_VERSION,
            safe_header_value(&self.packed_client_version.to_string(), "client version")?,
        );
        Ok(headers)
    }

    pub(super) fn post_headers(
        &self,
        token: Option<&SecretValue>,
    ) -> Result<HeaderMap, IlinkError> {
        let mut headers = self.application_headers()?;
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION_TYPE,
            HeaderValue::from_static("ilink_bot_token"),
        );
        headers.insert(
            WECHAT_UIN,
            safe_header_value(&random_wechat_uin(), "Weixin UIN")?,
        );
        if let Some(token) = token {
            headers.insert(
                AUTHORIZATION,
                safe_header_value(&format!("Bearer {}", token.expose()), "authorization")?,
            );
        }
        Ok(headers)
    }
}

#[derive(Clone)]
pub(in crate::api::code_web::weixin) struct IlinkAuth {
    pub(super) base_url: ValidatedBaseUrl,
    pub(super) bot_token: SecretValue,
}

impl IlinkAuth {
    pub(in crate::api::code_web::weixin) fn new(
        base_url: ValidatedBaseUrl,
        bot_token: SecretValue,
    ) -> Self {
        Self {
            base_url,
            bot_token,
        }
    }
}

impl fmt::Debug for IlinkAuth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IlinkAuth")
            .field("base_url", &self.base_url)
            .field("bot_token", &self.bot_token)
            .finish()
    }
}

fn bounded_ascii(value: String, field: &'static str, max: usize) -> Result<String, IlinkError> {
    if value.is_empty()
        || value.len() > max
        || !value.is_ascii()
        || value.chars().any(char::is_control)
    {
        return Err(IlinkError::InvalidConfiguration(field));
    }
    Ok(value)
}

fn safe_header_value(value: &str, field: &'static str) -> Result<HeaderValue, IlinkError> {
    HeaderValue::from_str(value).map_err(|_| IlinkError::InvalidConfiguration(field))
}

pub(super) fn ensure_api_success(
    operation: &'static str,
    ret: Option<i64>,
    errcode: Option<i64>,
) -> Result<(), IlinkError> {
    let code = errcode
        .filter(|code| *code != 0)
        .or_else(|| ret.filter(|code| *code != 0));
    match code {
        None => Ok(()),
        Some(STALE_TOKEN_ERROR_CODE) => Err(IlinkError::StaleCredential),
        Some(code) => Err(IlinkError::Protocol { operation, code }),
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub(in crate::api::code_web::weixin) enum IlinkError {
    #[error("iLink configuration field is invalid: {0}")]
    InvalidConfiguration(&'static str),
    #[error("iLink URL policy rejected the request")]
    Url(#[from] IlinkUrlError),
    #[error("iLink client version is invalid")]
    ClientVersion(#[from] ClientVersionError),
    #[error("iLink request timed out")]
    Timeout,
    #[error("iLink transport failed")]
    Transport,
    #[error("iLink returned HTTP status {0}")]
    HttpStatus(u16),
    #[error("iLink response exceeds the size limit")]
    ResponseTooLarge,
    #[error("iLink returned an invalid response for {0}")]
    InvalidResponse(&'static str),
    #[error("iLink credential is stale")]
    StaleCredential,
    #[error("iLink operation {operation} failed with code {code}")]
    Protocol { operation: &'static str, code: i64 },
}
