//! Subscription establish pipeline
//! — the between-steps glue the ADR lattice implies but never
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
//! Terminal criterion: snapshot consumed_by non-empty AND the
//! target platform not missing on Resin AND ConvergePhase in
//! {Converged, Drifted(acknowledged)}. `is_terminal_state` is a pure fn so
//! the check is unit-testable without Resin.
//!
//! Failure model (allow clause): a failed step is PERSISTENT STATE —
//! the event stays queued with attempts + last_error and is retried by the
//! caller with exponential backoff (`retry_delay`), capped at
//! MAX_ATTEMPTS. UI phase state is 's SubscriptionPhase. Since
//! a failed pass ALSO runs `compensate_failed_cascade` — the
//! partial-failure compensation that deletes only what THIS pass created
//! (never the strategy whitebox, never the subscription) and persists the
//! ordered sub/plat/port/apply marking as `last_cascade_error` on the
//! subscription's status row via
//! `StrategyService::record_cascade_failure`.

use std::collections::VecDeque;
use std::sync::Mutex as StdMutex;

use serde::{Deserialize, Serialize};

use crate::resin_client::ResinClient;
use crate::strategy::StrategyId;
use crate::strategy_engine::{AClassStrategy, PlatformStrategy};
use crate::strategy_engine::{EstablishStep, SubscriptionPhase};
use crate::strategy_service::{StrategyConfigStore, StrategyService};
use crate::snapshot::ConvergePhase;
use crate::db::{DbPool, PortMapping};
use crate::port_forwarder::PortForwarder;
use crate::whitebox_config::WhiteboxConfigStore;

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
                    b_class: StrategyId::Balanced,
                    manual_nodes: vec![],
                    regions: vec![],
                    subscriptions: vec![name.to_string()],
                    top_n: 10,
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
/// Returns the step status plus the platform the failure BELONGS to (the
/// first non-patched report row) — 's compensation keys on that owner so
/// it never deletes a cascade resource for a failure this cascade did not
/// cause. `None` = the apply pass failed without a platform-scoped row
/// (transport-level failure) or did not run.
pub async fn apply_strategy(
    client: &ResinClient,
    svc: &StrategyService<crate::strategy_service::FsStrategyStore>,
) -> (StepStatus, Option<String>) {
    match svc.apply(client, crate::resin_client::resolve_id_in).await {
        Ok(report) => {
            if report.platforms.iter().all(|p| p.patched) {
                (StepStatus::Written, None)
            } else {
                let failed = report.platforms.iter().find(|p| !p.patched);
                let reason = failed
                    .and_then(|p| p.reason.clone())
                    .unwrap_or_else(|| "apply failed".to_string());
                (StepStatus::Failed(reason), failed.map(|p| p.platform.clone()))
            }
        }
        Err(e) => (StepStatus::Failed(format!("strategy apply: {e}")), None),
    }
}

/// One full pass of the five-step cascade for one subscription. Public so
/// the integration test and the drain loop share one implementation.
///
/// the pass also DRIVES the persisted phase state
/// machine — Importing while the data-landing beats run, Establishing with
/// the platform/bind/apply sub-step while the whitebox beats run, Converged
/// on an all-green pass, Failed(stage, reason) on the first failure. Every
/// transition goes through `StrategyService::record_subscription_phase`
/// (the ONE store entry — validated, versioned, audited; generation does
/// NOT move, see the status-subresource note there). A FAILED phase write
/// itself never fails the cascade: the pipeline report is the source of
/// truth for the caller, the chip degrades to the last recorded phase
/// (honest staleness beats a broken import).
pub async fn run_pipeline(
    client: &ResinClient,
    svc: &StrategyService<crate::strategy_service::FsStrategyStore>,
    name: &str,
    url: &str,
) -> PipelineReport {
    // The user's establish request is the trigger — the phase leaves Never.
    let _ = svc.record_subscription_phase(name, SubscriptionPhase::Importing, None, None);

    let s1 = ensure_subscription(client, name, url).await;
    let s2 = if s1.is_failed() {
        StepStatus::Failed("upstream step failed".to_string())
    } else {
        resolve_subscription(client, name).await
    };
    if s1.is_failed() || s2.is_failed() {
        // Data-landing failure: the phase parks on Failed with the sub-step
        // that produced it (Import/Resolve; steps after a failure are
        // skipped, so the FIRST failed step is the stage).
        let (stage, reason) = failed_stage(EstablishStep::Import, &[&s1, &s2]);
// nothing past the subscription exists yet, so the
        // compensation is a recorded no-op — the subscription itself (even
        // when this pass just created it) is the user's original data and
        // is NEVER deleted.
        let actions = compensate_failed_cascade(client, name, false, None).await;
        let _ = svc.record_cascade_failure(name, stage, &reason, actions);
        return PipelineReport {
            subscription: name.to_string(),
            steps: [s1, s2, StepStatus::Failed("upstream step failed".to_string()), StepStatus::Failed("upstream step failed".to_string()), StepStatus::Failed("upstream step failed".to_string())],
        };
    }

    let _ = svc.record_subscription_phase(name, SubscriptionPhase::Establishing, Some(EstablishStep::Platform), None);
    let (s3, s4) = ensure_platform(client, svc, name).await;
    if s3.is_failed() || s4.is_failed() {
        let (stage, reason) = failed_stage(EstablishStep::Platform, &[&s3, &s4]);
        // the Resin platform row was NOT created this pass
        // (s4 failed — a create that returned Err is treated as not
        // created), so the compensation is a recorded no-op: only the
        // subscription survives. The whitebox entry this pass may have
        // written is L2 desired state and is NEVER deleted here (checkpoint
        // B: whitebox deletion is another ticket's path).
        let actions = compensate_failed_cascade(client, name, false, None).await;
        let _ = svc.record_cascade_failure(name, stage, &reason, actions);
        return PipelineReport {
            subscription: name.to_string(),
            steps: [s1, s2, s3, s4, StepStatus::Failed("upstream step failed".to_string())],
        };
    }

    let _ = svc.record_subscription_phase(name, SubscriptionPhase::Establishing, Some(EstablishStep::Apply), None);
    let (s5, failing_platform) = apply_strategy(client, svc).await;
    if let StepStatus::Failed(reason) = &s5 {
// the platform row exists. Compensation deletes it
        // ONLY when this pass created it (s4 Written) AND the apply failure
        // belongs to it — a failure owned by an unrelated platform must not
        // delete this cascade's resource (that platform may even be
        // already in sync).
        let created_this_pass = matches!(s4, StepStatus::Written);
        let actions =
            compensate_failed_cascade(client, name, created_this_pass, failing_platform.as_deref()).await;
        let _ = svc.record_cascade_failure(name, EstablishStep::Apply, reason, actions);
    } else {
        // Converged also clears last_cascade_error (record_subscription_phase).
        let _ = svc.record_subscription_phase(name, SubscriptionPhase::Converged, None, None);
    }
    PipelineReport { subscription: name.to_string(), steps: [s1, s2, s3, s4, s5] }
}

