//! Integration test (architecture-recovery Round 7 ticket 03, spec D-C1.3):
//! the establish cascade ends with ONE socks5 entry port when the user did
//! not provide a binding target, and unbind releases it through the SAME
//! row-level mutation the existing ipcPortRemove path performs (Resin
//! endpoint delete + whitebox retain + whitebox apply) — no new IPC is
//! introduced; the port_remove command surface is untouched shell code.
//!
//! mock Resin via mockito (the sidecar is NEVER started); whitebox/DB are
//! REAL resin-core stores so the assertions run against the actual write
//! entry (validate -> SQLite -> listeners -> atomic JSON -> swap).

use resin_core::db::DbPool;
use resin_core::port_forwarder::PortForwarder;
use resin_core::resin_client::ResinClient;
use resin_core::strategy_service::{FsStrategyStore, StrategyService};
use resin_core::subscription_pipeline::{
    ensure_default_port, remove_default_port_if_orphaned, run_pipeline, StepStatus,
};
use resin_core::whitebox_config::{WhiteboxConfig, WhiteboxConfigStore};
use serde_json::json;

const BEARER: (&str, &str) = ("authorization", "Bearer testtok");

#[tokio::test]
async fn import_creates_one_socks5_port_and_unbind_releases_it() {
    let mut server = mockito::Server::new_async().await;

    // ---- establish cascade (steps 1-5), same wire shape as the main e2e ----
    let m_subs_absent = server
        .mock("GET", "/api/v1/subscriptions")
        .match_header(BEARER.0, BEARER.1)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": []}).to_string())
        .expect(1)
        .create_async()
        .await;
    let m_create_sub = server
        .mock("POST", "/api/v1/subscriptions")
        .match_header(BEARER.0, BEARER.1)
        .match_body(mockito::Matcher::PartialJson(json!({
            "name": "e2e-sub", "source_type": "remote",
            "url": "https://example.invalid/e2e.yaml", "update_interval": "30s"
        })))
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(json!({"id": "s1", "name": "e2e-sub"}).to_string())
        .expect(1)
        .create_async()
        .await;
    let m_subs_present = server
        .mock("GET", "/api/v1/subscriptions")
        .match_header(BEARER.0, BEARER.1)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": [{"id": "s1", "name": "e2e-sub", "node_count": 4}]}).to_string())
        .expect(2) // resolve read + apply's dangling-ref read
        .create_async()
        .await;
    let m_platforms_absent = server
        .mock("GET", "/api/v1/platforms")
        .match_header(BEARER.0, BEARER.1)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": []}).to_string())
        .expect(1)
        .create_async()
        .await;
    let m_create_platform = server
        .mock("POST", "/api/v1/platforms")
        .match_header(BEARER.0, BEARER.1)
        .match_body(mockito::Matcher::PartialJson(json!({"name": "e2e-sub"})))
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(json!({"id": "p1", "name": "e2e-sub"}).to_string())
        .expect(1)
        .create_async()
        .await;
    let m_nodes = server
        .mock("GET", "/api/v1/nodes")
        .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
        .match_header(BEARER.0, BEARER.1)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": [{"node_hash": "n1", "region": "HK", "has_outbound": true, "failure_count": 0, "tags": [{"subscription_name": "e2e-sub"}]}]}).to_string())
        .expect(1)
        .create_async()
        .await;
    let m_platforms_created = server
        .mock("GET", "/api/v1/platforms")
        .match_header(BEARER.0, BEARER.1)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"items": [{"id": "p1", "name": "e2e-sub", "region_filters": []}]}).to_string(),
        )
        .expect(2)
        .create_async()
        .await;
    let m_patch = server
        .mock("PATCH", "/api/v1/platforms/p1")
        .match_header(BEARER.0, BEARER.1)
        .match_body(mockito::Matcher::PartialJson(
            json!({"region_filters": ["HK"]}),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"id": "p1"}).to_string())
        .expect(1)
        .create_async()
        .await;

    // ---- step 6: default-port (the user provided NO binding target) ----
    let m_endpoints_empty = server
        .mock("GET", "/api/v1/endpoints")
        .match_header(BEARER.0, BEARER.1)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": []}).to_string())
        .expect(1)
        .create_async()
        .await;
    // The suggested port is a runtime property — pin the listener SHAPE.
    let m_create_endpoint = server
        .mock("POST", "/api/v1/endpoints")
        .match_header(BEARER.0, BEARER.1)
        .match_body(mockito::Matcher::PartialJson(
            json!({"allow_socks5": true, "allow_proxy": true, "allow_management": false}),
        ))
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(json!({"id": "ep-new"}).to_string())
        .expect(1)
        .create_async()
        .await;

    // ---- fixtures: real resin-core stores ----
    let store_path =
        std::env::temp_dir().join(format!("sub-default-port-e2e-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&store_path);
    let svc = StrategyService::new(FsStrategyStore::new(store_path.clone()));
    let db = DbPool::open_in_memory().unwrap();
    let forwarder = PortForwarder::new(db.clone(), "127.0.0.1", 1, "");
    let wb_path = std::env::temp_dir().join(format!(
        "sub-default-port-e2e-wb-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&wb_path);
    let whitebox = WhiteboxConfigStore::open(wb_path.clone(), WhiteboxConfig::from_ports(vec![]))
        .await
        .unwrap();

    let base = server.url();
    let client = ResinClient::new(&base, "testtok".into()).unwrap();

    // Cascade pass: import -> resolve -> platform -> apply (all green).
    let report = run_pipeline(&client, &svc, "e2e-sub", "https://example.invalid/e2e.yaml").await;
    assert!(report.all_ok(), "establish pass must be green: {report:?}");

    // Cascade end (D-C1.3): no binding target provided -> one default port.
    let status = ensure_default_port(&client, &db, &forwarder, &whitebox, "e2e-sub", None).await;
    assert!(
        matches!(status, StepStatus::Written),
        "default port must be created: {status:?}"
    );

    // "导入 sub 后 1 个入口端口可用": exactly ONE default-protocol (mixed,
    // round-8 D-007) entry bound to the platform, enabled, present in BOTH
    // the whitebox and its SQLite sync partner.
    let rows = db.list_ports().unwrap();
    assert_eq!(rows.len(), 1, "exactly one default port: {rows:?}");
    assert_eq!(
        rows[0].protocol,
        resin_core::entry_protocol::DEFAULT_ENTRY_PORT_PROTOCOL
    );
    assert_eq!(rows[0].platform_name, "e2e-sub");
    assert!(rows[0].enabled);
    assert!(rows[0].port >= resin_core::MIN_USER_PORT);
    assert_eq!(whitebox.snapshot().entry_ports.len(), 1);
    let created_port = rows[0].port;

    // Idempotence: a re-run of the green pass (re-added subscription)
    // skips — the platform already has its binding; zero writes.
    let again = ensure_default_port(&client, &db, &forwarder, &whitebox, "e2e-sub", None).await;
    assert!(matches!(again, StepStatus::AlreadyPresent), "{again:?}");
    assert_eq!(db.list_ports().unwrap().len(), 1, "no duplicate port");

    // ---- unbind: the ipcPortRemove path (no new IPC) ----
    // The command is thin shell plumbing over exactly these moves:
    // DELETE endpoint by id -> whitebox retain -> whitebox apply. The id
    // here is the create response's ("ep-new", the mock's static body); the
    // port->id list hop in the real command is already covered by the
    // find_endpoint_id_by_port unit tests in the shell crate.
    let m_delete = server
        .mock("DELETE", "/api/v1/endpoints/ep-new")
        .match_header(BEARER.0, BEARER.1)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"id": "ep-new"}).to_string())
        .expect(1)
        .create_async()
        .await;
    client.delete_endpoint("ep-new").await.unwrap();
    let mut next = whitebox.snapshot();
    next.entry_ports.retain(|row| row.port != created_port);
    whitebox.apply(&db, &forwarder, next).await.unwrap();

    assert!(
        db.list_ports().unwrap().is_empty(),
        "port released from the DB partner"
    );
    assert!(
        whitebox.snapshot().entry_ports.is_empty(),
        "port released from the whitebox"
    );

    m_subs_absent.assert_async().await;
    m_create_sub.assert_async().await;
    m_subs_present.assert_async().await;
    m_platforms_absent.assert_async().await;
    m_create_platform.assert_async().await;
    m_nodes.assert_async().await;
    m_platforms_created.assert_async().await;
    m_patch.assert_async().await;
    m_endpoints_empty.assert_async().await;
    m_create_endpoint.assert_async().await;
    m_delete.assert_async().await;
    let _ = std::fs::remove_file(&store_path);
    let _ = std::fs::remove_file(&wb_path);
}

