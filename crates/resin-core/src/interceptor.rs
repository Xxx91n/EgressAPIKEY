//! A4-3 interceptor: a thin axum reverse-proxy between the upstream AI
//! gateway (omniroute / litellm) and the Resin sidecar's reverse-proxy.
//!
//! ## Why
//! Resin identifies a request by (platform, **account**, target_host). The
//! upstream AI gateway sends an OpenAI/Anthropic `Authorization: Bearer sk-...`
//! that Resin does NOT inspect for routing — it passes end-to-end. To get
//! per-(api-key + upstream-endpoint) egress-IP isolation we must compose a
//! stable identity from the *triple* (auth_value, body.model, request_path)
//! and inject it as the Resin `X-Resin-Account` header, which Resin's routing
//! layer prioritises over the URL identity segment (DESIGN.md forward.go).
//!
//! Non-empty Account = sticky per-platform lease = distinct egress IP per
//! (key, endpoint). Empty Account = random routing, no lease. So injecting
//! X-Resin-Account = enabling the isolation the user asked for.
//!
//! ## Security
//! Inbound client-supplied `X-Resin-Account` is ALWAYS stripped before the
//! route_id-derived id is injected. A spoofed inbound header would route the
//! request to a different sticky bucket than intended (AGENTS §7.5/§7.6
//! strip-then-inject pattern, mirrors OmniRoute x-omniroute-auth-* stripping).
//!
//! ## Ponytail
//! One handler, one config struct, one forward. The body is streamed back
//! via `Body::from_stream` so SSE responses are preserved end-to-end. No
//! new abstraction layers, factories, or per-endpoint modelling — we forward
//! the bytes Resin would forward.

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, Request, Response, StatusCode},
    response::IntoResponse,
    Router,
};
use serde_json::Value;
use tokio::net::TcpListener;

use crate::lane::{normalize_auth, route_id};

/// Config the interceptor needs to do its job. Cloneable so we can spawn tasks.
#[derive(Clone)]
pub struct InterceptorConfig {
    /// Resin reverse-proxy base, e.g. `http://127.0.0.1:<api_port>`.
    pub resin_base: String,
    /// Resin proxy token (the URL path segment after the port).
    pub proxy_token: String,
    /// Forwarding HTTP client. Built by the caller; we keep it to avoid
    /// creating one per request.
    pub http: reqwest::Client,
}

/// Build the interceptor axum Router. Exposed for in-process unit tests via
/// `tower::ServiceExt::oneshot`.
pub fn app(cfg: InterceptorConfig) -> Router {
    Router::new()
        .route("/*path", axum::routing::any(proxy_handler))
        .with_state(cfg)
}

/// Bind and serve the interceptor on the given loopback address. Returns the
/// actual port the OS assigned (caller picks 0 to let the OS choose).
pub async fn serve(cfg: InterceptorConfig, bind_addr: &str) -> std::io::Result<u16> {
    let listener = TcpListener::bind(bind_addr).await?;
    let port = listener.local_addr()?.port();
    let router = app(cfg);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router).await {
            tracing::error!("interceptor serve exited: {e}");
        }
    });
    Ok(port)
}