/// First failure in an ordered step run: its stage tag + KEP-1623-style
/// reason (the reason the failed step itself recorded). A run where every
/// step somehow reports failure without a Failed variant cannot happen —
/// the caller only invokes this when at least one step IS Failed.
fn failed_stage(base: EstablishStep, steps: &[&StepStatus]) -> (EstablishStep, String) {
    // Canonical beat order; the base stage anchors the caller's section and
    // each position past it advances one beat ( inserts Port between Bind
    // and Apply — the order already carries the slot).
    const ORDER: [EstablishStep; 6] = [
        EstablishStep::Import,
        EstablishStep::Resolve,
        EstablishStep::Platform,
        EstablishStep::Bind,
        EstablishStep::Port,
        EstablishStep::Apply,
    ];
    let base_idx = ORDER.iter().position(|s| *s == base).unwrap_or(0);
    for (idx, step) in steps.iter().enumerate() {
        if let StepStatus::Failed(reason) = step {
            let stage = ORDER[(base_idx + idx).min(ORDER.len() - 1)];
            return (stage, reason.clone());
        }
    }
    (base, "cascade failed".to_string())
}

/// Terminal criterion as a PURE function: consumed_by non-empty
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

// ---------------------------------------------------------------------------
// Optional default-port tail ( spec
// : the establish cascade may END by creating ONE socks5 entry port
// bound to the freshly established platform — only when the user did not
// provide a binding target. Everything the step needs (ResinClient, DbPool,
// PortForwarder, WhiteboxConfigStore) is resin-core-owned; the shell passes
// its Tauri-managed handles at the call site (subscription_add, right after
// a green drain pass — no drain/run_pipeline signature churn, the shared
// 5-step report shape is untouched).
//
// Conflict law: the user's pre-existing state ALWAYS wins. A
// whitebox row already bound to this platform (the user's own binding
// target, or this step's own pass-1 product = idempotence), a row holding
// the candidate port for another platform, a foreign Resin listener, or a
// 409 from the create POST: every one of those logs a WARNING and returns
// AlreadyPresent — never an overwrite, never an error. Only genuinely
// retryable failures (suggest drained, Resin unreachable, whitebox write
// rejected) return Failed so they park/backoff like any other step.
// ---------------------------------------------------------------------------

/// Suggest a free entry port — the same ADR-0031 algorithm the shell's
/// `port_suggest` command has always run: used ports from the DbPool (the
/// whitebox's SQLite sync partner), scan 17990..=65535 skipping used, probe
/// each candidate with a loopback TcpListener bind, first free port wins;
/// an OS-assigned ephemeral port is the fallback when the whole range is
/// blocked. The command now delegates here so the GUI and the pipeline
/// suggest identically (one implementation, not two).
pub fn suggest_free_entry_port(db: &DbPool) -> Result<u16, String> {
    let used: std::collections::HashSet<u16> =
        db.list_ports()?.into_iter().map(|m| m.port).collect();
    for candidate in 17990u16..=65535u16 {
        if used.contains(&candidate) {
            continue;
        }
        if std::net::TcpListener::bind(("127.0.0.1", candidate)).is_ok() {
            return Ok(candidate);
        }
    }
    Ok(std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("port_suggest: no free port: {e}"))?
        .local_addr()
        .map_err(|e| format!("port_suggest: no local addr: {e}"))?
        .port())
}

/// Resin endpoint port numbers from a list response (items wrapper or bare
/// array — the `items_of` tolerance). The `default` management endpoint is
/// excluded: only shell-owned custom listeners count as conflicts.
fn custom_endpoint_ports(v: &serde_json::Value) -> Vec<u16> {
    items_of(v)
        .iter()
        .filter(|ep| ep.get("id").and_then(|i| i.as_str()) != Some("default"))
        .filter_map(|ep| {
            ep.get("port")
                .and_then(|p| p.as_u64())
                .and_then(|p| u16::try_from(p).ok())
        })
        .collect()
}

