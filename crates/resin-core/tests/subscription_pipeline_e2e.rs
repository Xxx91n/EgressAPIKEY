//! Integration test: the subscription establish pipeline end-to-end against
//! a mockito Resin (mock ResinClient seam — the Resin sidecar is NEVER
//! started; architecture-recovery Round 7 ticket 01 acceptance).
//!
//! The load-bearing assertion is ORDER: the wire must see
//! POST /subscriptions -> (GET subscriptions resolve) -> whitebox store
//! (no wire) -> POST /platforms -> PATCH /platforms/{id}. mockito counts
//! per-mock hits, so expect(1) on each write + assert() proves both the
//! order (first-mock-with-missing-hits rule drives the read phases) and
//! the exactly-once write discipline.

use resin_core::strategy_engine::{AClassStrategy, PlatformStrategy, StrategyConfig, StrategyId};
use resin_core::strategy_service::{FsStrategyStore, StrategyService};
use resin_core::subscription_pipeline::run_pipeline;
use serde_json::json;

fn seeded_config() -> StrategyConfig {
    StrategyConfig {
        platforms: vec![PlatformStrategy {
            platform_name: "e2e-sub".into(),
            a_class: AClassStrategy::Subscription,
            b_class: StrategyId::Random,
            manual_nodes: vec![],
            regions: vec![],
            subscriptions: vec!["e2e-sub".into()],
            top_n: 10,
            b_class_params: Default::default(),
        }],
        ..Default::default()
    }
}