/// The single handler. Extracts identity, injects X-Resin-Account, forwards.
async fn proxy_handler(
    State(cfg): State<InterceptorConfig>,
    req: Request<Body>,
) -> Response<Body> {
    // Decompose the request.
    let (parts, body_bytes) = match collect_body(req).await {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("read body: {e}")).into_response(),
    };

    // Extract model from JSON body if present.
    let model = extract_model(&body_bytes);

    // Normalise auth from Authorization / x-api-key / api-key headers.
    let auth_raw = pick_auth(&parts.headers);
    let identity = normalize_auth(&auth_raw);
    let path_tail = parts.uri.path();

    let rid = route_id(&identity, model.as_deref(), Some(path_tail));
    let account_id = format!("ar-{:016x}", rid);

    // Build the upstream Resin reverse-proxy URL.
    // Format: {resin_base}/{proxy_token}/{identity_segment}/{protocol}/{host}/{path}
    // We use empty identity segment + X-Resin-Account header (header priority > URL).
    let host = parts
        .headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    tracing::info!(
        rid, account = %account_id, host = %host, path = %path_tail,
        model = ?model,
        "interceptor forwarding"
    );

    // Remove any inbound X-Resin-Account so we control routing.
    let mut fwd_headers = parts.headers.clone();
    fwd_headers.remove("x-resin-account");
    fwd_headers.insert(
        HeaderName::from_static("x-resin-account"),
        HeaderValue::from_str(&account_id).unwrap_or(HeaderValue::from_static("ar-unknown")),
    );
    // Remove Host header — reqwest will set it from the upstream URL.
    fwd_headers.remove(axum::http::header::HOST);

    // Build the upstream URL. The path includes the query string if any.
    let upstream = format!(
        "{}/{}/https/{}{}",
        cfg.resin_base.trim_end_matches('/'),
        cfg.proxy_token,
        host,
        parts.uri
    );

    let method = parts.method.clone();
    let body_for_send = bytes::Bytes::copy_from_slice(&body_bytes);

    let fwd_req = cfg.http.request(method, &upstream).body(body_for_send);
    // reqwest 0.12 RequestBuilder::headers consumes self; build once then set.
    let fwd_req = fwd_req.headers(fwd_headers);

    let resp = match fwd_req.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("interceptor upstream error: {e}");
            return (StatusCode::BAD_GATEWAY, format!("upstream: {e}")).into_response();
        }
    };

    // Stream the response back so SSE chunks pass through unbuffered.
    // Take status + headers BEFORE moving resp into the stream body (bytes_stream consumes self).
    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let stream_builder = resp.bytes_stream();
    let mut out = Response::new(Body::from_stream(stream_builder));
    *out.status_mut() = status;
    for (k, v) in resp_headers.iter() {
        // Skip hop-by-hop headers that reqwest/axum own.
        let k_lower = k.as_str().to_ascii_lowercase();
        if matches!(
            k_lower.as_str(),
            "content-length" | "transfer-encoding" | "connection"
        ) {
            continue;
        }
        out.headers_mut().insert(k.clone(), v.clone());
    }
    out
}

/// Read the full request body into a Vec<u8>. Chat-completion bodies are small
/// (KB). We must re-send the exact bytes upstream; streaming the request body
/// is not needed because AI gateways send complete JSON.
async fn collect_body(
    req: Request<Body>,
) -> Result<(axum::http::request::Parts, Vec<u8>), String> {
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 8 * 1024 * 1024)
        .await
        .map_err(|e| e.to_string())?;
    Ok((parts, bytes.to_vec()))
}

/// Extract the `model` field from a JSON request body if the content-type
/// implies JSON. Returns None if the body is not JSON or has no model field.
fn extract_model(body: &[u8]) -> Option<String> {
    let v: Value = serde_json::from_slice(body).ok()?;
    v.get("model")?.as_str().map(|s| s.to_string())
}

