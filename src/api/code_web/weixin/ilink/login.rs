use super::auth::SecretValue;
use super::client::{IlinkError, DEFAULT_API_TIMEOUT, DEFAULT_QR_POLL_TIMEOUT};
use super::transport::TencentIlinkTransport;
use super::types::{CreateQrRequest, CreateQrResponse, PollQrResponse};
use super::url_policy::ValidatedBaseUrl;

const MAX_QR_IMAGE_CONTENT_BYTES: usize = 256 * 1024;

impl TencentIlinkTransport {
    pub(super) async fn create_qr_request(&self) -> Result<CreateQrResponse, IlinkError> {
        let mut url = self.qr_base_url.join("ilink/bot/get_bot_qrcode")?;
        url.query_pairs_mut()
            .append_pair("bot_type", &self.identity.bot_type);
        let request = self
            .http
            .post(url)
            .headers(self.identity.post_headers(None)?);
        let response: CreateQrResponse = self
            .post_json(
                request,
                &CreateQrRequest {
                    local_token_list: Vec::new(),
                },
                DEFAULT_API_TIMEOUT,
                "create_qr",
            )
            .await?;
        if response.qrcode_img_content.expose().len() > MAX_QR_IMAGE_CONTENT_BYTES
            || response.qrcode_img_content.expose().contains('\0')
        {
            return Err(IlinkError::InvalidResponse("create_qr"));
        }
        Ok(response)
    }

    pub(super) async fn poll_qr_request(
        &self,
        base_url: &ValidatedBaseUrl,
        qrcode: &SecretValue,
        verify_code: Option<&SecretValue>,
    ) -> Result<PollQrResponse, IlinkError> {
        let mut url = base_url.join("ilink/bot/get_qrcode_status")?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("qrcode", qrcode.expose());
            if let Some(verify_code) = verify_code {
                query.append_pair("verify_code", verify_code.expose());
            }
        }
        let request = self
            .http
            .get(url)
            .headers(self.identity.application_headers()?);
        let response: PollQrResponse = self
            .get_json(request, DEFAULT_QR_POLL_TIMEOUT, "poll_qr")
            .await?;
        match response.status {
            super::types::QrCodeStatus::Unknown => {
                return Err(IlinkError::InvalidResponse("poll_qr"));
            }
            super::types::QrCodeStatus::ScanedButRedirect => {
                let redirect_host = response
                    .redirect_host
                    .as_deref()
                    .ok_or(IlinkError::InvalidResponse("poll_qr"))?;
                self.host_policy.validate_redirect_host(redirect_host)?;
            }
            super::types::QrCodeStatus::Confirmed => {
                if response.bot_token.is_none()
                    || response.ilink_bot_id.is_none()
                    || response.ilink_user_id.is_none()
                {
                    return Err(IlinkError::InvalidResponse("poll_qr"));
                }
                let account_base_url = response
                    .baseurl
                    .as_deref()
                    .ok_or(IlinkError::InvalidResponse("poll_qr"))?;
                self.host_policy.validate(account_base_url)?;
            }
            _ => {}
        }
        Ok(response)
    }
}
