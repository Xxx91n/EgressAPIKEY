//! Subscription establish pipeline (architecture-recovery Round 7 ticket 01,
//! spec D-C1.1) — the between-steps glue the ADR lattice implies but never
//! wired: import (POST /subscriptions) no longer stops at "data landed".
//!
//! Mental model (spec §9 R-A research verdict): a LEVEL-TRIGGERED desired-
//! state reconciler in the K8s controller tradition, NOT an event-driven
//! callback cascade. The caller enqueues a DesiredEstablish event; a SINGLE
//! drain loop walks the five steps in order; every step is IDEMPOTENT (it
//! re-reads the live world and only writes what is missing — re-running a
//! converged pipeline emits zero writes). There is no background loop and no
//! auto-heal: the pipeline runs when enqueued (and on explicit retry), which
//! keeps the ADR-0054 "reconcile is user-triggered" discipline intact —
//! enqueueing IS the user action here (the subscription_add call that asked
//! for pipeline=establish).
//!
//! The five steps and their per-step idempotence seam:
//! 1. create_subscription — skip when the name is already on Resin (list read).
//! 2. resolve          — the subscription exists AND its nodes arrived
//!                       (node_count > 0). This is a pure read; failure here
//!                       means Resin's own fetcher has not landed nodes yet.
//! 3. establish platform — ADR-0056: ensure the whitebox has an
//!                       a_class=subscription platform named after the
//!                       subscription (bump generation through the Service
//!                       store entry), then create it on Resin when missing.
//! 4. strategy_config_put — folded into step 3's whitebox write (the sub name
//!                       is appended to the platform's subscriptions list in
//!                       the SAME store entry; a second run sees the ref
//!                       present and writes nothing).
//! 5. strategy_apply   — the existing diff-then-skip apply (ADR-0057) +
//!                       generation write-back (ADR-0058). In-sync = zero
//!                       PATCH.
//!
//! Terminal criterion (spec D-C1.1): snapshot consumed_by non-empty AND the
//! target platform not missing on Resin AND ConvergePhase in
//! {Converged, Drifted(acknowledged)}. `is_terminal_state` is a pure fn so
//! the check is unit-testable without Resin.
//!
//! Failure model (D-C1.1 allow clause): a failed step is PERSISTENT STATE —
//! the event stays queued with attempts + last_error and is retried by the
//! caller with exponential backoff (`retry_delay`), capped at
//! MAX_ATTEMPTS. This ticket does NOT compensate partial failure (T04 owns
//! rollback) and does NOT own UI phase state (T02 owns SubscriptionPhase).

use std::collections::VecDeque;
use std::sync::Mutex as StdMutex;

use serde::{Deserialize, Serialize};

use crate::resin_client::ResinClient;
use crate::strategy::StrategyId;
use crate::strategy_engine::{AClassStrategy, PlatformStrategy};
use crate::strategy_service::{StrategyConfigStore, StrategyService};
use crate::snapshot::ConvergePhase;

/// Retry budget per event. Exponential backoff base (seconds): 2, 4, 8, 16…
/// capped by `retry_delay`. After MAX_ATTEMPTS the event parks in Failed
/// state (persistent, visible to the caller) instead of looping forever.
pub const MAX_ATTEMPTS: u32 = 5;

/// Bounded queue ceiling (AGENTS 7.5 bounded-collection template): a hostile
/// or buggy caller cannot grow memory unbounded.
pub const MAX_QUEUE: usize = 64;

/// A desired establish request, name-keyed. Level-triggered: enqueueing the
/// same name twice coalesces (the queue keeps one entry per subscription).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EstablishEvent {
    /// Subscription name (validated by the IPC layer before enqueue).
    pub subscription: String,
    /// Remote subscription URL (step 1 re-POSTs only when the name is NOT
    /// already on Resin; an already-present subscription never needs it).
    pub url: String,
}

/// Per-step execution outcome for one drain pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// The step wrote something this pass.
    Written,
    /// The step found the desired state already present (idempotent skip).
    AlreadyPresent,
    /// The step failed; `reason` carries the KEP-1623-style message.
    Failed(String),
}

impl StepStatus {
    pub fn is_failed(&self) -> bool {
        matches!(self, StepStatus::Failed(_))
    }
}

/// Outcome of one full five-step pass for one subscription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineReport {
    pub subscription: String,
    pub steps: [StepStatus; 5],
}

impl PipelineReport {
    /// True when every step either wrote or skipped — the pipeline is done.
    pub fn all_ok(&self) -> bool {
        self.steps.iter().all(|s| !s.is_failed())
    }

    /// The first failure reason, if any.
    pub fn first_error(&self) -> Option<&str> {
        self.steps.iter().find_map(|s| match s {
            StepStatus::Failed(reason) => Some(reason.as_str()),
            _ => None,
        })
    }
}