// ---------------------------------------------------------------------------
// Orphan-cleanup e2e (architecture-recovery ticket 05, spec IMP-5 / A-008 /
// Round 9 D-007 option B): Mode B (engine) residue — the default-port tail's
// create-then-whitebox window and the un-bind on subscription_remove.
// ---------------------------------------------------------------------------

/// the (e)->(f) window: create_endpoint succeeded, whitebox.apply failed ->
/// the tail compensates by DELETE-ing the endpoint it just created (Mode B
/// only; Mode A never creates). The failure injection is a whitebox at
/// MAX_ENTRY_PORTS capacity — the next document exceeds the ceiling and
/// validate() rejects it at the same write entry the production path uses.
#[tokio::test]
async fn default_port_whitebox_failure_compensates_created_endpoint() {
    let mut server = mockito::Server::new_async().await;

    let m_endpoints_empty = server
        .mock("GET", "/api/v1/endpoints")
        .match_header(BEARER.0, BEARER.1)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": []}).to_string())
        .expect(1)
        .create_async()
        .await;
    let m_create_endpoint = server
        .mock("POST", "/api/v1/endpoints")
        .match_header(BEARER.0, BEARER.1)
        .match_body(mockito::Matcher::PartialJson(json!({"port": 24410})))
        .with_status(201)
        .with_header("content-type", "application/json")
        .with_body(json!({"id": "ep-orphan", "port": 24410}).to_string())
        .expect(1)
        .create_async()
        .await;
    // The compensation's fresh live read: mockito prefers the first
    // still-hungry mock, so this one-row list answers the second GET.
    let m_list_one = server
        .mock("GET", "/api/v1/endpoints")
        .match_header(BEARER.0, BEARER.1)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"items": [{"id": "ep-orphan", "port": 24410}]}).to_string())
        .expect(1)
        .create_async()
        .await;
    let m_delete_endpoint = server
        .mock("DELETE", "/api/v1/endpoints/ep-orphan")
        .match_header(BEARER.0, BEARER.1)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"id": "ep-orphan"}).to_string())
        .expect(1)
        .create_async()
        .await;

    let db = DbPool::open_in_memory().unwrap();
    let forwarder = PortForwarder::new(db.clone(), "127.0.0.1", 1, "");
    let wb_path = std::env::temp_dir().join(format!(
        "sub-orphan-comp-wb-{}-{}.json",
        std::process::id(),
        line!()
    ));
    let _ = std::fs::remove_file(&wb_path);
    let whitebox = WhiteboxConfigStore::open(wb_path.clone(), WhiteboxConfig::from_ports(vec![]))
        .await
        .unwrap();
    // Seed the whitebox at capacity (MAX_ENTRY_PORTS rows, none bound to
    // "orphan-sub", none on port 24410): steps (a)/(c) pass, but the
    // hand-built "next" document holds MAX_ENTRY_PORTS + 1 rows and
    // validate() rejects it at the write entry — the same failure class a
    // real capacity race hits.
    let seed: Vec<resin_core::db::PortMapping> = (0..resin_core::port_forwarder::MAX_ENTRY_PORTS
        as u16)
        .map(|i| resin_core::db::PortMapping {
            port: 20000 + i,
            protocol: "mixed".into(),
            platform_name: format!("seed-{i}"),
            account: format!("port-{}", 20000 + i),
            label: String::new(),
            enabled: true,
            auth_required: true,
        })
        .collect();
    whitebox
        .apply(&db, &forwarder, WhiteboxConfig::from_ports(seed))
        .await
        .unwrap();

    let base = server.url();
    let client = ResinClient::new(&base, "testtok".into()).unwrap();
    let status = ensure_default_port(
        &client,
        &db,
        &forwarder,
        &whitebox,
        "orphan-sub",
        Some(24410),
    )
    .await;
    assert!(
        matches!(status, StepStatus::Failed(_)),
        "injected whitebox failure: {status:?}"
    );
    // The write entry never accepted the row: the seed set survives and
    // no "orphan-sub" row landed anywhere.
    assert_eq!(
        whitebox.snapshot().entry_ports.len(),
        resin_core::port_forwarder::MAX_ENTRY_PORTS
    );
    assert!(!db
        .list_ports()
        .unwrap()
        .iter()
        .any(|r| r.platform_name == "orphan-sub"));

    m_endpoints_empty.assert_async().await;
    m_create_endpoint.assert_async().await;
    m_list_one.assert_async().await;
    m_delete_endpoint.assert_async().await;
    let _ = std::fs::remove_file(&wb_path);
}