/// Step 6 — the OPTIONAL default-port. The shell's
/// subscription_add invokes it right after a green establish pass; the step
/// is idempotent (a re-invocation after a retried pass writes nothing), so
/// the level-triggered re-run discipline holds without owning queue state.
pub async fn ensure_default_port(
    client: &ResinClient,
    db: &DbPool,
    forwarder: &PortForwarder,
    whitebox: &WhiteboxConfigStore,
    platform_name: &str,
    user_port: Option<u16>,
) -> StepStatus {
    // (a) Binding-target conflict / idempotence: a whitebox row already
    // bound to this platform (ANY port). A pre-existing binding means the
    // user HAS a binding target — "only when the user did not provide one".
    let snapshot = whitebox.snapshot();
    if let Some(existing) = snapshot.entry_ports.iter().find(|row| row.platform_name == platform_name) {
        tracing::warn!(
            platform = %platform_name,
            port = existing.port,
            "subscription_pipeline: entry port already bound to this platform; default-port skipped (warning, not overwritten)"
        );
        return StepStatus::AlreadyPresent;
    }
    // (b) Candidate port: the user's explicit binding target wins (suggest
    // skipped); otherwise the ADR-0031 suggestion probe.
    let port = match user_port {
        // §7.5: the IPC boundary already rejected privileged ports; this
        // core-side guard only keeps a deterministic input bug from burning
        // the whole retry budget.
        Some(p) if p < crate::port_forwarder::MIN_USER_PORT => {
            tracing::warn!(
                platform = %platform_name,
                port = p,
                "subscription_pipeline: user-provided default_port is privileged; default-port skipped"
            );
            return StepStatus::AlreadyPresent;
        }
        Some(p) => p,
        None => match suggest_free_entry_port(db) {
            Ok(p) => p,
            Err(e) => return StepStatus::Failed(format!("suggest default port: {e}")),
        },
    };
    // (c) Same-number conflict: another platform already holds this port in
    // the whitebox. Suggest never picks a used port, so this fires only for
    // user-provided ports.
    if snapshot.entry_ports.iter().any(|row| row.port == port) {
        tracing::warn!(
            platform = %platform_name,
            port,
            "subscription_pipeline: port already bound to another platform; default-port skipped (warning, not overwritten)"
        );
        return StepStatus::AlreadyPresent;
    }
    // (d) Resin-level conflict: a foreign listener on the port is left
    // INTACT (restore_ports_from_whitebox precedent: conflict = skip).
    let live = match client.list_endpoints().await {
        Ok(v) => v,
        Err(e) => return StepStatus::Failed(format!("list endpoints: {e}")),
    };
    if custom_endpoint_ports(&live).contains(&port) {
        tracing::warn!(
            platform = %platform_name,
            port,
            "subscription_pipeline: Resin already listens on the port; default-port skipped (warning, not overwritten)"
        );
        return StepStatus::AlreadyPresent;
    }
    // (e) Create the mixed listener — the exact endpoint body port_upsert
// sends for a `mixed` mapping ( `socks5` no
    // longer implies HTTP forwarding, so the dual-flag default port declares
    // itself `mixed`). require_proxy_auth_info defaults on, matching the
    // GUI's default for new ports).
    // Mode A (ticket 17): the engine must NOT bind this port - the shell's
    // accept loop owns it (step (f) writes the row and the whitebox apply
    // binds it). A create_endpoint here would EADDRINUSE-collide.
    if !forwarder.is_shell() {
        let (allow_socks5, allow_http_forward) =
            crate::entry_protocol::engine_flags(crate::entry_protocol::DEFAULT_ENTRY_PORT_PROTOCOL);
        let body = serde_json::json!({
            "port": port,
            "allow_management": false,
            "allow_proxy": true,
            "allow_http_forward": allow_http_forward,
            "allow_http_reverse": false,
            "allow_socks5": allow_socks5,
            "require_proxy_auth_info": true,
        });
        if let Err(e) = client.create_endpoint(body).await {
            let msg = format!("{e}");
            if msg.contains("409") || msg.contains("CONFLICT") || msg.contains("Only one usage") {
                tracing::warn!(
                    platform = %platform_name,
                    port,
                    error = %msg,
                    "subscription_pipeline: Resin reports the port taken; default-port skipped (warning, not overwritten)"
                );
                return StepStatus::AlreadyPresent;
            }
            return StepStatus::Failed(format!("create endpoint: {msg}"));
        }
    }
    // (f) Whitebox write through the ONE write entry (validate -> SQLite ->
    // listeners -> atomic JSON -> swap; ADR-0042 S2, generation bump per
    // ADR-0058 ). Identity defaults mirror port_upsert's.
    let mapping = PortMapping {
        port,
        protocol: crate::entry_protocol::DEFAULT_ENTRY_PORT_PROTOCOL.to_string(),
        platform_name: platform_name.to_string(),
        account: format!("port-{port}"),
        label: String::new(),
        enabled: true,
        auth_required: true,
    };
    let mut next = whitebox.snapshot();
    // Race guard: a row may have landed between the read and this write.
    if next.entry_ports.iter().any(|row| row.port == port) {
        tracing::warn!(
            platform = %platform_name,
            port,
            "subscription_pipeline: default-port skipped, port landed concurrently (warning, not overwritten)"
        );
        return StepStatus::AlreadyPresent;
    }
    next.entry_ports.push(mapping);
    next.entry_ports.sort_by_key(|row| row.port);
    match whitebox.apply(db, forwarder, next).await {
        Ok(_) => {
            tracing::info!(
                platform = %platform_name,
                port,
                "subscription_pipeline: default socks5 entry port created"
            );
            StepStatus::Written
        }
        Err(e) => StepStatus::Failed(format!("whitebox store: {e}")),
    }
}

// ---------------------------------------------------------------------------
// Partial-failure compensation ( spec
// : a failed establish pass compensates ONLY the resources THIS pass
// created. The law (ADR-0054-compliant explicit rollback — never a silent
// auto-heal, never an L3->L2 write):
//   - the subscription is the user's original data and is NEVER deleted;
//   - the strategy whitebox (L2 desired state) is NEVER touched — whitebox
//     deletion is another ticket's path (checkpoint B);
//   - what CAN be undone is the Resin-side platform row created by this
//     pass's `ensure_platform`, and only when the apply failure BELONGS to
//     it (a failure owned by an unrelated platform must not delete this
//     cascade's resource).
// The default-port tail only arms after a GREEN pass, so it can never
// be part of a failed establish marking here.
// ---------------------------------------------------------------------------

/// Collapse control characters (C0 + DEL) to spaces so an embedded error
/// string can never smuggle a NUL into the persisted marking (the whitebox
/// validator rejects NUL) or a line break into a one-line UI hover.
fn sanitize_action(s: String) -> String {
    s.chars()
        .map(|c| if (c as u32) < 0x20 || (c as u32) == 0x7f { ' ' } else { c })
        .collect()
}

/// Compensate one failed cascade pass and return the ordered partial-failure
/// marking: STRICTLY one entry per cascade step in sub/plat/port/apply order
/// (the `last_cascade_error.rollback_actions[]` schema lock). Best-effort: a
/// failed DELETE is recorded in the marking and never fails the phase record.
///
/// `platform_created_this_pass` — the pass's ensure_platform Resin half
/// returned Written. `failing_platform` — the platform the apply failure
/// belongs to (`apply_strategy`'s owner probe); `None` on pre-apply
/// failures and transport-level apply errors.
pub async fn compensate_failed_cascade(
    client: &ResinClient,
    name: &str,
    platform_created_this_pass: bool,
    failing_platform: Option<&str>,
) -> Vec<String> {
    let mut actions = Vec::with_capacity(4);
    // [0] sub — kept, always: the subscription is the user's original data.
    actions.push("sub: kept (user data)".to_string());
    // [1] plat — delete ONLY the Resin row this pass created, and only when
    // the apply failure belongs to it.
    if platform_created_this_pass {
        match failing_platform {
            Some(owner) if owner == name => match delete_cascade_platform(client, name).await {
                Ok(()) => {
                    actions.push("plat: deleted on Resin (cascade-created, apply failed)".to_string())
                }
                Err(e) => actions.push(sanitize_action(format!("plat: delete failed ({e})"))),
            },
            Some(owner) => actions.push(sanitize_action(format!(
                "plat: kept (apply failure owned by {})",
                owner
            ))),
            None => actions.push("plat: kept (apply failure not platform-scoped)".to_string()),
        }
    } else {
        actions.push("plat: none (not created this pass)".to_string());
    }
    // [2] port — the default-port tail never arms on a failed pass.
    actions.push("port: none (not run)".to_string());
    // [3] apply — the failing beat itself (or not reached on earlier failures).
    actions.push(match failing_platform {
        Some(owner) => sanitize_action(format!("apply: failed on {}", owner)),
        None => "apply: not reached".to_string(),
    });
    actions
}