/// Queue entry state (persistent for the process lifetime; the queue is
/// process-local memory like DriftMemory — a restart clears it and the next
/// enqueue re-runs, which is honest for a level-triggered design).
#[derive(Debug, Clone, PartialEq, Eq)]
struct QueueEntry {
    subscription: String,
    url: String,
    attempts: u32,
    last_error: Option<String>,
    /// Unix seconds of the earliest next attempt (backoff schedule).
    next_retry_at: u64,
    /// PARKED after MAX_ATTEMPTS failures: stays visible, never retried.
    failed: bool,
}

/// The single-writer event queue + reconciler driver. Enqueue is the ONLY
/// mutation entry; `drain` is the single reconciler loop body (called by
/// the shell after enqueue — no background task owns it).
#[derive(Default)]
pub struct SubscriptionPipeline {
    queue: StdMutex<VecDeque<QueueEntry>>,
}

impl SubscriptionPipeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Level-triggered enqueue: coalesces per subscription name (a second
    /// event for an already-queued subscription refreshes nothing — one
    /// entry per name). Returns false when the queue is full (bounded).
    /// A FAILED (parked) entry is re-armed by an explicit re-enqueue: the
    /// user acting again resets the attempt budget (a fresh user action is
    /// a fresh reconcile, not a silent auto-retry).
    pub fn enqueue(&self, event: EstablishEvent) -> bool {
        let mut q = self.queue.lock().expect("pipeline queue poisoned");
        if let Some(pos) = q.iter().position(|e| e.subscription == event.subscription) {
            q.remove(pos);
        }
        if q.len() >= MAX_QUEUE {
            return false;
        }
        q.push_back(QueueEntry {
            subscription: event.subscription,
            url: event.url,
            attempts: 0,
            last_error: None,
            next_retry_at: 0,
            failed: false,
        });
        true
    }

    /// Snapshot of pending names (oldest first) — diagnostics / tests.
    pub fn pending(&self) -> Vec<String> {
        let q = self.queue.lock().expect("pipeline queue poisoned");
        q.iter().map(|e| e.subscription.clone()).collect()
    }

    /// How many events are parked in the terminal Failed state.
    pub fn failed_count(&self) -> usize {
        let q = self.queue.lock().expect("pipeline queue poisoned");
        q.iter().filter(|e| e.failed).count()
    }

    /// The backoff schedule: attempts n (0-based) waits BACKOFF_BASE_SECS *
    /// 2^n seconds, capped at BACKOFF_CAP_SECS. Pure — unit-tested.
    fn retry_delay(attempts_so_far: u32) -> u64 {
        const CAP: u64 = 60;
        (2u64 << attempts_so_far.min(5)).min(CAP)
    }

    /// Single reconciler: drain every due event, one pass. Steps run in
    /// order; a failed step records the error and parks the event for
    /// backoff retry (or Failed after MAX_ATTEMPTS). Returns one report per
    /// processed event. `now` is injected so tests own the clock.
    pub async fn drain(
        &self,
        client: &ResinClient,
        svc: &StrategyService<crate::strategy_service::FsStrategyStore>,
        now: u64,
    ) -> Vec<PipelineReport> {
        let due: Vec<QueueEntry> = {
            let mut q = self.queue.lock().expect("pipeline queue poisoned");
            let mut due = Vec::new();
            q.retain(|e| {
                if e.failed || e.next_retry_at > now {
                    true // keep parked / not-yet-due entries
                } else {
                    due.push(e.clone());
                    false // removed: either re-queued below or completed
                }
            });
            due
        };
        let mut reports = Vec::with_capacity(due.len());
        for event in due {
            let subscription = event.subscription.clone();
            let report = run_pipeline(client, svc, &subscription, &event.url).await;
            match report.first_error() {
                None => {
                    // Converged: the event is DONE — it stays out of the queue.
                }
                Some(reason) => {
                    let mut entry = event;
                    entry.attempts += 1;
                    entry.last_error = Some(reason.to_string());
                    if entry.attempts >= MAX_ATTEMPTS {
                        entry.failed = true;
                        tracing::warn!(subscription = %subscription, attempts = entry.attempts, "subscription_pipeline: parked after max attempts");
                    } else {
                        entry.next_retry_at = now + Self::retry_delay(entry.attempts - 1);
                    }
                    let mut q = self.queue.lock().expect("pipeline queue poisoned");
                    q.push_back(entry);
                }
            }
            reports.push(report);
        }
        reports
    }
}

/// Accept Resin's items-wrapper shape `{"items":[...]}` OR a bare array —
/// the same tolerance as the shell-side `items_arr` and strategy_service's
/// private `items` (kept module-local: no new pub surface on strategy_service).
fn items_of(v: &serde_json::Value) -> &[serde_json::Value] {
    if let Some(arr) = v.get("items").and_then(|i| i.as_array()) {
        return arr.as_slice();
    }
    if let Some(arr) = v.as_array() {
        return arr.as_slice();
    }
    &[]
}