/// subscription_remove's reverse tail: a default port bound to the deleted
/// subscription's platform is released through the SAME three-layer path
/// (whitebox retain -> apply -> Mode B Resin endpoint delete); rows bound to
/// OTHER platforms stay byte-identical; the id="default" read-only endpoint
/// is never touched.
#[tokio::test]
async fn remove_default_port_if_orphaned_releases_only_the_bound_default_port() {
    let mut server = mockito::Server::new_async().await;

    let m_list = server
        .mock("GET", "/api/v1/endpoints")
        .match_header(BEARER.0, BEARER.1)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"items": [
                {"id": "default", "port": 7890},
                {"id": "ep-keep", "port": 24401},
                {"id": "ep-orphan", "port": 24402}
            ]})
            .to_string(),
        )
        .expect(1)
        .create_async()
        .await;
    let m_delete = server
        .mock("DELETE", "/api/v1/endpoints/ep-orphan")
        .match_header(BEARER.0, BEARER.1)
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"id": "ep-orphan"}).to_string())
        .expect(1)
        .create_async()
        .await;

    let db = DbPool::open_in_memory().unwrap();
    let forwarder = PortForwarder::new(db.clone(), "127.0.0.1", 1, "");
    let wb_path = std::env::temp_dir().join(format!(
        "sub-orphan-rm-wb-{}-{}.json",
        std::process::id(),
        line!()
    ));
    let _ = std::fs::remove_file(&wb_path);
    let whitebox = WhiteboxConfigStore::open(wb_path.clone(), WhiteboxConfig::from_ports(vec![]))
        .await
        .unwrap();
    whitebox
        .apply(
            &db,
            &forwarder,
            WhiteboxConfig::from_ports(vec![
                resin_core::db::PortMapping {
                    port: 24401,
                    protocol: "mixed".into(),
                    platform_name: "other-platform".into(),
                    account: "port-24401".into(),
                    label: String::new(),
                    enabled: true,
                    auth_required: true,
                },
                resin_core::db::PortMapping {
                    port: 24402,
                    protocol: "mixed".into(),
                    platform_name: "gone-sub".into(),
                    account: "port-24402".into(),
                    label: String::new(),
                    enabled: true,
                    auth_required: true,
                },
            ]),
        )
        .await
        .unwrap();

    let base = server.url();
    let client = ResinClient::new(&base, "testtok".into()).unwrap();
    let removed =
        remove_default_port_if_orphaned(&client, &db, &forwarder, &whitebox, "gone-sub").await;
    assert!(removed, "the bound default port must be released");

    // port_list filter check: only the OTHER platform's row survives, in both
    // the SQLite partner (what port_list reads) and the whitebox file.
    let rows = db.list_ports().unwrap();
    assert_eq!(rows.len(), 1, "only the foreign row survives: {rows:?}");
    assert_eq!(rows[0].port, 24401);
    assert_eq!(rows[0].platform_name, "other-platform");
    let wb_rows = whitebox.snapshot().entry_ports;
    assert_eq!(wb_rows.len(), 1);
    assert_eq!(wb_rows[0].port, 24401);

    m_list.assert_async().await;
    m_delete.assert_async().await;
    let _ = std::fs::remove_file(&wb_path);
}

