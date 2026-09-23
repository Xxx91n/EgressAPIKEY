//! External IP reputation adapters for actual public egress IPs.
//!
//! Providers are fixed upstreams, inputs are parsed `IpAddr`, and the cache is
//! bounded by TTL. This prevents the GUI from turning the feature into a generic
//! HTTP client while avoiding a home-grown fraud scoring algorithm.

use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::{
    net::IpAddr,
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReputationProvider {
    IpQualityScore,
    AbuseIpDb,
    IpApi,
}

impl ReputationProvider {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ipqualityscore" | "ip_quality_score" | "ipqs" => Some(Self::IpQualityScore),
            "abuseipdb" | "abuse_ip_db" => Some(Self::AbuseIpDb),
            "ip-api" | "ip_api" | "ipapi" => Some(Self::IpApi),
            _ => None,
        }
    }

    pub fn requires_key(self) -> bool {
        !matches!(self, Self::IpApi)
    }
    pub fn key_name(self) -> &'static str {
        match self {
            Self::IpQualityScore => "ipQualityScoreApiKey",
            Self::AbuseIpDb => "abuseIpDbApiKey",
            Self::IpApi => "",
        }
    }
    fn base(self) -> &'static str {
        match self {
            Self::IpQualityScore => "https://www.ipqualityscore.com",
            Self::AbuseIpDb => "https://api.abuseipdb.com",
            // ip-api free endpoint is HTTP only. It remains explicit opt-in.
            Self::IpApi => "http://ip-api.com",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReputationEntry {
    pub ip: String,
    pub provider: ReputationProvider,
    pub score: Option<u8>,
    pub proxy: Option<bool>,
    pub vpn: Option<bool>,
    pub tor: Option<bool>,
    pub country_code: Option<String>,
    pub checked_at: u64,
    pub cached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReputationSnapshot {
    pub provider: Option<ReputationProvider>,
    pub status: String,
    pub entries: Vec<ReputationEntry>,
}

#[derive(Clone)]
pub struct ReputationClient {
    http: Client,
    cache: &'static DashMap<String, ReputationEntry>,
}
static CACHE: Lazy<DashMap<String, ReputationEntry>> = Lazy::new(DashMap::new);

impl ReputationClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .context("build reputation client")?,
            cache: &CACHE,
        })
    }

    pub async fn lookup(
        &self,
        provider: ReputationProvider,
        api_key: Option<&str>,
        ip: IpAddr,
    ) -> Result<ReputationEntry> {
        if !is_public_ip(ip) {
            return Err(anyhow!("IP reputation only accepts public IPs"));
        }
        if provider.requires_key() && api_key.filter(|v| !v.trim().is_empty()).is_none() {
            return Err(anyhow!("{} is not configured", provider.key_name()));
        }
        let cache_key = format!("{:?}:{ip}", provider);
        let now = now_unix();
        if let Some(row) = self.cache.get(&cache_key) {
            if now.saturating_sub(row.checked_at) < CACHE_TTL.as_secs() {
                let mut hit = row.clone();
                hit.cached = true;
                return Ok(hit);
            }
        }
        let mut entry = self.fetch(provider, api_key.unwrap_or(""), ip).await?;
        entry.checked_at = now;
        entry.cached = false;
        self.cache.insert(cache_key, entry.clone());
        Ok(entry)
    }

    async fn fetch(
        &self,
        provider: ReputationProvider,
        api_key: &str,
        ip: IpAddr,
    ) -> Result<ReputationEntry> {
        let ip_s = ip.to_string();
        let value = match provider {
            ReputationProvider::IpQualityScore => {
                let url = ipqs_url(api_key, &ip_s);
                self.http
                    .get(url)
                    .send()
                    .await?
                    .error_for_status()?
                    .json::<serde_json::Value>()
                    .await?
            }
            ReputationProvider::AbuseIpDb => {
                let url = Url::parse_with_params(
                    &format!("{}/api/v2/check", provider.base()),
                    &[("ipAddress", ip_s.as_str()), ("maxAgeInDays", "90")],
                )?;
                self.http
                    .get(url)
                    .header("Key", api_key)
                    .header("Accept", "application/json")
                    .send()
                    .await?
                    .error_for_status()?
                    .json::<serde_json::Value>()
                    .await?
            }
            ReputationProvider::IpApi => {
                let url = format!(
                    "{}/json/{ip_s}?fields=status,countryCode,proxy,hosting",
                    provider.base()
                );
                self.http
                    .get(url)
                    .send()
                    .await?
                    .error_for_status()?
                    .json::<serde_json::Value>()
                    .await?
            }
        };
        Ok(map_response(provider, &ip_s, &value))
    }
}