/// Step 1: POST /subscriptions — skip when the name already exists on Resin
/// (per-step idempotence; the name-keyed overwrite hazard from P20 item 3
/// makes re-POSTing a real hazard, not just a wasted write).
pub async fn ensure_subscription(
    client: &ResinClient,
    name: &str,
    url: &str,
) -> StepStatus {
    let live = match client.list_subscriptions().await {
        Ok(v) => v,
        Err(e) => return StepStatus::Failed(format!("list subscriptions: {e}")),
    };
    let items = items_of(&live);
    if items.iter().any(|s| s.get("name").and_then(|n| n.as_str()) == Some(name)) {
        return StepStatus::AlreadyPresent;
    }
    let body = serde_json::json!({
        "name": name,
        "source_type": "remote",
        "url": url,
        "update_interval": "30s",
    });
    match client.create_subscription(body).await {
        Ok(_) => StepStatus::Written,
        Err(e) => StepStatus::Failed(format!("create subscription: {e}")),
    }
}

/// Step 2: resolve — the subscription exists and its nodes have landed
/// (node_count > 0). Pure read; a zero count means Resin's own fetcher has
/// not completed yet (the 30s scheduler tick lands it within seconds).
pub async fn resolve_subscription(client: &ResinClient, name: &str) -> StepStatus {
    let live = match client.list_subscriptions().await {
        Ok(v) => v,
        Err(e) => return StepStatus::Failed(format!("list subscriptions: {e}")),
    };
    let items = items_of(&live);
    match items
        .iter()
        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some(name))
    {
        None => StepStatus::Failed(format!("subscription not found on Resin: {name}")),
        Some(row) => {
            let count = row.get("node_count").and_then(|v| v.as_u64()).unwrap_or(0);
            if count > 0 {
                StepStatus::AlreadyPresent
            } else {
                StepStatus::Failed(format!(
                    "subscription has no nodes yet (node_count=0): {name}"
                ))
            }
        }
    }
}

/// Step 3+4: ensure the whitebox carries an a_class=subscription platform
/// named `name` with `name` in its subscriptions list (ONE store entry,
/// generation bump per ADR-0058 D2), then ensure the platform exists on
/// Resin (ADR-0056 create seam). Both halves are idempotent: an existing
/// ref + an existing Resin row produce zero writes.
pub async fn ensure_platform<S: StrategyConfigStore>(
    client: &ResinClient,
    svc: &StrategyService<S>,
    name: &str,
) -> (StepStatus, StepStatus) {
    // ---- whitebox half (steps 3+4 of the spec's 5-step list) ----
    let config = match svc.get() {
        Ok(c) => c,
        Err(e) => return (StepStatus::Failed(format!("whitebox read: {e}")), StepStatus::Failed("whitebox unreadable".to_string())),
    };
    let whitebox_status = match config.platforms.iter().find(|ps| ps.platform_name == name) {
        Some(ps) if ps.a_class == AClassStrategy::Subscription
            && ps.subscriptions.iter().any(|s| s == name) =>
        {
            StepStatus::AlreadyPresent
        }
        _ => {
            let mut next = config;
            if let Some(ps) = next.platforms.iter_mut().find(|ps| ps.platform_name == name) {
                ps.a_class = AClassStrategy::Subscription;
                if !ps.subscriptions.iter().any(|s| s == name) {
                    ps.subscriptions.push(name.to_string());
                }
            } else {
                next.platforms.push(PlatformStrategy {
                    platform_name: name.to_string(),
                    a_class: AClassStrategy::Subscription,
                    b_class: StrategyId::Random,
                    manual_nodes: vec![],
                    regions: vec![],
                    subscriptions: vec![name.to_string()],
                    top_n: 10,
                    b_class_params: Default::default(),
                });
            }
            // The Service store entry: validate + generation bump + versioned
            // file swap (ADR-0036 write entry, ADR-0058 D2 bump point).
            match svc.store(next) {
                Ok(_) => StepStatus::Written,
                Err(e) => StepStatus::Failed(format!("whitebox store: {e}")),
            }
        }
    };
    if whitebox_status.is_failed() {
        // The Resin half cannot proceed without the whitebox intent.
        return (whitebox_status, StepStatus::Failed("whitebox write failed; Resin half skipped".to_string()));
    }

    // ---- Resin half (ADR-0056 create seam) ----
    let live = match client.list_platforms().await {
        Ok(v) => v,
        Err(e) => return (whitebox_status, StepStatus::Failed(format!("list platforms: {e}"))),
    };
    let items = items_of(&live);
    if items
        .iter()
        .any(|p| p.get("name").and_then(|n| n.as_str()) == Some(name))
    {
        return (whitebox_status, StepStatus::AlreadyPresent);
    }
    match client.create_platform_from_name(name).await {
        Ok(_) => (whitebox_status, StepStatus::Written),
        Err(e) => (whitebox_status, StepStatus::Failed(format!("create platform: {e}"))),
    }
}