/// Delete the Resin platform row `name` — the ONLY deletion this ticket's
/// compensation path performs. The id is resolved from a fresh live read;
/// an already-absent row is an idempotent no-op (per-step idempotence law).
async fn delete_cascade_platform(client: &ResinClient, name: &str) -> Result<(), String> {
    let live = client
        .list_platforms()
        .await
        .map_err(|e| format!("list platforms: {e}"))?;
    let Some(id) = crate::resin_client::resolve_id_in(&live, name) else {
        return Ok(()); // already gone — nothing to compensate
    };
    client
        .delete_platform(&id)
        .await
        .map_err(|e| format!("delete platform {name}: {e}"))?;
    tracing::info!(platform = %name, "subscription_pipeline: compensated cascade-created platform (deleted on Resin)");
    Ok(())
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

    #[test]
    fn failed_stage_maps_first_failure_to_section_beat() {
        let ok = StepStatus::Written;
        // Data-landing section: first failure = Import beat, second = Resolve.
        let f_import = StepStatus::Failed("create: 500".to_string());
        let f_resolve = StepStatus::Failed("resolve: nodes empty".to_string());
        let (stage, reason) = failed_stage(EstablishStep::Import, &[&f_import, &f_resolve]);
        assert_eq!(stage, EstablishStep::Import);
        assert_eq!(reason, "create: 500");
        let (stage, reason) = failed_stage(EstablishStep::Import, &[&ok, &f_resolve]);
        assert_eq!(stage, EstablishStep::Resolve);
        assert_eq!(reason, "resolve: nodes empty");
        // Whitebox section: platform beat then bind beat.
        let f_platform = StepStatus::Failed("create platform: 409".to_string());
        let (stage, reason) = failed_stage(EstablishStep::Platform, &[&f_platform, &StepStatus::Failed("x".into())]);
        assert_eq!(stage, EstablishStep::Platform);
        assert_eq!(reason, "create platform: 409");
        let (stage, reason) = failed_stage(EstablishStep::Platform, &[&ok, &StepStatus::Failed("bind: 500".into())]);
        assert_eq!(stage, EstablishStep::Bind);
        assert_eq!(reason, "bind: 500");
        // Apply section: single beat.
        let f_apply = StepStatus::Failed("strategy apply: PATCH 500".to_string());
        let (stage, reason) = failed_stage(EstablishStep::Apply, &[&f_apply]);
        assert_eq!(stage, EstablishStep::Apply);
        assert_eq!(reason, "strategy apply: PATCH 500");
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
                b_class: StrategyId::Balanced,
                manual_nodes: vec![],
                regions: vec![],
                subscriptions: vec!["newsub".into()],
                top_n: 10,
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
            .expect(3) // apply initial + apply per-platform re-read + T04 compensation resolve read
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
        // the compensation deletes the cascade-created platform.
        let m_delete_platform = server
            .mock("DELETE", "/api/v1/platforms/id-flaky")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "id-flaky"}).to_string())
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
        // checkpoint B: the whitebox entry this cascade wrote is L2
        // desired state and SURVIVES the compensation (whitebox deletion is
        // another ticket's path).
        assert!(
            cfg.platforms.iter().any(|ps| ps.platform_name == "flaky"
                && ps.a_class == AClassStrategy::Subscription),
            "whitebox entry must survive the compensation: {cfg:?}"
        );
