mod auth;
mod client;
mod login;
mod messages;
mod transport;
mod types;
mod updates;
mod url_policy;

use std::sync::Arc;

pub(super) use auth::SecretValue;
pub(super) use client::{IlinkAuth, IlinkError};
pub(super) use transport::{IlinkLoginTransport, IlinkMessagingTransport};
#[cfg(test)]
pub(super) use types::{
    CreateQrResponse, GetConfigResponse, NotifyResponse, SendMessageResponse, SendTypingResponse,
};
pub(super) use types::{
    GetUpdatesResponse, PollQrResponse, QrCodeStatus, WeixinMessage, MESSAGE_STATE_FINISH,
    MESSAGE_TYPE_USER,
};
pub(super) use url_policy::ValidatedBaseUrl;

pub(super) struct IlinkProductionTransports {
    pub(super) login: Arc<dyn IlinkLoginTransport>,
    pub(super) messaging: Arc<dyn IlinkMessagingTransport>,
}

pub(super) fn production_transports(
    app_id: String,
    bot_type: String,
    client_version: &str,
    bot_agent: String,
    allowed_hosts: &[String],
) -> Result<IlinkProductionTransports, IlinkError> {
    let identity = client::IlinkClientIdentity::new(app_id, bot_type, client_version, bot_agent)?;
    let host_policy = url_policy::IlinkHostPolicy::production(allowed_hosts)?;
    let transport = Arc::new(transport::TencentIlinkTransport::new(
        identity,
        host_policy,
        client::FIXED_QR_BASE_URL,
    )?);
    let login: Arc<dyn IlinkLoginTransport> = transport.clone();
    let messaging: Arc<dyn IlinkMessagingTransport> = transport;
    Ok(IlinkProductionTransports { login, messaging })
}

#[cfg(test)]
mod tests;