/// Step 5: strategy_apply — the existing diff-then-skip apply (ADR-0057) +
/// generation write-back (ADR-0058 D3). In-sync platforms cost zero PATCH.
pub async fn apply_strategy(
    client: &ResinClient,
    svc: &StrategyService<crate::strategy_service::FsStrategyStore>,
) -> StepStatus {
    match svc.apply(client, crate::resin_client::resolve_id_in).await {
        Ok(report) => {
            if report.platforms.iter().all(|p| p.patched) {
                StepStatus::Written
            } else {
                let reason = report
                    .platforms
                    .iter()
                    .find(|p| !p.patched)
                    .and_then(|p| p.reason.clone())
                    .unwrap_or_else(|| "apply failed".to_string());
                StepStatus::Failed(reason)
            }
        }
        Err(e) => StepStatus::Failed(format!("strategy apply: {e}")),
    }
}

/// One full pass of the five-step cascade for one subscription. Public so
/// the integration test and the drain loop share one implementation.
pub async fn run_pipeline(
    client: &ResinClient,
    svc: &StrategyService<crate::strategy_service::FsStrategyStore>,
    name: &str,
    url: &str,
) -> PipelineReport {
    let s1 = ensure_subscription(client, name, url).await;
    let s2 = if s1.is_failed() {
        StepStatus::Failed("upstream step failed".to_string())
    } else {
        resolve_subscription(client, name).await
    };
    let (s3, s4) = if s2.is_failed() {
        (StepStatus::Failed("upstream step failed".to_string()), StepStatus::Failed("upstream step failed".to_string()))
    } else {
        ensure_platform(client, svc, name).await
    };
    let s5 = if s3.is_failed() || s4.is_failed() {
        StepStatus::Failed("upstream step failed".to_string())
    } else {
        apply_strategy(client, svc).await
    };
    PipelineReport { subscription: name.to_string(), steps: [s1, s2, s3, s4, s5] }
}

