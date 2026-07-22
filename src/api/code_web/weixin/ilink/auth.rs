use std::fmt;

use base64::Engine as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use zeroize::{Zeroize, ZeroizeOnDrop};

const MAX_SECRET_BYTES: usize = 64 * 1024;

#[derive(Clone, PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub(in crate::api::code_web::weixin) struct SecretValue(String);

impl SecretValue {
    pub(in crate::api::code_web::weixin) fn new(
        value: impl Into<String>,
    ) -> Result<Self, SecretValueError> {
        let value = value.into();
        if value.is_empty() {
            return Err(SecretValueError::Empty);
        }
        if value.len() > MAX_SECRET_BYTES {
            return Err(SecretValueError::TooLarge);
        }
        Ok(Self(value))
    }

    pub(in crate::api::code_web::weixin) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

impl Serialize for SecretValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.expose())
    }
}

impl<'de> Deserialize<'de> for SecretValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(in crate::api::code_web::weixin) enum SecretValueError {
    #[error("secret value is empty")]
    Empty,
    #[error("secret value exceeds the protocol size limit")]
    TooLarge,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub(in crate::api::code_web::weixin) enum ClientVersionError {
    #[error("client version must contain exactly three numeric components")]
    InvalidShape,
    #[error("client version component exceeds 255")]
    ComponentOutOfRange,
}

pub(super) fn pack_client_version(version: &str) -> Result<u32, ClientVersionError> {
    let components = version.split('.').collect::<Vec<_>>();
    if components.len() != 3 || components.iter().any(|component| component.is_empty()) {
        return Err(ClientVersionError::InvalidShape);
    }
    let mut parsed = [0u32; 3];
    for (index, component) in components.into_iter().enumerate() {
        let value = component
            .parse::<u32>()
            .map_err(|_| ClientVersionError::InvalidShape)?;
        if value > u8::MAX as u32 {
            return Err(ClientVersionError::ComponentOutOfRange);
        }
        parsed[index] = value;
    }
    Ok((parsed[0] << 16) | (parsed[1] << 8) | parsed[2])
}

pub(super) fn random_wechat_uin() -> String {
    base64::engine::general_purpose::STANDARD.encode(rand::random::<u32>().to_string())
}