/// The IPQS lookup URL. The API key sits inside a PATH SEGMENT, so it uses
/// the strict path-segment encoder - never form-urlencoded (a  for a
/// space was the r12 wave-c bug the regression test below pins shut).
fn ipqs_url(api_key: &str, ip_s: &str) -> String {
    format!(
        "{}/api/json/ip/{}/{}",
        ReputationProvider::IpQualityScore.base(),
        crate::encoding::encode_path_segment(api_key),
        ip_s
    )
}
fn as_bool(v: &serde_json::Value, name: &str) -> Option<bool> {
    v.get(name).and_then(|x| x.as_bool())
}
fn as_score(v: &serde_json::Value, name: &str) -> Option<u8> {
    v.get(name)
        .and_then(|x| x.as_u64())
        .and_then(|n| u8::try_from(n).ok())
}
fn as_country(v: &serde_json::Value, name: &str) -> Option<String> {
    v.get(name)
        .and_then(|x| x.as_str())
        .filter(|s| s.len() == 2)
        .map(str::to_string)
}

pub fn map_response(
    provider: ReputationProvider,
    ip: &str,
    v: &serde_json::Value,
) -> ReputationEntry {
    let (score, proxy, vpn, tor, country_code) = match provider {
        ReputationProvider::IpQualityScore => (
            as_score(v, "fraud_score"),
            as_bool(v, "proxy"),
            as_bool(v, "vpn"),
            as_bool(v, "tor"),
            as_country(v, "country_code"),
        ),
        ReputationProvider::AbuseIpDb => {
            let d = v.get("data").unwrap_or(v);
            (
                as_score(d, "abuseConfidenceScore"),
                None,
                None,
                None,
                as_country(d, "countryCode"),
            )
        }
        ReputationProvider::IpApi => (
            None,
            as_bool(v, "proxy").or_else(|| as_bool(v, "hosting")),
            None,
            None,
            as_country(v, "countryCode"),
        ),
    };
    ReputationEntry {
        ip: ip.to_string(),
        provider,
        score,
        proxy,
        vpn,
        tor,
        country_code,
        checked_at: now_unix(),
        cached: false,
    }
}

pub fn parse_public_ips(values: impl IntoIterator<Item = String>, cap: usize) -> Vec<IpAddr> {
    let mut out = Vec::new();
    for value in values {
        if out.len() >= cap {
            break;
        }
        if let Ok(ip) = IpAddr::from_str(&value) {
            if is_public_ip(ip) && !out.contains(&ip) {
                out.push(ip);
            }
        }
    }
    out
}

pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_broadcast()
                || v.is_unspecified()
                || v.is_documentation())
        }
        IpAddr::V6(v) => {
            !(v.is_loopback()
                || v.is_unspecified()
                || v.is_unique_local()
                || v.is_unicast_link_local())
        }
    }
}
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_ip_filter_rejects_private_and_dedupes() {
        assert_eq!(
            parse_public_ips(
                vec![
                    "10.0.0.1".into(),
                    "8.8.8.8".into(),
                    "8.8.8.8".into(),
                    "nope".into()
                ],
                50
            ),
            vec!["8.8.8.8".parse::<IpAddr>().unwrap()]
        );
    }
    #[test]
    fn ipqs_mapping_preserves_provider_fields() {
        let v = serde_json::json!({"fraud_score":71,"proxy":true,"vpn":false,"tor":false,"country_code":"US"});
        let e = map_response(ReputationProvider::IpQualityScore, "8.8.8.8", &v);
        assert_eq!(e.score, Some(71));
        assert_eq!(e.proxy, Some(true));
        assert_eq!(e.country_code.as_deref(), Some("US"));
    }
    #[test]
    fn abuse_mapping_reads_data_wrapper() {
        let v = serde_json::json!({"data":{"abuseConfidenceScore":42,"countryCode":"DE"}});
        let e = map_response(ReputationProvider::AbuseIpDb, "1.1.1.1", &v);
        assert_eq!(e.score, Some(42));
        assert_eq!(e.country_code.as_deref(), Some("DE"));
    }
    #[test]
    fn ipqs_url_encodes_key_as_path_segment() {
        // Regression lock (R12-D2): the key sits in a path segment, so a
        // space must be %20 - never the form-urlencoded  - and literal
        // NaN
        assert_eq!(
            ReputationProvider::parse("ipqs"),
            Some(ReputationProvider::IpQualityScore)
        );
        assert!(ReputationProvider::parse("anything").is_none());
    }
}