// checkpoint A branch 1: the failure record persists with the
        // ordered sub/plat/port/apply marking; the cascade-created Resin
        // platform row was deleted.
        let row = cfg.subscriptions.iter().find(|r| r.name == "flaky").expect("status row");
        assert_eq!(row.phase, SubscriptionPhase::Failed);
        assert_eq!(row.stage, Some(EstablishStep::Apply));
        let ce = row.last_cascade_error.as_ref().expect("cascade error record");
        assert_eq!(ce.stage, EstablishStep::Apply);
        assert_eq!(ce.rollback_actions.len(), 4, "strictly one entry per step: {ce:?}");
        assert_eq!(ce.rollback_actions[0], "sub: kept (user data)");
        assert_eq!(ce.rollback_actions[1], "plat: deleted on Resin (cascade-created, apply failed)");
        assert_eq!(ce.rollback_actions[2], "port: none (not run)");
        assert_eq!(ce.rollback_actions[3], "apply: failed on flaky");

        m_subs_absent.assert_async().await;
        m_subs_present.assert_async().await;
        m_create_sub.assert_async().await;
        m_platforms_empty.assert_async().await;
        m_platforms_created.assert_async().await;
        m_create_platform.assert_async().await;
        m_nodes.assert_async().await;
        m_patch.assert_async().await;
        m_delete_platform.assert_async().await;
        let _ = std::fs::remove_file(path);
    }

    /// Checkpoint A branch 2 — sub OK + platform FAIL: the Resin platform
    /// row was never created, so the compensation is a recorded no-op (only
    /// the subscription survives; the whitebox entry written by the cascade's
    /// first half stays as desired state — checkpoint B).
    #[tokio::test]
    async fn pipeline_platform_create_fails_nothing_compensated() {
        let (svc, path) = temp_store("plat-fail");
        let mut server = mockito::Server::new_async().await;
        // Subscription hits: ensure read (absent) + create + resolve read.
        // Apply never runs, so the dangling-ref read does NOT happen.
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
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "s1", "name": "platfail"}).to_string())
            .expect(1)
            .create_async()
            .await;
        let m_subs_present = server
            .mock("GET", "/api/v1/subscriptions")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"id": "s1", "name": "platfail", "node_count": 2}]}).to_string())
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
        // The ONE failing write: POST /platforms 500s (3 attempts with the
        // write retry).
        let m_create_platform = server
            .mock("POST", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(500)
            .expect(3)
            .create_async()
            .await;
        // NO DELETE mock: the compensation must not touch the wire — nothing
        // was created this pass.

        let base = server.url();
        let client = ResinClient::new(&base, "testtok".into()).unwrap();
        let report = run_pipeline(&client, &svc, "platfail", "https://example.invalid/pf.yaml").await;
        assert!(!report.all_ok(), "{report:?}");
        assert!(matches!(report.steps[0], StepStatus::Written), "sub green: {report:?}");
        assert!(matches!(report.steps[2], StepStatus::Written), "whitebox half wrote: {report:?}");
        assert!(matches!(report.steps[3], StepStatus::Failed(_)), "Resin create failed: {report:?}");
        assert!(matches!(report.steps[4], StepStatus::Failed(_)), "apply never ran: {report:?}");
        let cfg = svc.get().unwrap();
        assert!(
            cfg.platforms.iter().any(|ps| ps.platform_name == "platfail"),
            "whitebox entry kept (checkpoint B): {cfg:?}"
        );
        let row = cfg.subscriptions.iter().find(|r| r.name == "platfail").expect("row");
        assert_eq!(row.phase, SubscriptionPhase::Failed);
        assert_eq!(row.stage, Some(EstablishStep::Bind), "Resin-create half maps to the bind beat (T02 ORDER)");
        let ce = row.last_cascade_error.as_ref().expect("cascade error record");
        assert_eq!(ce.rollback_actions.len(), 4);
        assert_eq!(ce.rollback_actions[0], "sub: kept (user data)");
        assert_eq!(ce.rollback_actions[1], "plat: none (not created this pass)");
        assert_eq!(ce.rollback_actions[2], "port: none (not run)");
        assert_eq!(ce.rollback_actions[3], "apply: not reached");

        m_subs_absent.assert_async().await;
        m_create_sub.assert_async().await;
        m_subs_present.assert_async().await;
        m_platforms_empty.assert_async().await;
        m_create_platform.assert_async().await;
        let _ = std::fs::remove_file(path);
    }

    /// Checkpoint A branch 3 — sub FAIL: nothing was created (the presence
    /// read itself failed), so the compensation is a recorded no-op, and the
    /// failure record still persists for the UI.
    #[tokio::test]
    async fn pipeline_subscription_fails_nothing_compensated() {
        let (svc, path) = temp_store("sub-fail");
        let mut server = mockito::Server::new_async().await;
        // The ONE failing read: list subscriptions 500s (3 attempts with the
        // read retry) -> step 1 fails -> everything after is skipped.
        let m_subs_absent = server
            .mock("GET", "/api/v1/subscriptions")
            .match_header(BEARER.0, BEARER.1)
            .with_status(500)
            .expect(3)
            .create_async()
            .await;
        // No other mocks: any further wire call would 404 loudly.

        let base = server.url();
        let client = ResinClient::new(&base, "testtok".into()).unwrap();
        let report = run_pipeline(&client, &svc, "subfail", "https://example.invalid/sf.yaml").await;
        assert!(!report.all_ok(), "{report:?}");
        assert!(matches!(report.steps[0], StepStatus::Failed(_)), "step 1 failed: {report:?}");
        for (i, step) in report.steps.iter().enumerate().skip(1) {
            assert!(matches!(step, StepStatus::Failed(_)), "step {i} skipped-failed: {step:?}");
        }
        let cfg = svc.get().unwrap();
        assert!(cfg.platforms.is_empty(), "nothing written to the whitebox: {cfg:?}");
        let row = cfg.subscriptions.iter().find(|r| r.name == "subfail").expect("row");
        assert_eq!(row.phase, SubscriptionPhase::Failed);
        assert_eq!(row.stage, Some(EstablishStep::Import));
        let ce = row.last_cascade_error.as_ref().expect("cascade error record");
        assert_eq!(
            ce.rollback_actions,
            vec![
                "sub: kept (user data)".to_string(),
                "plat: none (not created this pass)".to_string(),
                "port: none (not run)".to_string(),
                "apply: not reached".to_string(),
            ]
        );
        m_subs_absent.assert_async().await;
        let _ = std::fs::remove_file(path);
    }

    /// Checkpoint B — a whitebox-declared platform is NEVER touched
    /// by the compensation path: the user platform stays in the whitebox AND
    /// on Resin while the cascade-created platform is deleted (branch 1).
    #[tokio::test]
    async fn compensation_never_deletes_whitebox_declared_platform() {
        let (svc, path) = temp_store("ckpt-b-keep");
        // Seed the USER platform (a_class=Region — user-authored, not a
        // cascade product) already converged on Resin.
        svc.store(StrategyConfig {
            platforms: vec![PlatformStrategy {
                platform_name: "user-plat".into(),
                a_class: AClassStrategy::Region,
                b_class: StrategyId::Balanced,
                manual_nodes: vec![],
                regions: vec!["HK".into()],
                subscriptions: vec![],
                top_n: 10,
            }],
            ..Default::default()
        })
        .unwrap();

        let mut server = mockito::Server::new_async().await;
        // Subscription hits: ensure read (absent) + resolve + apply dangling
        // read (present — the cascade adds the guarded ref).
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
            .with_body(json!({"items": [{"id": "s1", "name": "guarded", "node_count": 3}]}).to_string())
            .expect(2)
            .create_async()
            .await;
        let m_create_sub = server
            .mock("POST", "/api/v1/subscriptions")
            .match_header(BEARER.0, BEARER.1)
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "s1", "name": "guarded"}).to_string())
            .expect(1)
            .create_async()
            .await;
        // Platform reads: (1) ensure presence read — user-plat only;
