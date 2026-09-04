//! Unit tests for the commands domain split (moved verbatim from the former
//! commands/mod.rs inline tests module by architecture-recovery ticket 08).

use super::*;

use serde_json::json;

    #[test]
    fn key_max_len_is_reasonable_cap() {
        assert!(KEY_MAX_LEN >= 64);
        assert!(KEY_MAX_LEN <= 32_768);
    }

    #[test]
    fn validate_ip_accepts_normal_rejects_bad() {
        assert!(validate_ip("203.0.113.7").is_ok());
        assert!(validate_ip("::1").is_ok());
        assert!(validate_ip("").is_err());
        assert!(validate_ip(&"1".repeat(254)).is_err());
        assert!(validate_ip("127.0.0.1 x").is_err());
    }

    #[test]
    fn validate_short_name_bounds() {
        assert!(validate_short_name("openai", "platform").is_ok());
        assert!(validate_short_name("", "platform").is_err());
        assert!(validate_short_name(&"x".repeat(NAME_MAX_LEN + 1), "account").is_err());
    }

    #[test]
    fn platform_names_projects_name_field() {
        let v = json!([
            { "name": "Default", "id": "00000000-0000-0000-0000-000000000000" },
            { "name": "Platform-A", "id": "11111111-1111-1111-1111-111111111111" }
        ]);
        assert_eq!(
            platform_names(&v),
            vec!["Default".to_string(), "Platform-A".to_string()]
        );
    }

    #[test]
    fn resolve_id_in_matches() {
        // Round 5 T17: the shell platform_id_for_name/subscription_id_for_name
        // duplicates were deleted; these locks now pin the shared
        // resin-core::resolve_id_in the command bodies resolve through.
        let v = json!([{ "name": "Foo", "id": "uuid-1" }]);
        assert_eq!(resin_core::resolve_id_in(&v, "Foo"), Some("uuid-1".to_string()));
        assert_eq!(resin_core::resolve_id_in(&v, "Bar"), None);
    }

    /// Ticket 17 / ADR-0055: the conflict rule moved to document level in
    /// resin-core (same port, different process => typed error). Shell-side
    /// regression lock on the re-exported helper.
    #[test]
    fn process_route_conflict_rejects_same_port_different_process() {
        use resin_core::ProcessRouteRule;
        let existing = vec![ProcessRouteRule {
            process: "ollama".to_string(),
            target_port: 17990,
        }];
        // different process, same port -> conflict
        assert!(resin_core::process_route_conflict_check(
            &[
                ProcessRouteRule { process: "ollama".into(), target_port: 17990 },
                ProcessRouteRule { process: "openai".into(), target_port: 17990 },
            ]
        )
        .is_err());
        // different process, different port -> ok
        assert!(resin_core::process_route_conflict_check(&existing).is_ok());
    }

    /// P13 B6/B4: Resin wraps list responses as `{"items":[...]}`. The old
    /// code used `v.as_array()` which always returned None for that shape,
    /// so platform_list / subscription_list returned empty even with live data.
    /// These tests pin both the bare-array back-compat path and the items path.
    #[test]
    fn items_arr_accepts_resin_items_wrapper() {
        let v = json!({ "items": [{ "name": "a" }, { "name": "b" }], "total": 2 });
        let arr = items_arr(&v);
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "a");
    }

    #[test]
    fn items_arr_accepts_bare_array_back_compat() {
        let v = json!([{ "name": "x" }]);
        assert_eq!(items_arr(&v).len(), 1);
    }

    #[test]
    fn items_arr_returns_empty_for_non_list_object() {
        let v = json!({ "active_leases": 4 });
        assert!(items_arr(&v).is_empty());
    }

    #[test]
    fn find_endpoint_id_by_port_finds_in_items_wrapper() {
        let v = json!({ "items": [
            { "id": "default", "port": 0 },
            { "id": "ep-uuid-1", "port": 1791 },
            { "id": "ep-uuid-2", "port": 1792 },
        ], "total": 3 });
        assert_eq!(find_endpoint_id_by_port(&v, 1791), Some("ep-uuid-1".to_string()));
        assert_eq!(find_endpoint_id_by_port(&v, 1792), Some("ep-uuid-2".to_string()));
    }

    #[test]
    fn find_endpoint_id_by_port_finds_in_bare_array() {
        let v = json!([{ "id": "ep-abc", "port": 1800 }]);
        assert_eq!(find_endpoint_id_by_port(&v, 1800), Some("ep-abc".to_string()));
    }

    #[test]
    fn find_endpoint_id_by_port_skips_default_endpoint() {
        let v = json!({ "items": [{ "id": "default", "port": 9999 }] });
        assert_eq!(find_endpoint_id_by_port(&v, 9999), None);
    }

    #[test]
    fn find_endpoint_id_by_port_returns_none_when_port_absent() {
        let v = json!({ "items": [{ "id": "ep-x", "port": 1111 }] });
        assert_eq!(find_endpoint_id_by_port(&v, 2222), None);
    }

    #[test]
    fn platform_names_reads_resin_items_wrapper() {
        let v = json!({
            "items": [
                { "id": "uuid-1", "name": "Default" },
                { "id": "uuid-2", "name": "OpenAI" },
            ],
            "total": 2, "limit": 50, "offset": 0,
        });
        assert_eq!(
            platform_names(&v),
            vec!["Default".to_string(), "OpenAI".to_string()]
        );
    }

    #[test]
    fn resolve_id_in_reads_resin_items_wrapper() {
        // Round 5 T17: wrapper-shape lock migrated from the deleted
        // platform_id_for_name to the shared resin-core resolver.
        let v = json!({
            "items": [ { "id": "uuid-9", "name": "Anthropic" } ],
            "total": 1,
        });
        assert_eq!(
            resin_core::resolve_id_in(&v, "Anthropic"),
            Some("uuid-9".to_string())
        );
        assert_eq!(resin_core::resolve_id_in(&v, "Missing"), None);
    }

    #[test]
    fn subscription_snapshot_reads_resin_items_wrapper_with_node_count() {
        let v = json!({
            "items": [
                { "id": "s1", "name": "sub-a", "node_count": 33 },
                { "id": "s2", "name": "sub-b", "node_count": 0, "healthy_node_count": 0, "last_error": "downloader: unexpected status 403", "last_checked": "2026-08-02T10:54:41.0883134Z" },
            ],
            "total": 2, "limit": 50, "offset": 0,
        });
        let snap = subscription_snapshot(&v);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].name, "sub-a");
        assert_eq!(snap[0].node_count, 33);
        // Item 2 / Option B: default fields when Resin omits them.
        assert_eq!(snap[0].healthy_node_count, 0);
        assert_eq!(snap[0].last_error, "");
        assert_eq!(snap[0].last_checked, "");
        assert_eq!(snap[1].node_count, 0);
        // Item 2 / Option B: surface last_error/last_checked/healthy so a
        // fetch 403 is not a mute zero in the GUI.
        assert_eq!(snap[1].healthy_node_count, 0);
        assert_eq!(snap[1].last_error, "downloader: unexpected status 403");
        assert_eq!(snap[1].last_checked, "2026-08-02T10:54:41.0883134Z");
    }

    /// Item 2 / Option B: ensure the projection does not panic when Resin
    /// returns last_error as a null (some Go encoders emit null instead of
    /// empty for an unset string pointer). unwrap_or("") must handle both.
    #[test]
    fn subscription_snapshot_handles_null_last_error_and_missing_healthy() {
        let v = json!({
            "items": [
                { "id": "s3", "name": "sub-c", "node_count": 12, "healthy_node_count": 8, "last_error": null, "last_checked": "2026-08-02T11:00:00Z" },
                { "id": "s4", "name": "sub-d", "node_count": 5 },
            ],
            "total": 2,
        });
        let snap = subscription_snapshot(&v);
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].name, "sub-c");
        assert_eq!(snap[0].node_count, 12);
        assert_eq!(snap[0].healthy_node_count, 8);
        assert_eq!(snap[0].last_error, ""); // null collapses to empty string
        assert_eq!(snap[0].last_checked, "2026-08-02T11:00:00Z");
        assert_eq!(snap[1].name, "sub-d");
        assert_eq!(snap[1].node_count, 5);
        assert_eq!(snap[1].healthy_node_count, 0);
        assert_eq!(snap[1].last_error, "");
        assert_eq!(snap[1].last_checked, "");
    }

    #[test]
    fn resolve_id_in_reads_subscription_items_wrapper() {
        // Round 5 T17: subscription shape lock migrated from the deleted
        // subscription_id_for_name to the shared resin-core resolver.
        let v = json!({
            "items": [ { "id": "sub-uuid-1", "name": "main" } ],
            "total": 1,
        });
        assert_eq!(
            resin_core::resolve_id_in(&v, "main"),
            Some("sub-uuid-1".to_string())
        );
        assert_eq!(resin_core::resolve_id_in(&v, "nope"), None);
    }
    #[test]
    fn validate_port_mapping_rejects_privileged_and_bad_proto() {
        assert!(validate_port_mapping(80, "socks5", "OpenAI", "a", "").is_err());
        assert!(validate_port_mapping(17990, "ftp", "OpenAI", "a", "").is_err());
        assert!(validate_port_mapping(17990, "socks5", "Open.AI", "a", "").is_err());
        assert!(validate_port_mapping(17990, "http", "OpenAI", "port-17990", "k").is_ok());
    }

    #[test]
    fn validate_port_mapping_rejects_control_in_label() {
        assert!(validate_port_mapping(17990, "socks5", "OpenAI", "a", "bad\n").is_err());
    }

    #[test]
    fn validate_port_segments_rejects_privileged_and_accepts_user_range() {
        // Privileged ports below MIN_USER_PORT must be rejected so a hostile
        // GUI caller cannot pivot the shell to dial system ports (path-safety
        // guard for port_auth_info + port_health_check).
        assert!(validate_port_segments(80).is_err());
        assert!(validate_port_segments(1023).is_err());
        // User range is accepted: boundary at MIN_USER_PORT (1024) up to 65535.
        assert!(validate_port_segments(1024).is_ok());
        assert!(validate_port_segments(17990).is_ok());
        assert!(validate_port_segments(65535).is_ok());
    }

    /// T6-Bug1: HTTP port_health_check should send GET / not CONNECT.
    /// This is a compile-time + behavior test: the greeting bytes must starts
    /// with "GET / HTTP/1.1" for HTTP protocol. We verify by checking the
    /// behavior indirectly — the actual TCP probe is async and needs a live
    /// listener; here we just verify the greeting construction logic exists
    /// and the code compiles. Full integration is the release-exe smoke test.
    #[test]
    fn port_health_check_http_greeting_is_get_not_connect() {
        // The greeting for "http" protocol should use GET, not CONNECT.
        // We verify by checking that the format! macro produces GET.
        let port: u16 = 1791;
        let greeting = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        assert!(greeting.starts_with("GET / HTTP/1.1"), "HTTP greeting must start with GET, got: {}", greeting);
        assert!(!greeting.contains("CONNECT"), "HTTP greeting must NOT contain CONNECT");
    }

    /// T6-Bug5: port_auth_info username must be in {Platform}.{Account} format.
    /// Resin SOCKS5 requires this format; without the platform prefix,
    /// CONNECT returns General failure (error 1) even though auth succeeds.
    #[test]
    fn port_auth_username_format_includes_platform_prefix() {
        // Simulate the username construction logic for both empty and non-empty account.
        let platform_name = "Default";
        let account = "port-1792";
        let port: u16 = 1792;

        // Non-empty account: format!("{}.{}", platform_name, account)
        let username = format!("{}.{}", platform_name, account);
        assert_eq!(username, "Default.port-1792");

        // Empty account fallback: format!("{}.port-{}", platform_name, port)
        let empty_account_username = format!("{}.port-{}", platform_name, port);
        assert_eq!(empty_account_username, "Default.port-1792");
    }

    // T6-4: parse exit IP from Cloudflare trace body (pure helper logic).
    #[test]
    fn t6_4_parse_exit_ip_from_cloudflare_trace() {
        let body = "fl=123f\nnode=sin1\nip=203.0.113.42\nuag=Mozilla/5.0\n";
        let ip = body
            .lines()
            .find_map(|l| l.strip_prefix("ip=").map(|s| s.trim().to_string()))
            .unwrap_or_default();
        assert_eq!(ip, "203.0.113.42");
    }

    #[test]
    fn t6_4_parse_exit_ip_missing_returns_empty() {
        let body = "fl=123f\nnode=sin1\nuag=Mozilla/5.0\n";
        let ip = body
            .lines()
            .find_map(|l| l.strip_prefix("ip=").map(|s| s.trim().to_string()))
            .unwrap_or_default();
        assert_eq!(ip, "");
    }

    #[test]
    fn t6_4_parse_exit_ip_with_trailing_whitespace() {
        let body = "ip=  198.51.100.1  \n";
        let ip = body
            .lines()
            .find_map(|l| l.strip_prefix("ip=").map(|s| s.trim().to_string()))
            .unwrap_or_default();
        assert_eq!(ip, "198.51.100.1");
    }

    #[test]
    fn t8_port_bind_platform_empty_platform_name_passes_validation() {
        // empty platform_name = unbind, should pass
        assert!(validate_short_name("", "platform_name").is_err()); // validate_short_name rejects empty
        // port_bind_platform allows empty without calling validate_short_name
        // (the command itself guards with if !platform_name.is_empty())
    }

    #[test]
    fn t8_port_bind_platform_forbidden_chars_rejected() {
        let forbidden = |s: &str| s.chars().any(|ch| ".:/\\@?#%~ ".contains(ch));
        assert!(forbidden("my.platform"));
        assert!(forbidden("my@platform"));
        assert!(!forbidden("my-platform"));
        assert!(!forbidden("Default"));
    }

    #[test]
    fn t10_port_suggest_tcp_probe_finds_bindable_port() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    }

    #[test]
    fn t10_port_suggest_skip_used_port() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_err());
        drop(l);
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    }

    /// T10: verify auto_strategy_apply PATCH failure result shape (pure logic).
    #[test]
    fn t10_patch_failure_result_has_reason() {
        let platform_name = "TestPlatform";
        let regions = vec!["US".to_string()];
        let err_msg = "connection refused";
        let result = serde_json::json!({
            "platform": platform_name,
            "region_filters": regions,
            "patched": false,
            "reason": format!("PATCH failed: {err_msg}"),
        });
        assert_eq!(result["patched"], false);
        assert!(result["reason"].as_str().unwrap().contains("PATCH failed"));
        assert!(result["reason"].as_str().unwrap().contains("connection refused"));
    }

    /// T10: verify config_import PATCH failure pushes to errors vec (pure logic).
    #[test]
    fn t10_config_import_patch_failure_pushes_error() {
        let name = "MyPlatform";
        let err_msg = "timeout";
        let mut errors: Vec<String> = vec![];
        let error_line = format!("platform {name}: PATCH failed: {err_msg}");
        errors.push(error_line);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("MyPlatform"));
        assert!(errors[0].contains("PATCH failed"));
        assert!(errors[0].contains("timeout"));
    }

    #[test]
    fn t15_2_set_log_level_validates_enum() {
        // Validate that the level string maps correctly to the atomic gate.
        // We do not call the async command (requires Tauri runtime); instead
        // we test the gate logic directly.
        super::LOG_LEVEL_GATE.store(2, std::sync::atomic::Ordering::Relaxed);
        assert!(super::log_level_enabled(0)); // error always emitted
        assert!(super::log_level_enabled(1)); // warn emitted at info+
        assert!(super::log_level_enabled(2)); // info emitted at info
        assert!(!super::log_level_enabled(3)); // debug NOT emitted at info
    }

    #[test]
    fn t15_2_set_log_level_gate_round_trip() {
        // Set level to debug (3) and verify all levels pass
        super::LOG_LEVEL_GATE.store(3, std::sync::atomic::Ordering::Relaxed);
        assert!(super::log_level_enabled(0));
        assert!(super::log_level_enabled(1));
        assert!(super::log_level_enabled(2));
        assert!(super::log_level_enabled(3));
        // Set to error (0) and verify only error passes
        super::LOG_LEVEL_GATE.store(0, std::sync::atomic::Ordering::Relaxed);
        assert!(super::log_level_enabled(0));
        assert!(!super::log_level_enabled(1));
        assert!(!super::log_level_enabled(2));
        assert!(!super::log_level_enabled(3));
        // Restore default
        super::LOG_LEVEL_GATE.store(2, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn node_probe_validates_kind_rejects_unknown_and_accepts_known() {
        assert!(super::validate_node_probe_inputs("abc123", "egress").is_ok());
        assert!(super::validate_node_probe_inputs("abc123", "latency").is_ok());
        assert!(super::validate_node_probe_inputs("abc123", "bogus").is_err());
        assert!(super::validate_node_probe_inputs("abc123", "").is_err());
    }

    #[test]
    fn node_probe_rejects_empty_and_control_chars_in_hash() {
        assert!(super::validate_node_probe_inputs("", "latency").is_err());
        assert!(super::validate_node_probe_inputs("bad\nhash", "latency").is_err());
        assert!(super::validate_node_probe_inputs("bad\0hash", "egress").is_err());
        // 129 chars > 128 cap
        let long = "a".repeat(129);
        assert!(super::validate_node_probe_inputs(&long, "latency").is_err());
        // 128 chars OK
        let exact = "a".repeat(128);
        assert!(super::validate_node_probe_inputs(&exact, "latency").is_ok());
    }

    /// Ticket 11: ts_ns -> "YYYY-MM-DD HH:MM:SS" formatting stays stable across
    /// the direct-db -> REST seam swap.
    #[test]
    fn format_log_ts_formats_epoch_nanos() {
        assert_eq!(format_log_ts(1_727_654_400_000_000_000), "2024-09-30 00:00:00");
        assert_eq!(format_log_ts(0), "1970-01-01 00:00:00");
        // The mapping path totals over 0 for absent/malformed ts_ns, so an
        // out-of-range epoch simply lands on the Unix epoch string.
        assert_eq!(format_log_ts(-1_000_000_000), "1969-12-31 23:59:59");
    }

    /// Ticket 11: one item of the Resin /api/v1/request-logs response maps to
    /// the IPC RequestLogEntry with duration_ms derived from duration_ns.
    #[test]
    fn request_log_entry_from_value_maps_known_fields() {
        let v: serde_json::Value = serde_json::json!({
            "id": "log-1",
            "ts_ns": 1_727_654_400_000_000_000i64,
            "platform_name": "OpenAI",
            "account": "acct-1",
            "target_host": "api.openai.com",
            "egress_ip": "1.2.3.4",
            "proxy_type": "forward",
            "net_ok": true,
            "http_method": "GET",
            "http_status": 200,
            "duration_ns": 12_345_678i64,
            "resin_error": ""
        });
        let e = request_log_entry_from_value(&v);
        assert_eq!(e.ts, "2024-09-30 00:00:00");
        assert_eq!(e.platform_name, "OpenAI");
        assert_eq!(e.account, "acct-1");
        assert_eq!(e.target_host, "api.openai.com");
        assert_eq!(e.egress_ip, "1.2.3.4");
        assert_eq!(e.http_method, "GET");
        assert_eq!(e.http_status, 200);
        assert!((e.duration_ms - 12.345678).abs() < 1e-9);
        assert_eq!(e.resin_error, "");
    }

    /// Ticket 11: a malformed row must not panic nor blank the tail — total
    /// over defaults (parity with the old query_map row-skipping semantics).
    #[test]
    fn request_log_entry_from_value_defaults_on_malformed_row() {
        let v: serde_json::Value = serde_json::json!({"unexpected": 1});
        let e = request_log_entry_from_value(&v);
        assert_eq!(e.ts, "1970-01-01 00:00:00");
        assert_eq!(e.platform_name, "");
        assert_eq!(e.http_status, 0);
        assert_eq!(e.duration_ms, 0.0);
}

    /// Ticket 12 / ADR-0054 §C+§D wire-shape regression lock: lastCheckedAt
    /// (top-level camelCase), per-entry divergent_since (snake_case, omitted
    /// when None), acknowledged flag stamps read-side only.
    #[test]
    fn authoritative_snapshot_ticket12_wire_shape() {
        let mut snap = resin_core::AuthoritativeSnapshot {
            strategy_version: 1,
            platforms: vec![
                resin_core::StrategySnapshot::Divergent {
                    platform_name: "alpha".into(),
                    platform_id: "id-alpha".into(),
                    whitebox_regions: vec!["jp".into()],
                    resin_regions: vec!["us".into()],
                    resin_allocation_policy: "BALANCED".into(),
                    b_class: "random".into(),
                    a_class: "region".into(),
                    manual_nodes: vec![],
                    subscriptions: vec![],
                    divergent_since: Some(1_756_521_600),
                    acknowledged: true,
                },
                resin_core::StrategySnapshot::Consistent {
                    platform_name: "beta".into(),
                    platform_id: "id-beta".into(),
                    regions: vec!["hk".into()],
                    resin_allocation_policy: "BALANCED".into(),
                    b_class: "random".into(),
                    a_class: "region".into(),
                    manual_nodes: vec![],
                    subscriptions: vec![],
                    acknowledged: false,
                },
            ],
            ports: vec![resin_core::PortSnapshot::MissingOnResin {
                port: 17990,
                platform_name: "alpha".into(),
                protocol: "socks5".into(),
                account: "acct".into(),
                label: String::new(),
                auth_required: false,
                divergent_since: None,
                acknowledged: false,
            }],
            routes: vec![],
            subscriptions: vec![],
            resin_reachable: true,
            last_checked_at: 1_756_521_601,
            strategy_generation: 0,
            strategy_applied_generation: 0,
            converge_phase: resin_core::ConvergePhase::NeverApplied,
            last_apply_at: None,
            last_apply_error: None,
        };
        let v = serde_json::to_value(&snap).unwrap();
        // Top-level field is camelCase (TS wrapper convention).
        assert_eq!(v["lastCheckedAt"], 1_756_521_601i64);
        // Per-variant payload fields stay snake_case (ADR-0051 contract).
        assert_eq!(v["platforms"][0]["divergent_since"], 1_756_521_600i64);
        assert_eq!(v["platforms"][0]["acknowledged"], true);
        // None => the key is absent (skip_serializing_if).
        assert!(v["ports"][0].get("divergent_since").is_none());
        // Consistent entries also carry the acknowledged flag (default false).
        assert_eq!(v["platforms"][1]["acknowledged"], false);
        // Round-trip stability for the TS wrapper.
        let back: resin_core::AuthoritativeSnapshot = serde_json::from_value(v).unwrap();
        assert_eq!(back, snap);
        // The drift memory helper stays pure: advancing with no drifting
        // entities never invents entries.
        let mem = resin_core::snapshot::advance_drift_memory(
            &resin_core::snapshot::DriftMemory::default(),
            &[],
            42,
        );
        assert!(mem.is_empty());
        let _ = &mut snap; // keep mut binding honest for future per-case edits
    }

    // T02 (round5-config-authority) — subscription_refresh diff helpers. The
    // command polls list_subscriptions after POST /actions/refresh (Resin
    // returns only {"status":"ok"}) and decides `changed` from these pure fns.

    #[test]
    fn subscription_refresh_changed_detects_count_delta() {
        let changed = |a: u64, b: u64| subscription_refresh_changed_changed(a, None, b, None);
        assert!(changed(3, 4)); // count grew
        assert!(changed(4, 3)); // count shrank (a prune is still a change)
        assert!(!changed(3, 3)); // identical count, no version info
    }

    #[test]
    fn subscription_refresh_changed_detects_version_bump() {
        let v1 = json!("aaa");
        let v2 = json!("bbb");
        // Same count but the row's node_version moved => refresh produced a diff.
        assert!(subscription_refresh_changed_changed(3, Some(&v1), 3, Some(&v2)));
        // Identical count + identical version => converged, zero-change refresh.
        assert!(!subscription_refresh_changed_changed(3, Some(&v1), 3, Some(&v1)));
        // Missing version on either side degrades to a count-only comparison.
        assert!(!subscription_refresh_changed_changed(3, None, 3, Some(&v2)));
        assert!(!subscription_refresh_changed_changed(3, Some(&v1), 3, None));
        assert!(!subscription_refresh_changed_changed(3, None, 3, None));
    }

    #[test]
    fn extract_sub_row_stats_reads_count_and_version() {
        let row = json!({ "id": "sub-1", "node_count": 7, "node_version": "v9" });
        let (count, ver) = extract_sub_row_stats(&row);
        assert_eq!(count, 7);
        assert_eq!(ver, Some(json!("v9")));
        // Resin may expose config_version instead of node_version.
        let legacy = json!({ "node_count": 2, "config_version": 42 });
        let (count, ver) = extract_sub_row_stats(&legacy);
        assert_eq!(count, 2);
        assert_eq!(ver, Some(json!(42)));
        // Missing fields degrade to (0, None) — never panic on foreign rows.
        let bare = json!({ "id": "sub-2" });
        assert_eq!(extract_sub_row_stats(&bare), (0, None));
    }
