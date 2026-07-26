//! mihomo sidecar REST API control.
//!
//! Mirrors Resin's subscription -> compiled YAML -> hot-reload flow but for
//! mihomo (the L4 sidecar). The core compiles a subscription URL into a
//! Clash YAML config, then calls the mihomo REST API (/configs?force=true)
//! with the new payload. pool_max_idle_per_host(0) is set in YAML so every
//! request opens a fresh TCP and gets a distinct exit IP per lane.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Config the core pushes into mihomo via REST.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MihomoConfig {
    pub mixed_port: u16,
    pub mode: String,
    pub log_level: String,
    pub allow_lan: bool,
    pub pool_max_idle_per_host: u32,
    pub proxies: Vec<serde_json::Value>,
    pub proxy_groups: Vec<serde_json::Value>,
    pub rules: Vec<String>,
}

impl Default for MihomoConfig {
    fn default() -> Self {
        Self {
            mixed_port: 7897,
            mode: "rule".to_string(),
            log_level: "warning".to_string(),
            allow_lan: false,
            pool_max_idle_per_host: 0,
            proxies: Vec::new(),
            proxy_groups: Vec::new(),
            rules: Vec::new(),
        }
    }
}

/// Compiled YAML-ready config for one lane of N.
#[derive(Debug, Clone, Serialize)]
pub struct LaneAssignment {
    pub lane: usize,
    pub inbound_port: u16,
    pub exit_ip: Option<String>,
    pub proxy_name: String,
}

/// REST controller for the mihomo sidecar.
pub struct MihomoController {
    api_base: String,
    secret: Option<String>,
    client: reqwest::Client,
}

impl MihomoController {
    pub fn new(api_base: impl Into<String>, secret: Option<String>) -> Self {
        Self { api_base: api_base.into(), secret, client: reqwest::Client::new() }
    }

    /// Hot-reload mihomo with a new config payload.
    pub async fn reload_config(&self, cfg: &MihomoConfig) -> Result<()> {
        let mut req = self.client.put(format!("{}/configs?force=true", self.api_base)).json(cfg);
        if let Some(s) = &self.secret {
            req = req.header("Authorization", format!("Bearer {s}"));
        }
        let resp = req.send().await.map_err(|e| anyhow!("mihomo reload request failed: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(anyhow!("mihomo reload failed: {status} body={body}"))
        }
    }

    /// Fetch mihomo runtime version (cheap liveness probe).
    pub async fn version(&self) -> Result<String> {
        let mut req = self.client.get(format!("{}/version", self.api_base));
        if let Some(s) = &self.secret {
            req = req.header("Authorization", format!("Bearer {s}"));
        }
        let resp = req.send().await.map_err(|e| anyhow!("mihomo version request failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(anyhow!("mihomo version request status {}", resp.status()));
        }
        let v: serde_json::Value = resp.json().await.map_err(|e| anyhow!("mihomo version parse: {e}"))?;
        v.get("version").and_then(|v| v.as_str()).map(|s| s.to_string()).ok_or_else(|| anyhow!("mihomo version payload missing 'version' field"))
    }
}

/// Build a MihomoConfig skeleton for N lanes, sharing one subscription's proxies.
pub fn build_skeleton_config(n_lanes: usize, mixed_port: u16) -> MihomoConfig {
    let mut cfg = MihomoConfig::default();
    cfg.mixed_port = mixed_port;
    cfg.rules = vec!["MATCH,DIRECT".to_string()];
    let _ = n_lanes;
    cfg
}

/// Compile a subscription URL into a MihomoConfig. Skeleton: production
/// fetches the subscription YAML and parses it into proxies/groups, mapping
/// each lane to one outbound. For tests and CI we only validate the skeleton.
pub async fn compile_subscription(_url: &str, n_lanes: usize, mixed_port: u16) -> Result<MihomoConfig> {
    Ok(build_skeleton_config(n_lanes, mixed_port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_config_has_zero_idle_pool() {
        let cfg = build_skeleton_config(10, 7897);
        assert_eq!(cfg.pool_max_idle_per_host, 0);
        assert_eq!(cfg.mixed_port, 7897);
    }

    #[test]
    fn config_serializes_to_yaml_friendly_json() {
        let cfg = build_skeleton_config(5, 7897);
        let s = serde_json::to_string(&cfg).unwrap();
        assert!(s.contains("pool_max_idle_per_host"));
        assert!(s.contains("\"mode\":\"rule\""));
    }

    #[tokio::test]
    async fn compile_subscription_returns_skeleton() {
        let cfg = compile_subscription("https://example.invalid/sub.yaml", 10, 7897).await.unwrap();
        assert_eq!(cfg.mixed_port, 7897);
        assert_eq!(cfg.pool_max_idle_per_host, 0);
    }

    #[test]
    fn lane_assignment_serializes() {
        let la = LaneAssignment { lane: 0, inbound_port: 7897, exit_ip: Some("1.2.3.4".into()), proxy_name: "L0".into() };
        let s = serde_json::to_string(&la).unwrap();
        assert!(s.contains("\"lane\":0"));
    }
}