// (2..) apply initial + per-platform re-reads (2 platforms) + the
        // compensation resolve read — user-plat + guarded.
        let m_p_user = server
            .mock("GET", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"id": "id-user", "name": "user-plat", "region_filters": ["HK"]}]}).to_string())
            .expect(1)
            .create_async()
            .await;
        let m_p_both = server
            .mock("GET", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [
                {"id": "id-user", "name": "user-plat", "region_filters": ["HK"]},
                {"id": "id-guarded", "name": "guarded", "region_filters": []}
            ]}).to_string())
            .expect(4)
            .create_async()
            .await;
        let m_create_platform = server
            .mock("POST", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "id-guarded", "name": "guarded"}).to_string())
            .expect(1)
            .create_async()
            .await;
        // One node serves both plans: user-plat (Region HK, in sync — no
        // PATCH) and guarded (subscription tag HK -> drift -> PATCH fails).
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"node_hash": "h1", "region": "HK", "has_outbound": true, "failure_count": 0, "tags": [{"subscription_name": "guarded"}]}]}).to_string())
            .expect(1)
            .create_async()
            .await;
        let m_patch = server
            .mock("PATCH", "/api/v1/platforms/id-guarded")
            .match_header(BEARER.0, BEARER.1)
            .with_status(500)
            .expect(3)
            .create_async()
            .await;
        // ONLY the cascade-created platform is deleted. NO DELETE mock exists
        // for id-user: if the compensation ever touched the user platform it
        // would 404 and flip the recorded action to "delete failed".
        let m_delete = server
            .mock("DELETE", "/api/v1/platforms/id-guarded")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "id-guarded"}).to_string())
            .expect(1)
            .create_async()
            .await;

        let base = server.url();
        let client = ResinClient::new(&base, "testtok".into()).unwrap();
        let report = run_pipeline(&client, &svc, "guarded", "https://example.invalid/guarded.yaml").await;
        assert!(!report.all_ok(), "apply failure must fail the pass: {report:?}");

        let cfg = svc.get().unwrap();
        // Checkpoint B: BOTH whitebox entries survive — the user platform
        // verbatim, the cascade-created entry as desired state for a retry.
        assert!(
            cfg.platforms.iter().any(|ps| ps.platform_name == "user-plat"),
            "user platform must survive: {cfg:?}"
        );
        assert!(cfg.platforms.iter().any(|ps| ps.platform_name == "guarded"));
        let row = cfg.subscriptions.iter().find(|r| r.name == "guarded").expect("status row");
        let ce = row.last_cascade_error.as_ref().expect("cascade error record");
        assert_eq!(ce.rollback_actions[1], "plat: deleted on Resin (cascade-created, apply failed)");
        assert_eq!(ce.rollback_actions[3], "apply: failed on guarded");

        m_subs_absent.assert_async().await;
        m_subs_present.assert_async().await;
        m_create_sub.assert_async().await;
        m_p_user.assert_async().await;
        m_p_both.assert_async().await;
        m_create_platform.assert_async().await;
        m_nodes.assert_async().await;
        m_patch.assert_async().await;
        m_delete.assert_async().await;
        let _ = std::fs::remove_file(path);
    }

    /// Checkpoint B — no overreach: when the apply failure belongs
    /// to an UNRELATED platform, the cascade-created platform is KEPT even
    /// though this pass created it (deleting a resource this cascade did not
    /// fail would churn a converged row on every retry).
    #[tokio::test]
    async fn compensation_keeps_platform_when_failure_owned_elsewhere() {
        let (svc, path) = temp_store("ckpt-b-foreign");
        // Seed the user platform DRIFTING on Resin (computed US vs live []) —
        // its PATCH is the failure owner.
        svc.store(StrategyConfig {
            platforms: vec![PlatformStrategy {
                platform_name: "user-plat".into(),
                a_class: AClassStrategy::Region,
                b_class: StrategyId::Balanced,
                manual_nodes: vec![],
                regions: vec!["US".into()],
                subscriptions: vec![],
                top_n: 10,
            }],
            ..Default::default()
        })
        .unwrap();

        let mut server = mockito::Server::new_async().await;
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
            .with_body(json!({"items": [{"id": "s1", "name": "foreign-ok", "node_count": 3}]}).to_string())
            .expect(2)
            .create_async()
            .await;
        let m_create_sub = server
            .mock("POST", "/api/v1/subscriptions")
            .match_header(BEARER.0, BEARER.1)
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "s1", "name": "foreign-ok"}).to_string())
            .expect(1)
            .create_async()
            .await;
        let m_p_user = server
            .mock("GET", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [{"id": "id-user", "name": "user-plat", "region_filters": []}]}).to_string())
            .expect(1)
            .create_async()
            .await;
        // Apply initial + per-platform re-reads (2 platforms). The
        // compensation makes NO wire call here (nothing to delete), so the
        // count stops at 3.
        let m_p_both = server
            .mock("GET", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [
                {"id": "id-user", "name": "user-plat", "region_filters": []},
                {"id": "id-foreign", "name": "foreign-ok", "region_filters": []}
            ]}).to_string())
            .expect(3)
            .create_async()
            .await;
        let m_create_platform = server
            .mock("POST", "/api/v1/platforms")
            .match_header(BEARER.0, BEARER.1)
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "id-foreign", "name": "foreign-ok"}).to_string())
            .expect(1)
            .create_async()
            .await;
        let m_nodes = server
            .mock("GET", "/api/v1/nodes")
            .match_query(mockito::Matcher::UrlEncoded("limit".into(), "500".into()))
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": [
                {"node_hash": "h-us", "region": "US", "has_outbound": true, "failure_count": 0, "tags": []},
                {"node_hash": "h-hk", "region": "HK", "has_outbound": true, "failure_count": 0, "tags": [{"subscription_name": "foreign-ok"}]}
            ]}).to_string())
            .expect(1)
            .create_async()
            .await;
        // user-plat's PATCH is the failing write; foreign-ok's PATCH lands.
        let m_patch_user = server
            .mock("PATCH", "/api/v1/platforms/id-user")
            .match_header(BEARER.0, BEARER.1)
            .with_status(500)
            .expect(3)
            .create_async()
            .await;
        let m_patch_foreign = server
            .mock("PATCH", "/api/v1/platforms/id-foreign")
            .match_header(BEARER.0, BEARER.1)
            .match_body(mockito::Matcher::PartialJson(json!({"region_filters": ["HK"]})))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "id-foreign"}).to_string())
            .expect(1)
            .create_async()
            .await;
        // NO DELETE mock at all: the failure belongs to user-plat, so the
        // compensation must not touch the wire. Any DELETE would 404 and
        // flip the recorded action away from "kept (apply failure owned by".

        let base = server.url();
        let client = ResinClient::new(&base, "testtok".into()).unwrap();
        let report = run_pipeline(&client, &svc, "foreign-ok", "https://example.invalid/fo.yaml").await;
        assert!(!report.all_ok(), "{report:?}");

        let cfg = svc.get().unwrap();
        assert!(cfg.platforms.iter().any(|ps| ps.platform_name == "user-plat"));
        assert!(
            cfg.platforms.iter().any(|ps| ps.platform_name == "foreign-ok"),
            "cascade-created platform kept (no overreach): {cfg:?}"
        );
        let row = cfg.subscriptions.iter().find(|r| r.name == "foreign-ok").expect("status row");
        let ce = row.last_cascade_error.as_ref().expect("cascade error record");
        assert_eq!(ce.rollback_actions[1], "plat: kept (apply failure owned by user-plat)");
        assert_eq!(ce.rollback_actions[3], "apply: failed on user-plat");

        m_subs_absent.assert_async().await;
        m_subs_present.assert_async().await;
        m_create_sub.assert_async().await;
        m_p_user.assert_async().await;
        m_p_both.assert_async().await;
        m_create_platform.assert_async().await;
        m_nodes.assert_async().await;
        m_patch_user.assert_async().await;
        m_patch_foreign.assert_async().await;
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
                b_class: StrategyId::Balanced,
                manual_nodes: vec![],
                regions: vec![],
                subscriptions: vec!["conv".into()],
                top_n: 10,
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

    // ---- the optional default-port tail ----
    //
    // Three checkpoint branches (A): default-create success / user-provided
    // port / existing same-name port; plus checkpoint B (conflict = warning,
    // never an overwrite, never an error) asserted at both the whitebox and
    // the Resin level. Fixtures are REAL resin-core stores (in-memory
    // DbPool + temp-file WhiteboxConfigStore + dummy PortForwarder — the
    // forwarder no longer binds listeners), so the assertions run against
    // the actual write entry, not a mock.

    async fn port_fixture(
        tag: &str,
    ) -> (DbPool, PortForwarder, WhiteboxConfigStore, std::path::PathBuf) {
        let db = DbPool::open_in_memory().unwrap();
        let forwarder = PortForwarder::new(db.clone(), "127.0.0.1", 1, "");
        let path = std::env::temp_dir().join(format!(
            "sub-pipeline-port-{tag}-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let whitebox = WhiteboxConfigStore::open(
            path.clone(),
            crate::whitebox_config::WhiteboxConfig::from_ports(vec![]),
        )
        .await
        .unwrap();
        (db, forwarder, whitebox, path)
    }

    async fn mock_endpoints_empty(server: &mut mockito::ServerGuard) -> mockito::Mock {
        server
            .mock("GET", "/api/v1/endpoints")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(json!({"items": []}).to_string())
            .expect(1)
            .create_async()
            .await
    }

    /// BRANCH A1 — default create success: no user port, no pre-existing
    /// binding -> the ADR-0031 suggest probe picks a port, the mixed
    /// endpoint POSTs (round-8 D-007: DEFAULT_ENTRY_PORT_PROTOCOL), and the
    /// whitebox (plus its SQLite partner) carries exactly one bound row with
    /// port_upsert's identity defaults.
    #[tokio::test]
    async fn default_port_created_when_user_provides_nothing() {
        let (db, forwarder, whitebox, path) = port_fixture("def-ok").await;
        let mut server = mockito::Server::new_async().await;
        let m_list = mock_endpoints_empty(&mut server).await;
        // The suggested port is a runtime property — the body matcher pins
        // the SHAPE (socks5 proxy listener), not the number.
        let m_create = server
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

        let base = server.url();
        let client = ResinClient::new(&base, "testtok".into()).unwrap();
        let status = ensure_default_port(&client, &db, &forwarder, &whitebox, "newsub", None).await;
        assert!(matches!(status, StepStatus::Written), "default port must be created: {status:?}");

        let rows = db.list_ports().unwrap();
        assert_eq!(rows.len(), 1, "exactly one row: {rows:?}");
        assert_eq!(rows[0].protocol, crate::entry_protocol::DEFAULT_ENTRY_PORT_PROTOCOL);
        assert_eq!(rows[0].platform_name, "newsub");
        assert_eq!(rows[0].account, format!("port-{}", rows[0].port));
        assert!(rows[0].enabled);
        assert!(rows[0].auth_required);
        assert!(rows[0].port >= crate::port_forwarder::MIN_USER_PORT);
        assert_eq!(whitebox.snapshot().entry_ports.len(), 1);

        m_list.assert_async().await;
        m_create.assert_async().await;
        let _ = std::fs::remove_file(path);
    }

    /// BRANCH A2 — user-provided port: the explicit binding target wins and
    /// the suggest probe is skipped (the created row is the user's port,
    /// not the 17990-baseline the suggest would have picked).
    #[tokio::test]
    async fn default_port_user_provided_skips_suggest() {
        let (db, forwarder, whitebox, path) = port_fixture("def-user").await;
        let mut server = mockito::Server::new_async().await;
        let m_list = mock_endpoints_empty(&mut server).await;
        let m_create = server
            .mock("POST", "/api/v1/endpoints")
            .match_header(BEARER.0, BEARER.1)
            .match_body(mockito::Matcher::PartialJson(json!({"port": 24310})))
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(json!({"id": "ep-user"}).to_string())
            .expect(1)
            .create_async()
            .await;

        let base = server.url();
        let client = ResinClient::new(&base, "testtok".into()).unwrap();
        let status =
            ensure_default_port(&client, &db, &forwarder, &whitebox, "newsub", Some(24310)).await;
        assert!(matches!(status, StepStatus::Written), "{status:?}");
        let rows = db.list_ports().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].port, 24310, "user-provided port must win over suggest");

        m_list.assert_async().await;
        m_create.assert_async().await;
        let _ = std::fs::remove_file(path);
    }

    /// BRANCH A3 — existing same-name/same-port entry: the whitebox row
    /// already bound to the platform (or the requested port) wins. The step
    /// short-circuits BEFORE any wire call (no endpoint mocks at all — any
    /// request would 404 and fail the step), warns, and returns
    /// AlreadyPresent; the existing row is byte-identical afterwards
    /// (checkpoint B: warning, not overwrite, not error).
    #[tokio::test]
    async fn default_port_existing_entry_warns_and_skips() {
        let (db, forwarder, whitebox, path) = port_fixture("def-conflict").await;
        let seeded = crate::db::PortMapping {
            port: 18500,
            protocol: "socks5".into(),
            platform_name: "other".into(),
            account: "port-18500".into(),
            label: String::new(),
            enabled: true,
            auth_required: true,
        };
        whitebox
            .apply(
                &db,
                &forwarder,
                crate::whitebox_config::WhiteboxConfig::from_ports(vec![seeded.clone()]),
            )
            .await
            .unwrap();

        // No server at all: the step must not touch the wire. A dead-socket
        // client proves any attempted call would fail loudly.
        let client = ResinClient::new("http://127.0.0.1:1", "t".into()).unwrap();
        // Same-port conflict (requested port held by another platform)...
        let status =
            ensure_default_port(&client, &db, &forwarder, &whitebox, "newsub", Some(18500)).await;
        assert!(
            matches!(status, StepStatus::AlreadyPresent),
            "conflict must skip, not fail: {status:?}"
        );
        // ...and same-PLATFORM conflict (the idempotence face: a row bound
        // to this platform on any port also skips).
        let status2 =
            ensure_default_port(&client, &db, &forwarder, &whitebox, "other", Some(18501)).await;
        assert!(matches!(status2, StepStatus::AlreadyPresent), "{status2:?}");

        let rows = db.list_ports().unwrap();
        assert_eq!(rows.len(), 1, "no row added");
        assert_eq!(rows[0].platform_name, "other", "existing port untouched");
        assert_eq!(rows[0].port, 18500);
        assert_eq!(whitebox.snapshot().entry_ports, vec![seeded], "whitebox untouched");
        let _ = std::fs::remove_file(path);
    }

    /// Checkpoint B (Resin level): a foreign Resin listener on the candidate
    /// port is left INTACT — warn + skip, no POST, no whitebox adoption.
    #[tokio::test]
    async fn default_port_resin_conflict_warns_and_skips() {
        let (db, forwarder, whitebox, path) = port_fixture("def-resin-conflict").await;
        let mut server = mockito::Server::new_async().await;
        let m_list = server
            .mock("GET", "/api/v1/endpoints")
            .match_header(BEARER.0, BEARER.1)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                json!({"items": [{"id": "ep-foreign", "port": 24311, "allow_socks5": true}]})
                    .to_string(),
            )
            .expect(1)
            .create_async()
            .await;
        // NO POST mock: an attempted create would 404 and fail the step.

        let base = server.url();
        let client = ResinClient::new(&base, "testtok".into()).unwrap();
        let status =
            ensure_default_port(&client, &db, &forwarder, &whitebox, "newsub", Some(24311)).await;
        assert!(matches!(status, StepStatus::AlreadyPresent), "{status:?}");
        assert!(db.list_ports().unwrap().is_empty(), "foreign listener must not be adopted");
        assert!(whitebox.snapshot().entry_ports.is_empty());
        m_list.assert_async().await;
        let _ = std::fs::remove_file(path);
    }

    /// §7.5 mirror: a privileged user port never reaches Resin or the
    /// whitebox — warn + skip (the IPC boundary rejects it first; this is
    /// the defensive core-side guard).
    #[tokio::test]
    async fn default_port_privileged_user_port_skips() {
        let (db, forwarder, whitebox, path) = port_fixture("def-priv").await;
        let client = ResinClient::new("http://127.0.0.1:1", "t".into()).unwrap();
        let status = ensure_default_port(&client, &db, &forwarder, &whitebox, "newsub", Some(80)).await;
        assert!(matches!(status, StepStatus::AlreadyPresent), "{status:?}");
        assert!(db.list_ports().unwrap().is_empty());
        assert!(whitebox.snapshot().entry_ports.is_empty());
        let _ = std::fs::remove_file(path);
    }

    /// Idempotence: the second call over the converged world skips via the
    /// platform-binding check BEFORE any wire call (the suggest probe
    /// itself is local, but no endpoint GET/POST may repeat).
    #[tokio::test]
    async fn default_port_idempotent_second_call_zero_writes() {
        let (db, forwarder, whitebox, path) = port_fixture("def-idem").await;
        // Seed pass-1's product directly: one socks5 row bound to newsub.
        whitebox
            .apply(
                &db,
                &forwarder,
                crate::whitebox_config::WhiteboxConfig::from_ports(vec![crate::db::PortMapping {
                    port: 17990,
                    protocol: "socks5".into(),
                    platform_name: "newsub".into(),
                    account: "port-17990".into(),
                    label: String::new(),
                    enabled: true,
                    auth_required: true,
                }]),
            )
            .await
            .unwrap();
        // Dead-socket client: any wire call fails the assertion.
        let client = ResinClient::new("http://127.0.0.1:1", "t".into()).unwrap();
        let status = ensure_default_port(&client, &db, &forwarder, &whitebox, "newsub", None).await;
        assert!(matches!(status, StepStatus::AlreadyPresent), "{status:?}");
        assert_eq!(db.list_ports().unwrap().len(), 1, "zero writes on re-run");
        let _ = std::fs::remove_file(path);
    }

    /// The suggest algorithm: never returns a privileged port, and a port
    /// present in the DbPool's used-set is skipped (17990 seeded -> the
    /// answer must not be 17990).
    #[test]
    fn suggest_free_entry_port_skips_used_and_stays_unprivileged() {
        let db = DbPool::open_in_memory().unwrap();
        // Baseline pick on an empty DB is unprivileged.
        let base = suggest_free_entry_port(&db).unwrap();
        assert!(base >= crate::port_forwarder::MIN_USER_PORT, "got {base}");
        // Seed 17990 as used (the same replace the whitebox write entry
        // performs) — the next pick must skip it.
        db.replace_ports(&[PortMapping {
            port: 17990,
            protocol: "socks5".into(),
            platform_name: "seed".into(),
            account: "port-17990".into(),
            label: String::new(),
            enabled: true,
            auth_required: true,
        }])
        .unwrap();
        let got = suggest_free_entry_port(&db).unwrap();
        assert_ne!(got, 17990, "used port must be skipped");
        assert!(got >= crate::port_forwarder::MIN_USER_PORT, "got {got}");
    }
}