/// the idempotent no-op: nothing bound to the platform -> zero wire calls
/// (a dead-socket client proves it) and Ok(true).
#[tokio::test]
async fn remove_default_port_if_orphaned_no_binding_is_a_no_op() {
    let db = DbPool::open_in_memory().unwrap();
    let forwarder = PortForwarder::new(db.clone(), "127.0.0.1", 1, "");
    let wb_path =
        std::env::temp_dir().join(format!("sub-orphan-rm-noop-wb-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&wb_path);
    let whitebox = WhiteboxConfigStore::open(wb_path.clone(), WhiteboxConfig::from_ports(vec![]))
        .await
        .unwrap();
    // Dead-socket client: any wire call would fail the call.
    let client = ResinClient::new("http://127.0.0.1:1", "testtok".into()).unwrap();
    let removed =
        remove_default_port_if_orphaned(&client, &db, &forwarder, &whitebox, "stranger").await;
    assert!(removed, "no binding must still report success");
    assert!(db.list_ports().unwrap().is_empty());
    assert!(whitebox.snapshot().entry_ports.is_empty());
    let _ = std::fs::remove_file(wb_path);
}

/// Mode A: the shell owns the listeners; the reverse tail must not touch
/// Resin at all — the whitebox/DB release happens, no wire call happens
/// (dead-socket client + a live list mock that would fail the expect(0)).
#[tokio::test]
async fn remove_default_port_if_orphaned_mode_a_skips_resin_delete() {
    let mut server = mockito::Server::new_async().await;
    let _never = server
        .mock("GET", "/api/v1/endpoints")
        .match_header(BEARER.0, BEARER.1)
        .with_status(500)
        .expect(0)
        .create_async()
        .await;

    let db = DbPool::open_in_memory().unwrap();
    let forwarder = PortForwarder::shell(db.clone(), "127.0.0.1", 1, "");
    let wb_path = std::env::temp_dir().join(format!(
        "sub-orphan-rm-mode-a-wb-{}-{}.json",
        std::process::id(),
        line!()
    ));
    let _ = std::fs::remove_file(&wb_path);
    let whitebox = WhiteboxConfigStore::open(wb_path.clone(), WhiteboxConfig::from_ports(vec![]))
        .await
        .unwrap();
    whitebox
        .apply(
            &db,
            &forwarder,
            WhiteboxConfig::from_ports(vec![resin_core::db::PortMapping {
                port: 24403,
                protocol: "mixed".into(),
                platform_name: "mode-a-sub".into(),
                account: "port-24403".into(),
                label: String::new(),
                enabled: true,
                auth_required: true,
            }]),
        )
        .await
        .unwrap();

    let base = server.url();
    let client = ResinClient::new(&base, "testtok".into()).unwrap();
    let removed =
        remove_default_port_if_orphaned(&client, &db, &forwarder, &whitebox, "mode-a-sub").await;
    assert!(removed, "Mode A release succeeds without Resin");
    // The whitebox/DB release happened (the shell listener came down via
    // the whitebox apply); Resin was never contacted (the expect(0) tripwire
    // mock would have failed the test on any endpoints call).
    let rows = db.list_ports().unwrap();
    assert!(rows.is_empty(), "Mode A release clears the rows: {rows:?}");
    assert!(whitebox.snapshot().entry_ports.is_empty());
    _never.assert_async().await;
    let _ = std::fs::remove_file(&wb_path);
}