#[tokio::test]
async fn e2e_sub_then_platform_then_apply_in_wire_order() {
    let mut server = mockito::Server::new_async().await;
    let base = server.url();
    let store_path = std::env::temp_dir().join(format!("sub-pipe-e2e-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&store_path);
    let svc = StrategyService::new(FsStrategyStore::new(store_path.clone()));

    // Step 1: list (absent) then create.
    let m_subs_absent = server
        .mock("GET", "/api/v1/subscriptions")
        .match_header("authorization", "Bearer testtok")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": []}).to_string())
        .expect(1)
        .create_async()
        .await;
    let m_create_sub = server
        .mock("POST", "/api/v1/subscriptions")
        .match_header("authorization", "Bearer testtok")
        .match_body(mockito::Matcher::PartialJson(json!({
            "name": "e2e-sub",
            "source_type": "remote",
            "url": "https://example.invalid/e2e.yaml",
            "update_interval": "30s"
        })))
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(json!({"id": "s1", "name": "e2e-sub"}).to_string())
        .expect(1)
        .create_async()
        .await;
    // Step 2 resolve: present with nodes.
    let m_subs_present = server
        .mock("GET", "/api/v1/subscriptions")
        .match_header("authorization", "Bearer testtok")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": [{"id": "s1", "name": "e2e-sub", "node_count": 4}]}).to_string())
        .expect(1)
        .create_async()
        .await;
    // Platform list reads, in wire order: (1) ensure_platform presence read
    // — absent; (2) apply initial live read + (3) apply per-platform
    // re-read — the row exists by then (created in step 3), so apply
    // diff-then-skips/PATCHes without re-creating.
    let m_platforms_absent = server
        .mock("GET", "/api/v1/platforms")
        .match_header("authorization", "Bearer testtok")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": []}).to_string())
        .expect(1)
        .create_async()
        .await;
    let m_create_platform = server
        .mock("POST", "/api/v1/platforms")
        .match_header("authorization", "Bearer testtok")
        .match_body(mockito::Matcher::PartialJson(json!({"name": "e2e-sub"})))
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(json!({"id": "p1", "name": "e2e-sub"}).to_string())
        .expect(1)
        .create_async()
        .await;
    // Step 5 apply: nodes land one HK node tagged with the subscription.
    let m_nodes = server
        .mock("GET", "/api/v1/nodes")
        .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
        .match_header("authorization", "Bearer testtok")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": [{"node_hash": "n1", "region": "HK", "has_outbound": true, "failure_count": 0, "tags": [{"subscription_name": "e2e-sub"}]}]}).to_string())
        .expect(1)
        .create_async()
        .await;
    // Post-create re-read by apply finds the platform.
    let m_platforms_created = server
        .mock("GET", "/api/v1/platforms")
        .match_header("authorization", "Bearer testtok")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": [{"id": "p1", "name": "e2e-sub", "region_filters": []}]}).to_string())
        .expect(2)
        .create_async()
        .await;
    let m_patch = server
        .mock("PATCH", "/api/v1/platforms/p1")
        .match_header("authorization", "Bearer testtok")
        .match_body(mockito::Matcher::PartialJson(json!({"region_filters": ["HK"]})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"id": "p1"}).to_string())
        .expect(1)
        .create_async()
        .await;

    let client = resin_core::ResinClient::new(&base, "testtok".into()).unwrap();
    let report = run_pipeline(&client, &svc, "e2e-sub", "https://example.invalid/e2e.yaml").await;
    assert!(report.all_ok(), "e2e pass must be green: {report:?}");

    // Whitebox side effects: one store entry, ref present, generation = 1.
    let cfg = svc.get().unwrap();
    assert_eq!(cfg.generation, 1);
    assert!(cfg.platforms.iter().any(|ps| ps.platform_name == "e2e-sub"
        && ps.a_class == AClassStrategy::Subscription
        && ps.subscriptions.contains(&"e2e-sub".to_string())));

    // Wire order + exactly-once writes: every write mock asserts its count,
    // and the phase mocks (absent -> created) prove the read order.
    m_subs_absent.assert_async().await;
    m_create_sub.assert_async().await;
    m_subs_present.assert_async().await;
    m_platforms_absent.assert_async().await;
    m_create_platform.assert_async().await;
    m_nodes.assert_async().await;
    m_platforms_created.assert_async().await;
    m_patch.assert_async().await;
    let _ = std::fs::remove_file(&store_path);
}

#[tokio::test]
async fn e2e_twice_second_pass_zero_writes() {
    let mut server = mockito::Server::new_async().await;
    let base = server.url();
    let store_path = std::env::temp_dir().join(format!("sub-pipe-e2e2-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&store_path);
    let svc = StrategyService::new(FsStrategyStore::new(store_path.clone()));
    // Seed the converged whitebox (pass 1 already ran in the previous test;
    // here both passes run against the same seeded whitebox).
    svc.store(seeded_config()).unwrap();

    // Converged world: sub present with nodes; platform live with the
    // region_filters the plan computes (HK); node pool one healthy HK node.
    let m_subs = server
        .mock("GET", "/api/v1/subscriptions")
        .match_header("authorization", "Bearer testtok")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": [{"id": "s1", "name": "e2e-sub", "node_count": 4}]}).to_string())
        .expect(4)
        .create_async()
        .await;
    let m_platforms = server
        .mock("GET", "/api/v1/platforms")
        .match_header("authorization", "Bearer testtok")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": [{"id": "p1", "name": "e2e-sub", "region_filters": ["HK"]}]}).to_string())
        .expect(6) // 3 platform GETs per pass x 2 passes
        .create_async()
        .await;
    let m_nodes = server
        .mock("GET", "/api/v1/nodes")
        .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
        .match_header("authorization", "Bearer testtok")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": [{"node_hash": "n1", "region": "HK", "has_outbound": true, "failure_count": 0, "tags": [{"subscription_name": "e2e-sub"}]}]}).to_string())
        .expect(2)
        .create_async()
        .await;

    let client = resin_core::ResinClient::new(&base, "testtok".into()).unwrap();
    let now: u64 = 1_700_000_000;
    let p = resin_core::SubscriptionPipeline::new();
    for pass in [1u64, 2] {
        assert!(p.enqueue(resin_core::EstablishEvent {
            subscription: "e2e-sub".into(),
            url: "https://example.invalid/e2e.yaml".into(),
        }));
        let reports = p.drain(&client, &svc, now + pass).await;
        assert_eq!(reports.len(), 1);
        assert!(reports[0].all_ok(), "pass {pass} must be green: {:?}", reports[0]);
    }
    // Both passes green, queue empty, generation unchanged since the seed.
    assert!(p.pending().is_empty());
    let cfg = svc.get().unwrap();
    assert_eq!(cfg.generation, 1, "neither pass may write the whitebox");

    m_subs.assert_async().await;
    m_platforms.assert_async().await;
    m_nodes.assert_async().await;
    let _ = std::fs::remove_file(&store_path);
}