/// Terminal criterion (spec D-C1.1) as a PURE function: consumed_by non-empty
/// AND the target platform is not missing on Resin AND ConvergePhase is
/// Converged or a Drifted whose entries are all acknowledged (Drifted(ack)).
/// The caller feeds it from the authoritative snapshot it already holds —
/// zero new requests.
pub fn is_terminal_state(
    consumed_by: &[String],
    platform_missing_on_resin: bool,
    phase: ConvergePhase,
    drifted_entries_all_acknowledged: bool,
) -> bool {
    if consumed_by.is_empty() || platform_missing_on_resin {
        return false;
    }
    matches!(phase, ConvergePhase::Converged)
        || (matches!(phase, ConvergePhase::Drifted) && drifted_entries_all_acknowledged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy_engine::StrategyConfig;
    use serde_json::json;
    use std::path::PathBuf;

    fn temp_store(tag: &str) -> (StrategyService<crate::strategy_service::FsStrategyStore>, PathBuf) {
        let path = std::env::temp_dir().join(format!("sub-pipeline-{tag}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        (StrategyService::new(crate::strategy_service::FsStrategyStore::new(path.clone())), path)
    }

    fn platform_id_for_name(v: &serde_json::Value, name: &str) -> Option<String> {
        crate::resin_client::resolve_id_in(v, name)
    }

    // ---- pure-fn gates ----

    #[test]
    fn retry_delay_is_exponential_and_capped() {
        assert_eq!(SubscriptionPipeline::retry_delay(0), 2);
        assert_eq!(SubscriptionPipeline::retry_delay(1), 4);
        assert_eq!(SubscriptionPipeline::retry_delay(2), 8);
        assert_eq!(SubscriptionPipeline::retry_delay(4), 32);
        assert_eq!(SubscriptionPipeline::retry_delay(5), 60, "capped at 60s");
        assert_eq!(SubscriptionPipeline::retry_delay(10), 60, "stays capped");
    }

    #[test]
    fn terminal_state_requires_all_three_conditions() {
        let consumer = vec!["sub-x".to_string()];
        assert!(is_terminal_state(&consumer, false, ConvergePhase::Converged, false));
        // consumed_by empty -> not terminal even when converged
        assert!(!is_terminal_state(&[], false, ConvergePhase::Converged, false));
        // platform missing -> not terminal
        assert!(!is_terminal_state(&consumer, true, ConvergePhase::Converged, false));
        // PendingApply / ApplyFailed / NeverApplied / Unknown -> not terminal
        assert!(!is_terminal_state(&consumer, false, ConvergePhase::PendingApply, false));
        assert!(!is_terminal_state(&consumer, false, ConvergePhase::ApplyFailed, false));
        assert!(!is_terminal_state(&consumer, false, ConvergePhase::NeverApplied, false));
        assert!(!is_terminal_state(&consumer, false, ConvergePhase::Unknown, false));
        // Drifted with acknowledged entries -> terminal (Drifted(ack))
        assert!(is_terminal_state(&consumer, false, ConvergePhase::Drifted, true));
        // Drifted with UNacknowledged drift -> not terminal
        assert!(!is_terminal_state(&consumer, false, ConvergePhase::Drifted, false));
    }

    // ---- queue mechanics (level-triggered + bounded) ----

    #[test]
    fn enqueue_coalesces_per_name_and_respects_bound() {
        let p = SubscriptionPipeline::new();
        assert!(p.enqueue(EstablishEvent { subscription: "a".into(), url: "https://u/a".into() }));
        assert!(p.enqueue(EstablishEvent { subscription: "a".into(), url: "https://u/a2".into() }), "duplicate enqueue must coalesce, not duplicate");
        assert!(p.enqueue(EstablishEvent { subscription: "b".into(), url: "https://u/b".into() }));
        assert_eq!(p.pending(), vec!["a".to_string(), "b".to_string()]);
        // Fill the remaining 62 slots (a+b already queued) to exactly MAX_QUEUE.
        for i in 0..(MAX_QUEUE - 2) {
            assert!(p.enqueue(EstablishEvent { subscription: format!("n{i}"), url: String::new() }));
        }
        assert_eq!(p.pending().len(), MAX_QUEUE);
        assert!(!p.enqueue(EstablishEvent { subscription: "overflow".into(), url: String::new() }), "queue must refuse beyond MAX_QUEUE");
    }

    // ---- THREE-STATE behavior locks against a mockito Resin ----

    const BEARER: (&str, &str) = ("authorization", "Bearer testtok");

    async fn mock_sub(server: &mut mockito::ServerGuard, name: &str, count: u64) -> mockito::Mock {
        server
            .mock("GET", "/api/v1/subscriptions")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"id": "sub-1", "name": name, "node_count": count}]}).to_string())
            .expect(3) // ensure read + resolve read + apply's dangling-ref read
            .create_async()
            .await
    }

    /// STATE 1 — success: a fresh subscription flows through all five steps
    /// (POST sub, resolve, whitebox store, POST platform, apply PATCH) and
    /// the report is all-green.
    #[tokio::test]
    async fn pipeline_success_all_five_steps_write() {
        let (svc, path) = temp_store("success");
        let mut server = mockito::Server::new_async().await;
        // Subscription list hits, in wire order: (1) ensure_subscription presence
        // read — ABSENT; (2) resolve read — present with nodes; (3) apply's
        // dangling-ref check read — present.
        let m_subs_absent = server
            .mock("GET", "/api/v1/subscriptions")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": []}).to_string())
            .expect(1)
            .create_async()
            .await;
        let m_subs_present = server
            .mock("GET", "/api/v1/subscriptions")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"id": "sub-1", "name": "newsub", "node_count": 7}]}).to_string())
            .expect(2)
            .create_async()
            .await;
        let m_create_sub = server
            .mock("POST", "/api/v1/subscriptions")
            .match_header(BEARER.0, BEARER.1)
            .match_body(mockito::Matcher::PartialJson(json!({
                "name": "newsub", "source_type": "remote",
                "url": "https://example.invalid/sub.yaml", "update_interval": "30s"
            })))
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "sub-1", "name": "newsub"}).to_string())
            .expect(1)
            .create_async()
            .await;
        // platforms hits, in wire order: (1) ensure_platform's presence read —
        // absent; (2) apply's initial live read — the row EXISTS by now (the
        // create happened in step 3), so apply does NOT re-create; (3) apply's
        // per-platform re-read — the created row with its id.
        let m_platforms_empty = server
            .mock("GET", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": []}).to_string())
            .expect(1)
            .create_async()
            .await;
        let m_platforms_created = server
            .mock("GET", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"id": "id-newsub", "name": "newsub", "region_filters": []}]}).to_string())
            .expect(2)
            .create_async()
            .await;
        let m_create_platform = server
            .mock("POST", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .match_body(mockito::Matcher::PartialJson(json!({"name": "newsub"})))
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "id-newsub", "name": "newsub"}).to_string())
            .expect(1)
            .create_async()
            .await;
        // nodes: the subscription landed one healthy HK node (with subscription_name tag)
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"node_hash": "h1", "region": "HK", "has_outbound": true, "failure_count": 0, "tags": [{"subscription_name": "newsub"}]}]}).to_string())
            .expect(1)
            .create_async()
            .await;
        let m_patch = server
            .mock("PATCH", "/api/v1/platforms/id-newsub")
            .match_header(BEARER.0, BEARER.1)
            .match_body(mockito::Matcher::PartialJson(json!({"region_filters": ["HK"]})))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "id-newsub"}).to_string())
            .expect(1)
            .create_async()
            .await;

        let base = server.url();
        let client = ResinClient::new(&base, "testtok".into()).unwrap();
        let report = run_pipeline(&client, &svc, "newsub", "https://example.invalid/sub.yaml").await;
        assert!(report.all_ok(), "full pass must be green: {report:?}");
        assert!(matches!(report.steps[0], StepStatus::Written), "sub create: {report:?}");
        assert!(matches!(report.steps[1], StepStatus::AlreadyPresent), "resolve is a read: {report:?}");
        assert!(matches!(report.steps[2], StepStatus::Written), "whitebox store: {report:?}");
        assert!(matches!(report.steps[3], StepStatus::Written), "platform create: {report:?}");
        assert!(matches!(report.steps[4], StepStatus::Written), "apply PATCH: {report:?}");
        // The whitebox entry exists with the subscription ref (step 3+4 fused).
        let cfg = svc.get().unwrap();
        assert!(cfg.platforms.iter().any(|ps| ps.platform_name == "newsub"
            && ps.a_class == AClassStrategy::Subscription
            && ps.subscriptions.contains(&"newsub".to_string())));
        // generation: +1 from the pipeline's one store entry, +1 from apply's
        // GREEN write-back (ADR-0058 D3 — the green pass lands applied ==
        // generation at the post-bump value).
        assert_eq!(cfg.generation, 2);

        m_subs_absent.assert_async().await;
        m_subs_present.assert_async().await;
        m_create_sub.assert_async().await;
        m_platforms_empty.assert_async().await;
        m_platforms_created.assert_async().await;
        m_create_platform.assert_async().await;
        m_nodes.assert_async().await;
        m_patch.assert_async().await;
        let _ = std::fs::remove_file(path);
    }

    /// STATE 2 — idempotent: a second pass over the converged world writes
    /// NOTHING (no POST sub, no POST platform, no PATCH): every step reports
    /// AlreadyPresent / a zero-write apply skip.
    #[tokio::test]
    async fn pipeline_idempotent_second_pass_zero_writes() {
        let (svc, path) = temp_store("idem");
        // Seed the whitebox with the converged entry BEFORE pass 2.
        svc.store(StrategyConfig {
            platforms: vec![PlatformStrategy {
                platform_name: "newsub".into(),
                a_class: AClassStrategy::Subscription,
                b_class: StrategyId::Random,
                manual_nodes: vec![],
                regions: vec![],
                subscriptions: vec!["newsub".into()],
                top_n: 10,
                b_class_params: Default::default(),
            }],
            ..Default::default()
        }).unwrap();

        let mut server = mockito::Server::new_async().await;
        // subscription list: already present (used by step 1 + step 2).
        let m_subs = mock_sub(&mut server, "newsub", 7).await;
        // platforms: live row already has the platform AND the converged
        // region_filters the apply would compute (HK from the node mock) ->
        // diff-then-skip finds it in sync: zero PATCH.
        // Platform GETs, in wire order: ensure presence read (1) + apply's
        // initial live read (2) + apply's per-platform re-read (3). All three
        // see the same in-sync row.
        let m_platforms = server
            .mock("GET", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"id": "id-newsub", "name": "newsub", "region_filters": ["HK"]}]}).to_string())
            .expect(3)
            .create_async()
            .await;
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"node_hash": "h1", "region": "HK", "has_outbound": true, "failure_count": 0, "tags": [{"subscription_name": "newsub"}]}]}).to_string())
            .expect(1)
            .create_async()
            .await;
        // NO mocks for POST /subscriptions, POST /platforms, PATCH — any
        // write would 404 and fail the report, which is the wire-level gate.
        // (mockito without a matching mock returns 404 by default.)

        let base = server.url();
        let client = ResinClient::new(&base, "testtok".into()).unwrap();
        let report = run_pipeline(&client, &svc, "newsub", "https://example.invalid/sub.yaml").await;
        assert!(report.all_ok(), "idempotent pass must be green with zero writes: {report:?}");
        assert!(matches!(report.steps[0], StepStatus::AlreadyPresent), "sub already on Resin: {report:?}");
        assert!(matches!(report.steps[2], StepStatus::AlreadyPresent), "whitebox ref already present: {report:?}");
        assert!(matches!(report.steps[3], StepStatus::AlreadyPresent), "platform already on Resin: {report:?}");
        // The PIPELINE's whitebox store entry did NOT run (ref already present,
        // platforms list unchanged). The generation DID advance 1->2 — that is
        // apply's OWN green write-back (ADR-0058 D3: a diff-then-skip zero-PATCH
        // pass still refreshes last_apply_at, Terraform re-apply semantics).
        let cfg = svc.get().unwrap();
        assert_eq!(cfg.generation, 2, "only apply's green write-back may bump");
        assert_eq!(cfg.platforms.len(), 1, "no duplicate platform entry");
        assert_eq!(cfg.platforms[0].subscriptions, vec!["newsub".to_string()], "ref not duplicated");

        m_subs.assert_async().await;
        m_platforms.assert_async().await;
        m_nodes.assert_async().await;
        let _ = std::fs::remove_file(path);
    }

    /// STATE 3 — partial failure: the subscription lands, the platform
    /// creates, but the apply PATCH fails (sub OK + platform OK + apply
    /// failed). The report carries the failure in step 5 with the upstream
    /// reason; earlier steps stay green.
    #[tokio::test]
    async fn pipeline_partial_failure_apply_fails_after_platform_ok() {
        let (svc, path) = temp_store("partial");
        let mut server = mockito::Server::new_async().await;
        // Subscription list hits: (1) ensure read — ABSENT; (2) resolve read +
        // (3) apply dangling-ref read — present with nodes.
        let m_subs_absent = server
            .mock("GET", "/api/v1/subscriptions")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": []}).to_string())
            .expect(1)
            .create_async()
            .await;
        // present-phase mock: exactly 2 hits here (resolve read + apply's
        // dangling-ref read) — the ensure read went to m_subs_absent above.
        // Research rule (atomcode 2.5-E): chain length = Σ reads per phase;
        // a present-from-start flow would be 3, this one is absent-first.
        let m_subs_present = server
            .mock("GET", "/api/v1/subscriptions")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"id": "s1", "name": "flaky", "node_count": 5}]}).to_string())
            .expect(2)
            .create_async()
            .await;
        let m_create_sub = server
            .mock("POST", "/api/v1/subscriptions")
            .match_header(BEARER.0, BEARER.1)
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "sub-2", "name": "flaky"}).to_string())
            .expect(1)
            .create_async()
            .await;
        let m_platforms_empty = server
            .mock("GET", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": []}).to_string())
            .expect(1)
            .create_async()
            .await;
        let m_platforms_created = server
            .mock("GET", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"id": "id-flaky", "name": "flaky", "region_filters": []}]}).to_string())
            .expect(2)
            .create_async()
            .await;
        let m_create_platform = server
            .mock("POST", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "id-flaky", "name": "flaky"}).to_string())
            .expect(1)
            .create_async()
            .await;
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"node_hash": "h1", "region": "US", "has_outbound": true, "failure_count": 0, "tags": [{"subscription_name": "flaky"}]}]}).to_string())
            .expect(1)
            .create_async()
            .await;
        // The ONE failing write: the apply PATCH 500s. The node carries the
        // subscription tag so the computed region plan (["US"]) DIFFERS from
        // the live row ([]) — without it diff-then-skip would find the empty
        // plan in sync and the pass would go green without any PATCH.
        // Research rule (atomcode D-1/A): every HTTP request INCLUDING retries
        // consumes one quota — ResinClient retries 5xx 3x total, so the 500
        // mock must accept exactly 3 hits (attempt 2-3 get 501 from the
        // exhausted chain, also 5xx-class, still retried to exhaustion).
        let m_patch = server
            .mock("PATCH", "/api/v1/platforms/id-flaky")
            .match_header(BEARER.0, BEARER.1)
            .with_status(500)
            .expect(3)
            .create_async()
            .await;

        let base = server.url();
        let client = ResinClient::new(&base, "testtok".into()).unwrap();
        let report = run_pipeline(&client, &svc, "flaky", "https://example.invalid/flaky.yaml").await;
        assert!(!report.all_ok(), "apply failure must fail the pass: {report:?}");
        assert!(matches!(report.steps[0], StepStatus::Written), "sub step green: {report:?}");
        assert!(matches!(report.steps[3], StepStatus::Written), "platform step green: {report:?}");
        assert!(matches!(report.steps[4], StepStatus::Failed(_)), "apply step failed: {report:?}");
        assert!(report.first_error().unwrap().contains("PATCH failed"), "reason carries the upstream failure: {report:?}");
        // ADR-0058: a failed apply keeps applied_generation at the old value
        // and records last_apply_error — no fake convergence.
        let cfg = svc.get().unwrap();
        assert!(cfg.last_apply_error.is_some(), "failure must be persistent state");

        m_subs_absent.assert_async().await;
        m_subs_present.assert_async().await;
        m_create_sub.assert_async().await;
        m_platforms_empty.assert_async().await;
        m_platforms_created.assert_async().await;
        m_create_platform.assert_async().await;
        m_nodes.assert_async().await;
        m_patch.assert_async().await;
        let _ = std::fs::remove_file(path);
    }

    /// Backoff + parking: drain re-queues a failed event with next_retry_at
    /// in the future; a later drain past the backoff retries it; after
    /// MAX_ATTEMPTS it parks as failed_count().
    #[tokio::test]
    async fn drain_backoff_retries_then_parks_after_max_attempts() {
        let (svc, path) = temp_store("backoff");
        // No mocks: every request 404s -> step 1 fails every pass.
        let server_url = "http://127.0.0.1:1";
        let client = ResinClient::new(server_url, "t".into()).unwrap();
        let p = SubscriptionPipeline::new();
        assert!(p.enqueue(EstablishEvent { subscription: "x".into(), url: "https://u/x".into() }));

        let now: u64 = 1_000_000;
        // MAX_ATTEMPTS consecutive drains, each advancing the clock past the backoff.
        for attempt in 1..=MAX_ATTEMPTS {
            let reports = p.drain(&client, &svc, now + u64::from(attempt) * 120).await;
            assert_eq!(reports.len(), 1, "attempt {attempt} must process the event");
            assert!(!reports[0].all_ok());
            if attempt < MAX_ATTEMPTS {
                assert_eq!(p.pending(), vec!["x".to_string()], "re-queued for backoff retry");
                assert_eq!(p.failed_count(), 0);
            }
        }
        assert!(p.pending().is_empty() || p.failed_count() == 1, "after max attempts the event parks");
        assert_eq!(p.failed_count(), 1, "parked event is counted");
        // A parked event is NOT retried by later drains.
        let reports = p.drain(&client, &svc, now + 999_999).await;
        assert!(reports.is_empty(), "parked events never re-run");
        // A fresh user enqueue re-arms it (fresh action = fresh budget).
        assert!(p.enqueue(EstablishEvent { subscription: "x".into(), url: "https://u/x".into() }));
        let reports = p.drain(&client, &svc, now + 1_000_000).await;
        assert_eq!(reports.len(), 1, "re-enqueued event runs again");
        let _ = std::fs::remove_file(path);
    }

    /// drain on success removes the event entirely (terminal: no re-run).
    #[tokio::test]
    async fn drain_success_removes_event_from_queue() {
        let (svc, path) = temp_store("drain-ok");
        let mut server = mockito::Server::new_async().await;
        // Converged world: sub present with nodes, platform live + in sync.
        let m_subs = mock_sub(&mut server, "conv", 9).await;
        let m_platforms = server
            .mock("GET", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"id": "id-conv", "name": "conv", "region_filters": ["HK"]}]}).to_string())
            .expect(3) // ensure read + apply initial + apply per-platform re-read
            .create_async()
            .await;
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"node_hash": "h1", "region": "HK", "has_outbound": true, "failure_count": 0, "tags": [{"subscription_name": "conv"}]}]}).to_string())
            .expect(1)
            .create_async()
            .await;
        // Whitebox pre-seeded with the converged entry.
        svc.store(StrategyConfig {
            platforms: vec![PlatformStrategy {
                platform_name: "conv".into(),
                a_class: AClassStrategy::Subscription,
                b_class: StrategyId::Random,
                manual_nodes: vec![],
                regions: vec![],
                subscriptions: vec!["conv".into()],
                top_n: 10,
                b_class_params: Default::default(),
            }],
            ..Default::default()
        }).unwrap();

        let base = server.url();
        let client = ResinClient::new(&base, "testtok".into()).unwrap();
        let p = SubscriptionPipeline::new();
        assert!(p.enqueue(EstablishEvent { subscription: "conv".into(), url: "https://u/conv".into() }));
        let reports = p.drain(&client, &svc, 1_700_000_000).await;
        assert_eq!(reports.len(), 1);
        assert!(reports[0].all_ok(), "converged world must pass green: {:?}", reports[0]);
        assert!(p.pending().is_empty(), "successful drain removes the event");
        assert_eq!(p.failed_count(), 0);

        m_subs.assert_async().await;
        m_platforms.assert_async().await;
        m_nodes.assert_async().await;
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pipeline_report_helpers() {
        let ok = PipelineReport {
            subscription: "x".into(),
            steps: [
                StepStatus::Written,
                StepStatus::AlreadyPresent,
                StepStatus::Written,
                StepStatus::AlreadyPresent,
                StepStatus::Written,
            ],
        };
        assert!(ok.all_ok());
        assert_eq!(ok.first_error(), None);
        let partial = PipelineReport {
            subscription: "x".into(),
            steps: [
                StepStatus::Written,
                StepStatus::AlreadyPresent,
                StepStatus::Written,
                StepStatus::AlreadyPresent,
                StepStatus::Failed("PATCH failed: 500".into()),
            ],
        };
        assert!(!partial.all_ok());
        assert_eq!(partial.first_error(), Some("PATCH failed: 500"));
    }
}