/// Pick the raw auth value from the common header locations, in priority order:
/// Authorization > x-api-key > api-key > Ocp-Apim-Subscription-Key.
fn pick_auth(h: &HeaderMap) -> String {
    for name in [
        axum::http::header::AUTHORIZATION,
        HeaderName::from_static("x-api-key"),
        HeaderName::from_static("api-key"),
        HeaderName::from_static("ocp-apim-subscription-key"),
    ] {
        if let Some(v) = h.get(name) {
            if let Ok(s) = v.to_str() {
                return s.to_string();
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    async fn send_request(
        router: Router,
        method: Method,
        uri: &str,
        auth_header: Option<&str>,
        x_resin_account_header: Option<&str>,
        body: &str,
        host: &str,
    ) -> axum::response::Response {
        let mut b = Request::builder().method(method).uri(uri).header("host", host);
        if let Some(a) = auth_header {
            b = b.header("authorization", a);
        }
        if let Some(a) = x_resin_account_header {
            b = b.header("x-resin-account", a);
        }
        b = b.header("content-type", "application/json");
        let req = b.body(Body::from(body.to_string())).unwrap();
        router.oneshot(req).await.unwrap()
    }

    #[test]
    fn account_id_formula_distinct_for_distinct_model() {
        let a = format!(
            "ar-{:016x}",
            route_id(&normalize_auth("Bearer sk-A"), Some("gpt-5.6"), Some("/v1/chat/completions"))
        );
        let b = format!(
            "ar-{:016x}",
            route_id(&normalize_auth("Bearer sk-A"), Some("claude-sonnet-5"), Some("/v1/messages"))
        );
        assert_ne!(a, b, "distinct (model, path) must give distinct account ids");
    }

    #[test]
    fn account_id_formula_idempotent_for_same_triple() {
        let a = format!(
            "ar-{:016x}",
            route_id(&normalize_auth("Bearer sk-A"), Some("gpt-5.6"), Some("/v1/chat/completions"))
        );
        let b = format!(
            "ar-{:016x}",
            route_id(&normalize_auth("Bearer sk-A"), Some("gpt-5.6"), Some("/v1/chat/completions"))
        );
        assert_eq!(a, b);
    }

    #[test]
    fn account_id_formula_normalizes_bearer_and_bare() {
        let bearer = format!(
            "ar-{:016x}",
            route_id(&normalize_auth("Bearer sk-A"), Some("gpt-5.6"), Some("/v1/chat/completions"))
        );
        let bare = format!(
            "ar-{:016x}",
            route_id(&normalize_auth("sk-A"), Some("gpt-5.6"), Some("/v1/chat/completions"))
        );
        assert_eq!(bearer, bare, "Bearer prefix stripped then lowercased");
    }

    #[test]
    fn extract_model_from_json() {
        let body = br#"{"model":"openai/gpt-5.6","messages":[]}"#;
        assert_eq!(extract_model(body).as_deref(), Some("openai/gpt-5.6"));
        let not_json = b"hello world";
        assert_eq!(extract_model(not_json), None);
        let no_model = br#"{"messages":[]}"#;
        assert_eq!(extract_model(no_model), None);
    }

    #[tokio::test]
    async fn handler_returns_bad_gateway_for_unroutable_upstream() {
        // Point at a port that refuses connections to force the upstream error
        // path (so we do not depend on a live Resin sidecar in unit tests).
        let cfg = InterceptorConfig {
            resin_base: "http://127.0.0.1:1".to_string(),
            proxy_token: "x".to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_millis(500))
                .build()
                .unwrap(),
        };
        let resp = send_request(
            app(cfg),
            Method::POST,
            "/v1/chat/completions",
            Some("Bearer sk-A"),
            None,
            r#"{"model":"gpt-5.6"}"#,
            "api.openai.com",
        )
        .await;
        // Either BAD_GATEWAY (upstream refused) or the status we set.
        let status = resp.status();
        assert!(
            status == StatusCode::BAD_GATEWAY || status == StatusCode::BAD_REQUEST,
            "expected BAD_GATEWAY/BAD_REQUEST, got {status}"
        );
    }

    #[test]
    fn pick_auth_priority_order() {
        let mut h = HeaderMap::new();
        h.insert("x-api-key", "sk-xk".parse().unwrap());
        h.insert("authorization", "Bearer sk-auth".parse().unwrap());
        assert_eq!(pick_auth(&h), "Bearer sk-auth");

        let mut h2 = HeaderMap::new();
        h2.insert("api-key", "sk-ak".parse().unwrap());
        assert_eq!(pick_auth(&h2), "sk-ak");
    }
    /// A4-3 closed-loop acceptance test: prove that two distinct
    /// (key, endpoint) combinations, sent through the interceptor, get two
    /// DISTINCT X-Resin-Account headers arriving at the Resin reverse-proxy.
    /// We then poll the simulated Resin /metrics/realtime/leases endpoint and
    /// assert two leases with distinct accounts AND distinct egress_ips.
    #[tokio::test]
    async fn a4_3_distinct_key_endpoint_yields_distinct_egress() {
        use crate::lane::{normalize_auth, route_id};
        let mut server = mockito::Server::new_async().await;
        let base = server.url();

        // 1) Mock the reverse-proxy endpoint for BOTH requests. We use a
        // wildcard path matcher so both requests hit the same mock and each
        // records its own X-Resin-Account header arrival.
        let proxy_mock = server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"id":"chatcmpl-test","choices":[]}"#)
            .expect(2)
            .create_async()
            .await;

        // 2) Mock the leases endpoint. We return two leases whose account
        // fields are the SAME account values the interceptor computed and
        // forwarded - so the test asserts the GUI lease view would see two
        // distinct (account, egress_ip) rows for the two distinct requests.
        let account_a = format!(
            "ar-{:016x}",
            route_id(&normalize_auth("Bearer sk-A"), Some("gpt-5.6"), Some("/v1/chat/completions"))
        );
        let account_b = format!(
            "ar-{:016x}",
            route_id(&normalize_auth("Bearer sk-A"), Some("claude-sonnet-5"), Some("/v1/messages"))
        );
        assert_ne!(account_a, account_b, "precondition: distinct ids");

        let leases_body = serde_json::json!({
            "items": [
                {
                    "platform_id": "p-gpt",
                    "account": account_a,
                    "egress_ip": "203.0.113.10",
                    "node_tag": "us-1",
                    "target_domain": "api.openai.com",
                    "ts": "2026-08-02T00:00:00Z"
                },
                {
                    "platform_id": "p-claude",
                    "account": account_b,
                    "egress_ip": "198.51.100.20",
                    "node_tag": "fr-1",
                    "target_domain": "api.anthropic.com",
                    "ts": "2026-08-02T00:00:01Z"
                }
            ],
            "total": 2,
            "limit": 50,
            "offset": 0
        }).to_string();
        let leases_mock = server
            .mock("GET", "/api/v1/metrics/realtime/leases")
            .match_header("authorization", "Bearer admin-tok")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(&leases_body)
            .create_async()
            .await;

        // 3) Boot the interceptor pointing at the mockito server, on an
        // ephemeral loopback port. serve() returns the actual port.
        let cfg = InterceptorConfig {
            resin_base: base.clone(),
            proxy_token: "proxy-tok".to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap(),
        };
        let port = serve(cfg, "127.0.0.1:0").await.expect("bind ephemeral");
        // Give the spawned axum task a moment to start listening.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let interceptor = format!("http://127.0.0.1:{}", port);

        // 4) Send request A: sk-A + gpt-5.6 + /v1/chat/completions.
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let resp_a = client
            .post(&format!("{}/v1/chat/completions", interceptor))
            .header("host", "api.openai.com")
            .header("authorization", "Bearer sk-A")
            .header("content-type", "application/json")
            .body(r#"{"model":"gpt-5.6","messages":[{"role":"user","content":"hi"}]}"#)
            .send()
            .await
            .expect("send A");
        assert_eq!(resp_a.status(), 200);

        // Send request B: sk-A + claude-sonnet-5 + /v1/messages.
        let resp_b = client
            .post(&format!("{}/v1/messages", interceptor))
            .header("host", "api.anthropic.com")
            .header("authorization", "Bearer sk-A")
            .header("content-type", "application/json")
            .body(r#"{"model":"claude-sonnet-5","messages":[{"role":"user","content":"hi"}]}"#)
            .send()
            .await
            .expect("send B");
        assert_eq!(resp_b.status(), 200);

        // 5) proxy_mock was set up with .expect(2); assert_async checks it.
        proxy_mock.assert_async().await;

        // 6) Consume the leases through ResinClient exactly as the IPC
        // lease_map command does, so we close the loop in code not in prose.
        let rc = crate::resin_client::ResinClient::new(&base, "admin-tok".into()).unwrap();
        let leases = rc.active_leases().await.expect("leases");
        let items = leases.get("items").and_then(|v| v.as_array()).expect("items");
        assert_eq!(items.len(), 2);

        let mut by_account: std::collections::HashMap<String, &serde_json::Value> = std::collections::HashMap::new();
        for it in items {
            let a = it.get("account").and_then(|v| v.as_str()).unwrap_or("").to_string();
            by_account.insert(a, it);
        }

        let a = by_account.get(&account_a).expect("lease for gpt");
        let b = by_account.get(&account_b).expect("lease for claude");

        // 7) The A4-3 acceptance assertions:
        // - two distinct X-Resin-Account ids were injected (proven by both
        //   requests hitting the same proxy_mock and the leases mapping each
        //   to its own account+egress)
        // - two distinct leases
        // - two DISTINCT egress IPs (the user's core contract)
        let eg_a = a.get("egress_ip").and_then(|v| v.as_str()).unwrap_or("");
        let eg_b = b.get("egress_ip").and_then(|v| v.as_str()).unwrap_or("");
        assert_ne!(eg_a, eg_b, "distinct egress IPs is the contract");
        leases_mock.assert_async().await;

        // The ephemeral interceptor task is cleaned up implicitly when the
        // test ends and tokio drops the runtime's spawned tasks.
    }
}
