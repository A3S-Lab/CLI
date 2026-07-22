use std::collections::HashSet;
use std::fmt;

use thiserror::Error;
use url::{Host, Url};

#[derive(Clone)]
pub(super) struct IlinkHostPolicy {
    allowed_hosts: HashSet<String>,
    allow_insecure_loopback: bool,
}

impl IlinkHostPolicy {
    pub(super) fn production(
        allowed_hosts: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<Self, IlinkUrlError> {
        let mut normalized = HashSet::new();
        for host in allowed_hosts {
            let host = normalize_allowed_host(host.as_ref())?;
            normalized.insert(host);
        }
        if normalized.is_empty() {
            return Err(IlinkUrlError::EmptyHostPolicy);
        }
        Ok(Self {
            allowed_hosts: normalized,
            allow_insecure_loopback: false,
        })
    }

    #[cfg(test)]
    pub(super) fn for_test_origin(origin: &str) -> Result<Self, IlinkUrlError> {
        let url = Url::parse(origin).map_err(|_| IlinkUrlError::InvalidUrl)?;
        let host = url
            .host_str()
            .ok_or(IlinkUrlError::MissingHost)?
            .to_ascii_lowercase();
        if url.scheme() != "http" || !is_loopback_host(&host) {
            return Err(IlinkUrlError::InsecureScheme);
        }
        Ok(Self {
            allowed_hosts: [host].into_iter().collect(),
            allow_insecure_loopback: true,
        })
    }

    pub(super) fn validate(&self, candidate: &str) -> Result<ValidatedBaseUrl, IlinkUrlError> {
        let mut url = Url::parse(candidate).map_err(|_| IlinkUrlError::InvalidUrl)?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(IlinkUrlError::UserInfoNotAllowed);
        }
        if url.query().is_some() {
            return Err(IlinkUrlError::QueryNotAllowed);
        }
        if url.fragment().is_some() {
            return Err(IlinkUrlError::FragmentNotAllowed);
        }
        let host = url
            .host_str()
            .ok_or(IlinkUrlError::MissingHost)?
            .to_ascii_lowercase();
        let production_https = url.scheme() == "https" && url.port().is_none_or(|port| port == 443);
        let test_http =
            self.allow_insecure_loopback && url.scheme() == "http" && is_loopback_host(&host);
        if !production_https && !test_http {
            return if url.scheme() != "https" && !test_http {
                Err(IlinkUrlError::InsecureScheme)
            } else {
                Err(IlinkUrlError::PortNotAllowed)
            };
        }
        match url.host() {
            Some(Host::Domain(_)) => {}
            Some(Host::Ipv4(_)) | Some(Host::Ipv6(_)) if test_http => {}
            Some(Host::Ipv4(_)) | Some(Host::Ipv6(_)) => {
                return Err(IlinkUrlError::IpLiteralNotAllowed)
            }
            None => return Err(IlinkUrlError::MissingHost),
        }
        if !self.allowed_hosts.contains(&host) {
            return Err(IlinkUrlError::HostNotAllowed);
        }
        if !url.path().ends_with('/') {
            let path = format!("{}/", url.path());
            url.set_path(&path);
        }
        Ok(ValidatedBaseUrl(url))
    }

    pub(super) fn validate_redirect_host(
        &self,
        candidate: &str,
    ) -> Result<ValidatedBaseUrl, IlinkUrlError> {
        let probe =
            Url::parse(&format!("https://{candidate}/")).map_err(|_| IlinkUrlError::InvalidUrl)?;
        if !probe.username().is_empty()
            || probe.password().is_some()
            || probe.path() != "/"
            || probe.query().is_some()
            || probe.fragment().is_some()
        {
            return Err(IlinkUrlError::InvalidUrl);
        }
        let host = probe
            .host_str()
            .ok_or(IlinkUrlError::MissingHost)?
            .to_ascii_lowercase();
        let scheme = if self.allow_insecure_loopback && is_loopback_host(&host) {
            "http"
        } else {
            "https"
        };
        self.validate(&format!("{scheme}://{candidate}/"))
    }
}

fn normalize_allowed_host(host: &str) -> Result<String, IlinkUrlError> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty()
        || host.contains('/')
        || host.contains(':')
        || host.parse::<std::net::IpAddr>().is_ok()
    {
        return Err(IlinkUrlError::InvalidAllowedHost);
    }
    let probe =
        Url::parse(&format!("https://{host}")).map_err(|_| IlinkUrlError::InvalidAllowedHost)?;
    if !matches!(probe.host(), Some(Host::Domain(_))) {
        return Err(IlinkUrlError::InvalidAllowedHost);
    }
    Ok(host)
}

fn is_loopback_host(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::api::code_web::weixin) struct ValidatedBaseUrl(Url);

impl ValidatedBaseUrl {
    pub(super) fn join(&self, endpoint: &str) -> Result<Url, IlinkUrlError> {
        if endpoint.starts_with('/') || endpoint.contains("..") {
            return Err(IlinkUrlError::InvalidEndpoint);
        }
        self.0
            .join(endpoint)
            .map_err(|_| IlinkUrlError::InvalidEndpoint)
    }

    #[cfg(test)]
    pub(in crate::api::code_web::weixin) fn for_test(origin: &str) -> Result<Self, IlinkUrlError> {
        IlinkHostPolicy::for_test_origin(origin)?.validate(origin)
    }
}

impl fmt::Debug for ValidatedBaseUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValidatedBaseUrl")
            .field("scheme", &self.0.scheme())
            .field("host", &self.0.host_str())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(in crate::api::code_web::weixin) enum IlinkUrlError {
    #[error("iLink URL is invalid")]
    InvalidUrl,
    #[error("iLink URL must use HTTPS")]
    InsecureScheme,
    #[error("iLink URL host is missing")]
    MissingHost,
    #[error("iLink URL user information is not allowed")]
    UserInfoNotAllowed,
    #[error("iLink URL port is not allowed")]
    PortNotAllowed,
    #[error("iLink URL IP literals are not allowed")]
    IpLiteralNotAllowed,
    #[error("iLink URL host is not approved")]
    HostNotAllowed,
    #[error("iLink URL query is not allowed")]
    QueryNotAllowed,
    #[error("iLink URL fragment is not allowed")]
    FragmentNotAllowed,
    #[error("iLink endpoint is invalid")]
    InvalidEndpoint,
    #[error("iLink host policy must not be empty")]
    EmptyHostPolicy,
    #[error("iLink allowed host is invalid")]
    InvalidAllowedHost,
}
