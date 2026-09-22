//! Platform-level strategy orchestration controller (graded hybrid autonomy).
//!
//! ONE state machine drives both autonomy tiers; the tier only decides
//! whether a proposed transition executes (auto) or parks as a pending
//! proposal awaiting human approval (suggest) — never two code paths.
//!
//! Invariants (all in `OrchestrationParams`, conservative factory values):
//!  - consecutive-failure threshold AND sliding-window failure-rate breach
//!    (slow calls count as failures — proxy risk control surfaces as
//!    challenge/timeout, not explicit 5xx),
//!  - tolerance band: a candidate region must beat the current set by
//!    >= improvement_ratio on ok-share OR >= error_ratio on error share,
//!  - observation period: a switch only cements after M consecutive good
//!    cycles; any regression restarts the count (and can roll back),
//!  - Envoy-style exponential cooldown backoff with a cap,
//!  - per-platform min_switch_interval, global max_switches_per_hour, and
//!    a per-round max_switch_ratio so one tick can never move every
//!    platform at once (avalanche guard).
//!
//! Action surface is deliberately narrow: reversible PATCHes only
//! (region_filters). Deletion, backup rollback, cross-platform fan-out and
//! account-binding changes are NEVER in the controller's repertoire.
//! Every desired-state change lands through the one authoritative write
//! entry (`StrategyService::set_platform_regions` + `apply`) — generation
//! bump + backup ring + audit row included — while pure bookkeeping writes
//! (phase flips, parked proposals) ride the status-subresource path
//! (validated + versioned + audited, no generation bump, same discipline
//! as `record_subscription_phase`).
//!
//! Runtime-only signal windows live in the caller (`commands::orchestration`);
//! this module is pure: serde types + one evaluation pass per tick.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// Hard caps (validator + ingest) — the section is user-editable whitebox
/// content so every array/string needs a bound (ADR-0059 audit discipline).
pub const MAX_ORCH_PLATFORMS: usize = 64;
pub const MAX_SWITCH_LOG: usize = 64;
pub const MAX_PROPOSAL_REASON: usize = 512;
pub const MAX_PROPOSAL_DIFF: usize = 1024;
pub const MAX_REGIONS_PER_SWITCH: usize = 64;
/// Cap on a verdict detail string (ADR-0080) — operator-facing evidence
/// carried on the IPC/audit surface only, never into the ring.
pub const MAX_VERDICT_DETAIL: usize = 64;

/// Autonomy tier. `None` in the persisted params resolves per transport
/// headless default = auto, desktop default = suggest; an explicit
/// value always wins.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Autonomy {
    /// Execute qualifying transitions immediately through the
    /// authoritative write entry.
    Auto,
    /// Park the transition as a pending proposal + diff; it only executes
    /// after `orchestration_approve`.
    Suggest,
}

/// The six-invariant parameter pack. Every field is independently
/// configurable; serde defaults reproduce `Default` so an `orchestration`
/// section written with only `{ "enabled": true }` lands the conservative
/// pack untouched.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrchestrationParams {
    /// Master switch. Absent section or false = controller fully inert.
    #[serde(default)]
    pub enabled: bool,
    /// Explicit autonomy override; None = transport default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autonomy: Option<Autonomy>,
    /// Fast trip: this many consecutively failed tick verdicts degrade a
    /// platform regardless of window rate.
    #[serde(default = "d_fail_streak")]
    pub consecutive_failure_threshold: u32,
    /// Sliding window needs at least this many tick verdicts before the
    /// failure-rate threshold may fire (insufficient evidence = no action).
    #[serde(default = "d_min_samples")]
    pub window_min_samples: u32,
    /// Window verdict failure rate (0..1) that degrades a platform.
    #[serde(default = "d_fail_rate")]
    pub failure_rate_threshold: f64,
    /// A probe slower than this counts as a failed verdict contribution
    /// (slow calls are real risk signals in proxying).
    #[serde(default = "d_slow_ms")]
    pub slow_call_ms: u64,
    /// Tolerance band A: candidate ok_share must be >= current * this.
    #[serde(default = "d_improve")]
    pub improvement_ratio: f64,
    /// Tolerance band B: candidate err_share must be <= current / this.
    #[serde(default = "d_err_ratio")]
    pub error_ratio: f64,
    /// Observation period: consecutive good cycles before a switch cements.
    #[serde(default = "d_good_cycles")]
    pub observe_good_cycles: u32,
    /// Cooldown backoff base seconds (2^streak * base, capped).
    #[serde(default = "d_cd_base")]
    pub cooldown_base_secs: u64,
    /// Cooldown backoff cap (Envoy-style ceiling).
    #[serde(default = "d_cd_max")]
    pub cooldown_max_secs: u64,
    /// Per-round switch cap as a fraction of orchestrated platforms.
    #[serde(default = "d_ratio")]
    pub max_switch_ratio: f64,
    /// Minimum seconds between two switches of the same platform.
    #[serde(default = "d_min_iv")]
    pub min_switch_interval_secs: u64,
    /// Global switch ceiling per rolling hour (all platforms combined).
    #[serde(default = "d_max_hr")]
    pub max_switches_per_hour: u32,
}

fn d_fail_streak() -> u32 {
    3
}
fn d_min_samples() -> u32 {
    8
}
fn d_fail_rate() -> f64 {
    0.5
}
fn d_slow_ms() -> u64 {
    8000
}
fn d_improve() -> f64 {
    1.2
}
fn d_err_ratio() -> f64 {
    2.0
}
fn d_good_cycles() -> u32 {
    3
}
fn d_cd_base() -> u64 {
    60
}
fn d_cd_max() -> u64 {
    1800
}
fn d_ratio() -> f64 {
    0.34
}
fn d_min_iv() -> u64 {
    300
}
fn d_max_hr() -> u32 {
    4
}

impl Default for OrchestrationParams {
    fn default() -> Self {
        Self {
            enabled: false,
            autonomy: None,
            consecutive_failure_threshold: d_fail_streak(),
            window_min_samples: d_min_samples(),
            failure_rate_threshold: d_fail_rate(),
            slow_call_ms: d_slow_ms(),
            improvement_ratio: d_improve(),
            error_ratio: d_err_ratio(),
            observe_good_cycles: d_good_cycles(),
            cooldown_base_secs: d_cd_base(),
            cooldown_max_secs: d_cd_max(),
            max_switch_ratio: d_ratio(),
            min_switch_interval_secs: d_min_iv(),
            max_switches_per_hour: d_max_hr(),
        }
    }
}

/// Per-platform orchestration phase. `Observing` covers the
/// post-switch window (a.k.a. the "switching" beat): the PATCH already
/// landed; the machine watches M cycles before cementing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrchPhase {
    Healthy,
    Degraded,
    Observing,
    Cooldown,
}

/// Persisted pre-switch desired state — the rollback target if the
/// observation window regresses (reversible PATCH, not a backup restore).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrchBaseline {
    pub regions: Vec<String>,
}

/// A parked transition awaiting human approval (suggest tier). The `diff`
/// is a one-line human summary; the authoritative content is `regions`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrchProposal {
    pub regions: Vec<String>,
    pub reason: String,
    pub diff: String,
    pub created_at: u64,
    /// true = this proposal restores `baseline` (a regression rollback);
    /// false = a fresh region switch.
    pub is_rollback: bool,
}

/// Per-platform orchestration STATUS row inside the strategy whitebox's
/// optional `orchestration` section. Bookkeeping only — writes land through
/// the status-subresource path (no generation bump).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlatformOrch {
    pub platform_name: String,
    pub phase: OrchPhase,
    /// Consecutive good observation cycles while Observing.
    #[serde(default)]
    pub good_cycles: u32,
    /// Unix seconds until which the platform is parked in Cooldown.
    #[serde(default)]
    pub cooldown_until: u64,
    /// Cooldown re-entry count driving the exponential backoff.
    #[serde(default)]
    pub cooldown_streak: u32,
    /// Unix seconds of the last executed switch (min_switch_interval).
    #[serde(default)]
    pub last_switch_at: u64,
    /// Switch timestamps (global max_switches_per_hour accounting).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub switch_log: Vec<u64>,
    /// Pre-switch desired state to restore on observation regression.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<OrchBaseline>,
    /// Parked transition awaiting approval (suggest tier only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<OrchProposal>,
}

impl PlatformOrch {
    pub fn new(platform_name: &str) -> Self {
        Self {
            platform_name: platform_name.to_string(),
            phase: OrchPhase::Healthy,
            good_cycles: 0,
            cooldown_until: 0,
            cooldown_streak: 0,
            last_switch_at: 0,
            switch_log: Vec::new(),
            baseline: None,
            pending: None,
        }
    }
}

/// The optional `orchestration` section of `egressapikey-strategy.json`.
/// Absent = controller off (zero migration, same serde-default story as
/// `subscriptions`/`acknowledged`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrchestrationSection {
    #[serde(default)]
    pub params: OrchestrationParams,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub platforms: Vec<PlatformOrch>,
    /// Latest signal verdict per probed platform (ADR-0080 §7): persisted
    /// bookkeeping so consumers project verdicts without riding a tick.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub signal_verdicts: HashMap<String, ProbeVerdict>,
    /// Consecutive environment-suspect ticks (common-mode suppressor).
    #[serde(default, skip_serializing_if = "u32_is_zero")]
    pub suspect_streak: u32,
}

impl Default for OrchestrationSection {
    fn default() -> Self {
        Self {
            params: OrchestrationParams::default(),
            platforms: Vec::new(),
            signal_verdicts: HashMap::new(),
            suspect_streak: 0,
        }
    }
}

fn u32_is_zero(v: &u32) -> bool {
    *v == 0
}

/// This tick's aggregated end-to-end verdict for one platform, computed by
/// the caller from the in-memory sample ring (port_health_check /
/// probe_exit_ip samples). `fail` already includes slow-call contributions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowStats {
    /// Verdicts currently in the sliding window.
    pub samples: u32,
    /// Failed verdicts inside the window (slow calls counted).
    pub fails: u32,
    /// Verdicts failed consecutively at the window's tail.
    pub consecutive_fails: u32,
    /// This tick's own verdict (already pushed into the window by the
    /// caller before evaluation).
    pub tick_failed: bool,
}

/// Typed per-port probe outcome (ADR-0080). The join matrix classifies
/// WHERE a failure lives so a local outage is never rewritten as N remote
/// platform failures.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeVerdict {
    /// Loopback + egress both healthy.
    Ok,
    /// Loopback probe failed (listener down / protocol mismatch / slow) —
    /// the shell-side entry is broken regardless of egress.
    LocalFail,
    /// Loopback ok but the end-to-end egress probe failed.
    RemoteFail,
    /// The port produced no evidence this tick (probe task failure) — an
    /// explicit no-evidence slot, never silently counted either way.
    Skipped,
    /// Stamped by the common-mode suppressor (never by the join): this
    /// tick's failures are attributed to the environment, not the platform.
    EnvironmentSuspect,
}

impl ProbeVerdict {
    /// Counted as a failure by the evaluator's window/streak gates.
    pub fn is_fail(self) -> bool {
        matches!(self, Self::LocalFail | Self::RemoteFail)
    }
    /// Counted in window denominators (real evidence — suspect/skipped are
    /// time barriers, not samples).
    pub fn is_valid(self) -> bool {
        matches!(self, Self::Ok | Self::LocalFail | Self::RemoteFail)
    }
}

/// Per-port join matrix (ADR-0080 §1). The contradiction case (loopback
/// fail + egress ok) classifies as LocalFail; the caller records it in the
/// ≤64B detail field.
pub fn join_probe(loopback_ok: bool, egress_ok: bool) -> ProbeVerdict {
    match (loopback_ok, egress_ok) {
        (true, true) => ProbeVerdict::Ok,
        (true, false) => ProbeVerdict::RemoteFail,
        (false, _) => ProbeVerdict::LocalFail,
    }
}

/// Aggregate one platform's per-port verdicts into its tick verdict
/// (ADR-0080 §2 — supersedes the typeless majority count): skipped holds
/// no evidence and leaves the denominator; a strict majority of fail-class
/// ports over the valid set yields the dominant class (ties → RemoteFail,
/// the actionable class); otherwise Ok.
pub fn aggregate_platform_verdict(verdicts: &[ProbeVerdict]) -> ProbeVerdict {
    let (mut valid, mut local, mut remote) = (0u32, 0u32, 0u32);
    for v in verdicts {
        match v {
            ProbeVerdict::Skipped | ProbeVerdict::EnvironmentSuspect => {}
            ProbeVerdict::Ok => valid += 1,
            ProbeVerdict::LocalFail => {
                valid += 1;
                local += 1;
            }
            ProbeVerdict::RemoteFail => {
                valid += 1;
                remote += 1;
            }
        }
    }
    if valid == 0 {
        return ProbeVerdict::Skipped;
    }
    if (local + remote) * 2 > valid {
        if local > remote {
            ProbeVerdict::LocalFail
        } else {
            ProbeVerdict::RemoteFail
        }
    } else {
        ProbeVerdict::Ok
    }
}

/// Common-mode suppressor (ADR-0080 §3): >=80% of ALL probed bound ports
/// failing loopback in one tick means the environment broke, not the
/// platforms. Denominator = attempted probes (skipped attempts included).
pub fn common_mode_suspect(local_fail_ports: u32, probed_ports: u32) -> bool {
    probed_ports > 0 && local_fail_ports.saturating_mul(5) >= probed_ports.saturating_mul(4)
}

/// Recompute WindowStats from a typed verdict ring. Denominators count
/// valid verdicts only; suspect/skipped entries are time barriers that
/// truncate streaks without entering any rate.
pub fn window_stats(ring: &VecDeque<ProbeVerdict>, tick: ProbeVerdict) -> WindowStats {
    let samples = ring.iter().filter(|v| v.is_valid()).count() as u32;
    let fails = ring.iter().filter(|v| v.is_fail()).count() as u32;
    let consecutive_fails = ring.iter().rev().take_while(|v| v.is_fail()).count() as u32;
    WindowStats {
        samples,
        fails,
        consecutive_fails,
        tick_failed: tick.is_fail(),
    }
}

/// Trailing run of entries equal to `v` — suspect/skipped entries
/// truncate the run (ADR-0080 streak semantics).
pub fn verdict_streak(ring: &VecDeque<ProbeVerdict>, v: ProbeVerdict) -> u32 {
    ring.iter().rev().take_while(|&&x| x == v).count() as u32
}

/// ok / valid share of the ring window; None when no valid samples exist
/// (honest absence beats a fabricated 0 or 1).
pub fn ok_share(ring: &VecDeque<ProbeVerdict>) -> Option<f64> {
    let valid = ring.iter().filter(|v| v.is_valid()).count() as f64;
    if valid == 0.0 {
        None
    } else {
        Some(ring.iter().filter(|&&v| v == ProbeVerdict::Ok).count() as f64 / valid)
    }
}

/// Cap a verdict detail at MAX_VERDICT_DETAIL bytes, char-boundary safe.
pub fn bounded_detail(s: &str) -> String {
    if s.len() <= MAX_VERDICT_DETAIL {
        return s.to_string();
    }
    let mut end = MAX_VERDICT_DETAIL;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Push this tick's `failure_count` sample into the node's ring and return
/// the in-window delta (now - oldest retained sample). The engine counter is
/// cumulative across process lifetime, so a node that recovered long ago
/// still reports failure_count>0 and would poison `ok_share` forever; the
/// delta isolates failures INSIDE the caller's evidence window
/// caliber review). `ring` is caller-owned (process-local per node_hash);
/// `cap` is the window length in samples. Returns 0 on first sight (the
/// sighting establishes the baseline rather than penalizing pre-observation
/// history).
pub fn failure_count_window_delta(ring: &mut VecDeque<i64>, now: i64, cap: usize) -> i64 {
    let oldest = ring.front().copied();
    ring.push_back(now);
    while ring.len() > cap {
        ring.pop_front();
    }
    match oldest {
        Some(o) => now - o,
        None => 0,
    }
}

/// Aggregated node-pool quality for one region (caller computes from
/// `list_nodes`: ok_share = has_outbound && failure_count==0 share).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionMetric {
    pub ok_share: f64,
    pub err_share: f64,
    pub node_count: u32,
}

/// What the evaluation pass wants done. The driver (commands layer)
/// executes it per the resolved autonomy tier: `Switch`/`Restore` become
/// real PATCHes under `auto` and parked proposals under `suggest`.
#[derive(Debug, Clone, PartialEq)]
pub enum OrchAction {
    /// Execute a region switch: set `regions` on the platform, then apply.
    /// `baseline` carries the pre-switch regions for the regression path.
    Switch {
        platform_name: String,
        regions: Vec<String>,
        baseline: OrchBaseline,
        reason: String,
    },
    /// Restore the recorded baseline (observation regression).
    Restore {
        platform_name: String,
        regions: Vec<String>,
        reason: String,
    },
    /// A qualifying transition existed but a gate refused it (hourly
    /// budget / round cap / min interval) — surfaced so the driver can log
    /// gate pressure instead of silently dropping it.
    Blocked {
        platform_name: String,
        reason: String,
    },
    /// Pure bookkeeping: set the row's phase (no spec change).
    Bookkeep { platform_name: String },
}

/// Cap a human-readable proposal diff at MAX_PROPOSAL_DIFF bytes (char
/// boundary-safe). A 64-region platform's full region list can exceed the
/// cap; without this the bookkeeping write fails validation every tick.
fn bounded_diff(s: String) -> String {
    if s.len() <= MAX_PROPOSAL_DIFF {
        return s;
    }
    let mut end = MAX_PROPOSAL_DIFF;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…(+{}B)", &s[..end], s.len() - end)
}

/// One tick of the controller over the whole orchestration section.
/// `verdicts`/`regions` are keyed by platform name; `current_regions`
/// supplies each orchestrated platform's live desired `regions` (the
/// baseline source). Returns the actions to take; the section rows are
/// mutated in place (phase/bookkeeping), so the caller persists the
/// section once per tick.
pub fn evaluate_section(
    sec: &mut OrchestrationSection,
    verdicts: &HashMap<String, WindowStats>,
    region_metrics: &HashMap<String, HashMap<String, RegionMetric>>,
    current_regions: &HashMap<String, Vec<String>>,
    autonomy: Autonomy,
    now: u64,
) -> Vec<OrchAction> {
    let params = sec.params.clone();
    let mut actions = Vec::new();

    // Global hourly budget (rolling window over all rows' switch_log).
    let switches_last_hour: usize = sec
        .platforms
        .iter()
        .flat_map(|p| p.switch_log.iter())
        .filter(|&&ts| now.saturating_sub(ts) < 3600)
        .count();
    let mut budget = (params.max_switches_per_hour as usize).saturating_sub(switches_last_hour);
    // Per-round cap: never move more than max_switch_ratio of the managed
    // set in one pass (minimum 1 so a small fleet can still heal).
    let mut round_cap = ((sec.platforms.len() as f64) * params.max_switch_ratio)
        .floor()
        .max(1.0) as usize;

    for row in sec.platforms.iter_mut() {
        // Parked suggest proposals freeze the row until approved/dismissed.
        if row.pending.is_some() {
            continue;
        }
        let stats = match verdicts.get(&row.platform_name) {
            Some(s) => *s,
            None => continue, // no signal source this tick — no evidence, no action
        };

        match row.phase {
            OrchPhase::Cooldown => {
                if now >= row.cooldown_until {
                    row.phase = OrchPhase::Healthy;
                    row.good_cycles = 0;
                    actions.push(OrchAction::Bookkeep {
                        platform_name: row.platform_name.clone(),
                    });
                }
            }
            OrchPhase::Healthy => {
                let window_breach = stats.samples >= params.window_min_samples
                    && (stats.fails as f64 / stats.samples as f64) >= params.failure_rate_threshold;
                let streak_breach = stats.consecutive_fails >= params.consecutive_failure_threshold;
                if window_breach || streak_breach {
                    row.phase = OrchPhase::Degraded;
                    row.good_cycles = 0;
                    actions.push(OrchAction::Bookkeep {
                        platform_name: row.platform_name.clone(),
                    });
                }
            }
            OrchPhase::Degraded => {
                if stats.tick_failed {
                    let metrics = region_metrics.get(&row.platform_name);
                    let cur_regions = current_regions
                        .get(&row.platform_name)
                        .cloned()
                        .unwrap_or_default();
                    if let Some((region, metric)) = pick_candidate(&cur_regions, metrics, &params) {
                        let allowed = budget > 0
                            && round_cap > 0
                            && now.saturating_sub(row.last_switch_at)
                                >= params.min_switch_interval_secs;
                        if !allowed {
                            // Gate-blocked transitions stay visible: the
                            // driver records every emitted action to the
                            // audit surface — gates block and log.
                            actions.push(OrchAction::Blocked {
                                platform_name: row.platform_name.clone(),
                                reason: "switch gates: hourly budget / round cap / min interval"
                                    .into(),
                            });
                            continue;
                        }
                        let baseline = OrchBaseline {
                            regions: cur_regions.clone(),
                        };
                        let (cur_ok, cur_err) = metrics
                            .map(|m| {
                                (
                                    current_ok_share(&cur_regions, m),
                                    current_err_share(&cur_regions, m),
                                )
                            })
                            .unwrap_or((0.0, 1.0));
                        let reason = format!(
                            "region {region} beats current set (ok {:.0}% vs {:.0}%, err {:.0}% vs {:.0}%)",
                            metric.ok_share * 100.0,
                            cur_ok * 100.0,
                            metric.err_share * 100.0,
                            cur_err * 100.0,
                        );
                        if autonomy == Autonomy::Auto {
                            // Executed now: enter Observing with baseline +
                            // accounting; the driver applies the PATCH.
                            row.phase = OrchPhase::Observing;
                            row.good_cycles = 0;
                            row.last_switch_at = now;
                            row.switch_log.push(now);
                            // keep newest MAX_SWITCH_LOG entries
                            if row.switch_log.len() > MAX_SWITCH_LOG {
                                let cut = row.switch_log.len() - MAX_SWITCH_LOG;
                                row.switch_log.drain(..cut);
                            }
                            row.baseline = Some(baseline.clone());
                            budget = budget.saturating_sub(1);
                            round_cap = round_cap.saturating_sub(1);
                            actions.push(OrchAction::Switch {
                                platform_name: row.platform_name.clone(),
                                regions: vec![region.clone()],
                                baseline,
                                reason,
                            });
                        } else {
                            // Suggest: park the proposal; the row stays
                            // Degraded until a human approves/dismisses.
                            row.pending = Some(OrchProposal {
                                regions: vec![region.clone()],
                                reason: reason.clone(),
                                diff: bounded_diff(format!(
                                    "{}: regions [{}] -> [{}]",
                                    row.platform_name,
                                    cur_regions.join(","),
                                    region
                                )),
                                created_at: now,
                                is_rollback: false,
                            });
                            actions.push(OrchAction::Bookkeep {
                                platform_name: row.platform_name.clone(),
                            });
                        }
                    } else {
                        // Nothing strictly better: park in cooldown so the
                        // machine does not hot-loop re-proposing.
                        cool_down(row, &params, now);
                        actions.push(OrchAction::Bookkeep {
                            platform_name: row.platform_name.clone(),
                        });
                    }
                } else {
                    // Latest tick healthy again — self-healed before acting.
                    row.phase = OrchPhase::Healthy;
                    actions.push(OrchAction::Bookkeep {
                        platform_name: row.platform_name.clone(),
                    });
                }
            }
            OrchPhase::Observing => {
                if stats.tick_failed {
                    // Regression inside the observation window: restore the
                    // recorded baseline (reversible PATCH) then cool down.
                    let baseline = row.baseline.clone();
                    cool_down(row, &params, now);
                    row.good_cycles = 0;
                    match baseline {
                        Some(b) if autonomy == Autonomy::Auto => {
                            actions.push(OrchAction::Restore {
                                platform_name: row.platform_name.clone(),
                                regions: b.regions,
                                reason: "observation regressed".to_string(),
                            });
                        }
                        Some(b) => {
                            row.pending = Some(OrchProposal {
                                regions: b.regions.clone(),
                                reason: "observation regressed".to_string(),
                                diff: bounded_diff(format!(
                                    "{}: rollback regions -> [{}]",
                                    row.platform_name,
                                    b.regions.join(",")
                                )),
                                created_at: now,
                                is_rollback: true,
                            });
                            actions.push(OrchAction::Bookkeep {
                                platform_name: row.platform_name.clone(),
                            });
                        }
                        None => actions.push(OrchAction::Bookkeep {
                            platform_name: row.platform_name.clone(),
                        }),
                    }
                } else {
                    row.good_cycles += 1;
                    if row.good_cycles >= params.observe_good_cycles {
                        // Cemented: back to Healthy, baseline + streaks clear.
                        row.phase = OrchPhase::Healthy;
                        row.baseline = None;
                        row.good_cycles = 0;
                        row.cooldown_streak = 0;
                    }
                    actions.push(OrchAction::Bookkeep {
                        platform_name: row.platform_name.clone(),
                    });
                }
            }
        }
    }
    actions
}

/// Enter cooldown with Envoy-style exponential backoff:
/// `min(base * 2^streak, max)`; the streak increments per entry.
pub fn cool_down(row: &mut PlatformOrch, params: &OrchestrationParams, now: u64) {
    let shift = row.cooldown_streak.min(20);
    let backoff = params
        .cooldown_base_secs
        .saturating_mul(1u64 << shift)
        .min(params.cooldown_max_secs);
    row.phase = OrchPhase::Cooldown;
    row.cooldown_until = now.saturating_add(backoff);
    row.cooldown_streak = row.cooldown_streak.saturating_add(1);
    row.good_cycles = 0;
    row.baseline = None;
    row.pending = None;
}

/// Tolerance-band candidate pick: among regions NOT in the current set,
/// the one with the highest ok_share that is strictly better —
/// ok_share >= current * improvement_ratio OR err_share <= current /
/// error_ratio. Regions the platform already uses never qualify.
fn pick_candidate<'a>(
    current_regions: &[String],
    metrics: Option<&'a HashMap<String, RegionMetric>>,
    params: &OrchestrationParams,
) -> Option<(&'a String, &'a RegionMetric)> {
    let metrics = metrics?;
    let cur_ok = current_ok_share(current_regions, metrics);
    let cur_err = current_err_share(current_regions, metrics);
    metrics
        .iter()
        .filter(|(r, _)| !current_regions.contains(*r))
        .filter(|(_, m)| {
            m.node_count > 0
                && (m.ok_share >= cur_ok * params.improvement_ratio
                    || (cur_err > 0.0 && m.err_share * params.error_ratio <= cur_err))
        })
        .max_by(|a, b| {
            a.1.ok_share
                .partial_cmp(&b.1.ok_share)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn current_ok_share(regions: &[String], metrics: &HashMap<String, RegionMetric>) -> f64 {
    aggregate(regions, metrics).0
}

fn current_err_share(regions: &[String], metrics: &HashMap<String, RegionMetric>) -> f64 {
    aggregate(regions, metrics).1
}

/// Aggregate ok/err share over the region set the platform currently uses;
/// unknown regions contribute nothing (empty set => (0.0, 1.0): treat as
/// fully broken so any measured region qualifies as better).
fn aggregate(regions: &[String], metrics: &HashMap<String, RegionMetric>) -> (f64, f64) {
    let mut nodes = 0u32;
    let mut ok = 0.0f64;
    let mut err = 0.0f64;
    for r in regions {
        if let Some(m) = metrics.get(r) {
            nodes += m.node_count;
            ok += m.ok_share * m.node_count as f64;
            err += m.err_share * m.node_count as f64;
        }
    }
    if nodes == 0 {
        return (0.0, 1.0);
    }
    (ok / nodes as f64, err / nodes as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> OrchestrationParams {
        OrchestrationParams::default()
    }

    fn sec_with(names: &[&str]) -> OrchestrationSection {
        let mut s = OrchestrationSection::default();
        s.params.enabled = true;
        for n in names {
            s.platforms.push(PlatformOrch::new(n));
        }
        s
    }

    fn verdict(samples: u32, fails: u32, streak: u32, tick_failed: bool) -> WindowStats {
        WindowStats {
            samples,
            fails,
            consecutive_fails: streak,
            tick_failed,
        }
    }

    fn metric(ok: f64, err: f64) -> RegionMetric {
        RegionMetric {
            ok_share: ok,
            err_share: err,
            node_count: 10,
        }
    }

    #[test]
    fn failure_count_window_delta_isolates_recent_failures() {
        // cumulative counter: first sight establishes baseline
        let mut ring = VecDeque::new();
        assert_eq!(failure_count_window_delta(&mut ring, 100, 8), 0);
        // static counter (recovered node) -> zero delta, not "err"
        assert_eq!(failure_count_window_delta(&mut ring, 100, 8), 0);
        // counter grows inside the window -> positive delta = failing now
        assert_eq!(failure_count_window_delta(&mut ring, 103, 8), 3);
        // counter stops again -> delta stays positive while the growth
        // sample is inside the window, then ages out
        for _ in 0..7 {
            failure_count_window_delta(&mut ring, 103, 8);
        }
        // ring now holds [103;8] - the 100-sample aged out, delta back to 0
        assert_eq!(failure_count_window_delta(&mut ring, 103, 8), 0);
    }

    #[test]
    fn serde_defaults_off_and_conservative() {
        let p: OrchestrationParams = serde_json::from_str("{}").unwrap();
        assert!(!p.enabled);
        assert_eq!(p.autonomy, None);
        assert_eq!(p.consecutive_failure_threshold, 3);
        assert_eq!(p.observe_good_cycles, 3);
        assert_eq!(p.cooldown_max_secs, 1800);
        let sec: OrchestrationSection = serde_json::from_str("{}").unwrap();
        assert!(!sec.params.enabled);
    }

    #[test]
    fn healthy_to_degraded_on_consecutive_fails() {
        let mut s = sec_with(&["P1"]);
        let mut v = HashMap::new();
        // 3 consecutive fails == threshold
        v.insert("P1".to_string(), verdict(4, 1, 3, true));
        let acts = evaluate_section(
            &mut s,
            &v,
            &HashMap::new(),
            &HashMap::new(),
            Autonomy::Auto,
            1000,
        );
        assert_eq!(s.platforms[0].phase, OrchPhase::Degraded);
        assert_eq!(acts.len(), 1);
    }

    #[test]
    fn healthy_to_degraded_on_window_rate_breach() {
        let mut s = sec_with(&["P1"]);
        let mut v = HashMap::new();
        // window breach: 5/8 = 0.625 >= 0.5, streak below threshold
        v.insert("P1".to_string(), verdict(8, 5, 1, true));
        evaluate_section(
            &mut s,
            &v,
            &HashMap::new(),
            &HashMap::new(),
            Autonomy::Auto,
            1000,
        );
        assert_eq!(s.platforms[0].phase, OrchPhase::Degraded);
    }

    #[test]
    fn insufficient_samples_hold_healthy() {
        let mut s = sec_with(&["P1"]);
        let mut v = HashMap::new();
        // rate breach but window below min_samples AND streak below threshold
        v.insert("P1".to_string(), verdict(4, 4, 2, true));
        evaluate_section(
            &mut s,
            &v,
            &HashMap::new(),
            &HashMap::new(),
            Autonomy::Auto,
            1000,
        );
        assert_eq!(s.platforms[0].phase, OrchPhase::Healthy);
    }

    fn degraded_platform() -> (OrchestrationSection, HashMap<String, WindowStats>) {
        let mut s = sec_with(&["P1"]);
        s.platforms[0].phase = OrchPhase::Degraded;
        let mut v = HashMap::new();
        v.insert("P1".to_string(), verdict(10, 6, 3, true));
        (s, v)
    }

    fn region_metrics() -> HashMap<String, HashMap<String, RegionMetric>> {
        let mut inner = HashMap::new();
        inner.insert("HK".to_string(), metric(0.2, 0.8));
        inner.insert("SG".to_string(), metric(0.9, 0.05));
        let mut m = HashMap::new();
        m.insert("P1".to_string(), inner);
        m
    }

    #[test]
    fn degraded_auto_executes_switch_and_observes() {
        let (mut s, v) = degraded_platform();
        let mut cur = HashMap::new();
        cur.insert("P1".to_string(), vec!["HK".to_string()]);
        let acts = evaluate_section(&mut s, &v, &region_metrics(), &cur, Autonomy::Auto, 1000);
        assert_eq!(s.platforms[0].phase, OrchPhase::Observing);
        assert_eq!(
            s.platforms[0].baseline.as_ref().unwrap().regions,
            vec!["HK".to_string()]
        );
        assert!(
            matches!(&acts[0], OrchAction::Switch { regions, .. } if regions == &vec!["SG".to_string()])
        );
        assert_eq!(s.platforms[0].switch_log, vec![1000]);
    }

    #[test]
    fn degraded_suggest_parks_proposal() {
        let (mut s, v) = degraded_platform();
        let mut cur = HashMap::new();
        cur.insert("P1".to_string(), vec!["HK".to_string()]);
        let acts = evaluate_section(&mut s, &v, &region_metrics(), &cur, Autonomy::Suggest, 1000);
        // parked: phase stays Degraded, pending holds the diff, NO Switch action
        assert_eq!(s.platforms[0].phase, OrchPhase::Degraded);
        let p = s.platforms[0].pending.as_ref().unwrap();
        assert_eq!(p.regions, vec!["SG".to_string()]);
        assert!(!p.is_rollback);
        assert!(p.diff.contains("HK") && p.diff.contains("SG"));
        assert!(acts
            .iter()
            .all(|a| matches!(a, OrchAction::Bookkeep { .. })));
    }

    #[test]
    fn pending_proposal_freezes_row() {
        let (mut s, v) = degraded_platform();
        s.platforms[0].pending = Some(OrchProposal {
            regions: vec!["SG".to_string()],
            reason: "r".into(),
            diff: "d".into(),
            created_at: 1,
            is_rollback: false,
        });
        let mut cur = HashMap::new();
        cur.insert("P1".to_string(), vec!["HK".to_string()]);
        let acts = evaluate_section(&mut s, &v, &region_metrics(), &cur, Autonomy::Auto, 1000);
        assert!(acts.is_empty());
        assert_eq!(s.platforms[0].phase, OrchPhase::Degraded);
    }

    #[test]
    fn tolerance_band_blocks_marginal_candidate() {
        let (mut s, v) = degraded_platform();
        let mut inner = HashMap::new();
        inner.insert("HK".to_string(), metric(0.9, 0.05));
        inner.insert("SG".to_string(), metric(0.95, 0.04)); // only 5% better — under the band
        let mut m = HashMap::new();
        m.insert("P1".to_string(), inner);
        let mut cur = HashMap::new();
        cur.insert("P1".to_string(), vec!["HK".to_string()]);
        evaluate_section(&mut s, &v, &m, &cur, Autonomy::Auto, 1000);
        // no qualifying candidate -> cooldown, not a switch
        assert_eq!(s.platforms[0].phase, OrchPhase::Cooldown);
        assert!(s.platforms[0].cooldown_until > 1000);
    }

    #[test]
    fn cooldown_expiry_returns_healthy_and_backoff_grows() {
        let mut s = sec_with(&["P1"]);
        s.platforms[0].phase = OrchPhase::Degraded;
        let mut v = HashMap::new();
        v.insert("P1".to_string(), verdict(10, 9, 9, true));
        let m = region_metrics();
        // remove candidates entirely -> cooldown entry
        let mut cur = HashMap::new();
        cur.insert("P1".to_string(), vec!["HK".to_string()]);
        let empty: HashMap<String, HashMap<String, RegionMetric>> = HashMap::new();
        evaluate_section(&mut s, &v, &empty, &cur, Autonomy::Auto, 1000);
        let until1 = s.platforms[0].cooldown_until;
        assert_eq!(s.platforms[0].phase, OrchPhase::Cooldown);
        // still cooling -> no transition
        evaluate_section(&mut s, &v, &empty, &cur, Autonomy::Auto, until1 - 1);
        assert_eq!(s.platforms[0].phase, OrchPhase::Cooldown);
        // expired -> Healthy
        evaluate_section(&mut s, &v, &empty, &cur, Autonomy::Auto, until1);
        assert_eq!(s.platforms[0].phase, OrchPhase::Healthy);
        // second cooldown entry backs off 2x
        s.platforms[0].phase = OrchPhase::Degraded;
        evaluate_section(&mut s, &v, &empty, &cur, Autonomy::Auto, until1 + 10);
        let until2 = s.platforms[0].cooldown_until;
        assert!(until2 - (until1 + 10) > until1 - 1000);
    }

    #[test]
    fn observing_cements_after_m_good_cycles() {
        let mut s = sec_with(&["P1"]);
        s.platforms[0].phase = OrchPhase::Observing;
        s.platforms[0].baseline = Some(OrchBaseline {
            regions: vec!["HK".to_string()],
        });
        let mut v = HashMap::new();
        v.insert("P1".to_string(), verdict(10, 0, 0, false));
        for _ in 0..2 {
            evaluate_section(
                &mut s,
                &v,
                &HashMap::new(),
                &HashMap::new(),
                Autonomy::Auto,
                1000,
            );
            assert_eq!(s.platforms[0].phase, OrchPhase::Observing);
        }
        evaluate_section(
            &mut s,
            &v,
            &HashMap::new(),
            &HashMap::new(),
            Autonomy::Auto,
            1000,
        );
        assert_eq!(s.platforms[0].phase, OrchPhase::Healthy);
        assert!(s.platforms[0].baseline.is_none());
    }

    #[test]
    fn observing_regression_restores_baseline_auto() {
        let mut s = sec_with(&["P1"]);
        s.platforms[0].phase = OrchPhase::Observing;
        s.platforms[0].baseline = Some(OrchBaseline {
            regions: vec!["HK".to_string()],
        });
        let mut v = HashMap::new();
        v.insert("P1".to_string(), verdict(10, 8, 3, true));
        let acts = evaluate_section(
            &mut s,
            &v,
            &HashMap::new(),
            &HashMap::new(),
            Autonomy::Auto,
            1000,
        );
        assert!(
            matches!(&acts[0], OrchAction::Restore { regions, .. } if regions == &vec!["HK".to_string()])
        );
        assert_eq!(s.platforms[0].phase, OrchPhase::Cooldown);
    }

    #[test]
    fn observing_regression_parks_rollback_suggest() {
        let mut s = sec_with(&["P1"]);
        s.platforms[0].phase = OrchPhase::Observing;
        s.platforms[0].baseline = Some(OrchBaseline {
            regions: vec!["HK".to_string()],
        });
        let mut v = HashMap::new();
        v.insert("P1".to_string(), verdict(10, 8, 3, true));
        evaluate_section(
            &mut s,
            &v,
            &HashMap::new(),
            &HashMap::new(),
            Autonomy::Suggest,
            1000,
        );
        let p = s.platforms[0].pending.as_ref().unwrap();
        assert!(p.is_rollback);
        assert_eq!(p.regions, vec!["HK".to_string()]);
    }

    #[test]
    fn min_switch_interval_blocks_rapid_reswitch() {
        let (mut s, v) = degraded_platform();
        s.platforms[0].last_switch_at = 900; // 100s ago < 300s min interval
        let mut cur = HashMap::new();
        cur.insert("P1".to_string(), vec!["HK".to_string()]);
        let acts = evaluate_section(&mut s, &v, &region_metrics(), &cur, Autonomy::Auto, 1000);
        assert!(acts.iter().all(|a| !matches!(a, OrchAction::Switch { .. })));
        assert_eq!(s.platforms[0].phase, OrchPhase::Degraded);
    }

    #[test]
    fn max_switches_per_hour_global_budget() {
        let mut s = sec_with(&["P1", "P2"]);
        for r in s.platforms.iter_mut() {
            r.phase = OrchPhase::Degraded;
        }
        // exhaust the hourly budget with P1's log (4 entries < 3600s old)
        s.platforms[0].switch_log = vec![900, 910, 920, 930];
        let stats = verdict(10, 9, 9, true);
        let v: HashMap<String, WindowStats> =
            [("P1".to_string(), stats), ("P2".to_string(), stats)]
                .into_iter()
                .collect();
        let mut m = HashMap::new();
        for p in ["P1", "P2"] {
            let mut inner = HashMap::new();
            inner.insert("SG".to_string(), metric(0.95, 0.02));
            m.insert(p.to_string(), inner);
        }
        let cur: HashMap<String, Vec<String>> = [
            ("P1".to_string(), vec!["HK".to_string()]),
            ("P2".to_string(), vec!["HK".to_string()]),
        ]
        .into_iter()
        .collect();
        let acts = evaluate_section(&mut s, &v, &m, &cur, Autonomy::Auto, 1000);
        // global budget exhausted -> no switches at all
        assert!(acts.iter().all(|a| !matches!(a, OrchAction::Switch { .. })));
    }

    #[test]
    fn round_cap_limits_switches_per_tick() {
        let mut s = sec_with(&["P1", "P2", "P3", "P4"]);
        for r in s.platforms.iter_mut() {
            r.phase = OrchPhase::Degraded;
        }
        let stats = verdict(10, 9, 9, true);
        let v: HashMap<String, WindowStats> = ["P1", "P2", "P3", "P4"]
            .iter()
            .map(|p| (p.to_string(), stats))
            .collect();
        let mut m = HashMap::new();
        for p in ["P1", "P2", "P3", "P4"] {
            let mut inner = HashMap::new();
            inner.insert("SG".to_string(), metric(0.95, 0.02));
            m.insert(p.to_string(), inner);
        }
        let cur: HashMap<String, Vec<String>> = ["P1", "P2", "P3", "P4"]
            .iter()
            .map(|p| (p.to_string(), vec!["HK".to_string()]))
            .collect();
        let acts = evaluate_section(&mut s, &v, &m, &cur, Autonomy::Auto, 1000);
        // 4 platforms * 0.34 = floor 1 -> exactly one switch this round
        let switches = acts
            .iter()
            .filter(|a| matches!(a, OrchAction::Switch { .. }))
            .count();
        assert_eq!(switches, 1);
    }

    #[test]
    fn degraded_self_heals_when_tick_recovers() {
        let (mut s, _) = degraded_platform();
        let mut v = HashMap::new();
        v.insert("P1".to_string(), verdict(10, 2, 0, false));
        evaluate_section(
            &mut s,
            &v,
            &region_metrics(),
            &HashMap::new(),
            Autonomy::Auto,
            1000,
        );
        assert_eq!(s.platforms[0].phase, OrchPhase::Healthy);
    }

    #[test]
    fn proposal_diff_is_capped() {
        let big = format!("P: regions [{}] -> [US]", "R".repeat(MAX_PROPOSAL_DIFF * 3));
        let capped = bounded_diff(big);
        assert!(capped.len() <= MAX_PROPOSAL_DIFF + 16);
        assert!(capped.contains("(+"));
        let small = "x: regions [HK] -> [SG]".to_string();
        assert_eq!(bounded_diff(small.clone()), small);
    }

    #[test]
    fn oversized_region_list_proposal_still_validates() {
        // A platform with a huge current region set must not produce a
        // pending proposal that fails validation on the next persist.
        let (mut s, _) = degraded_platform();
        s.platforms[0].phase = OrchPhase::Degraded;
        let mut cur = HashMap::new();
        cur.insert(
            "P1".to_string(),
            (0..64)
                .map(|i| format!("REGION-{i:02}-aaaaaaaaaaaaaaaaaaaa"))
                .collect(),
        );
        let mut metrics = HashMap::new();
        metrics.insert(
            "P1".to_string(),
            HashMap::from([(
                "SG".to_string(),
                RegionMetric {
                    ok_share: 1.0,
                    err_share: 0.0,
                    node_count: 4,
                },
            )]),
        );
        let mut v = HashMap::new();
        v.insert("P1".to_string(), verdict(10, 3, 3, true));
        evaluate_section(&mut s, &v, &metrics, &cur, Autonomy::Suggest, 1000);
        let p = s.platforms[0].pending.as_ref().expect("proposal parked");
        assert!(p.diff.len() <= MAX_PROPOSAL_DIFF + 16);
    }

    // ---- ADR-0080 signal-plane verdicts ----

    use ProbeVerdict as PV;

    fn ring_of(vs: &[PV]) -> VecDeque<PV> {
        vs.iter().copied().collect()
    }

    #[test]
    fn join_probe_matrix() {
        // legislated join: loopback x egress -> typed verdict
        assert_eq!(join_probe(true, true), PV::Ok);
        assert_eq!(join_probe(true, false), PV::RemoteFail);
        assert_eq!(join_probe(false, false), PV::LocalFail);
        // contradiction (loopback dead but egress flowed) classifies local
        assert_eq!(join_probe(false, true), PV::LocalFail);
    }

    #[test]
    fn aggregate_platform_verdict_dominant_class_and_valid_denominator() {
        // fail majority over VALID ports -> dominant fail class
        assert_eq!(
            aggregate_platform_verdict(&[PV::RemoteFail, PV::RemoteFail, PV::Ok]),
            PV::RemoteFail
        );
        assert_eq!(
            aggregate_platform_verdict(&[PV::LocalFail, PV::LocalFail, PV::RemoteFail]),
            PV::LocalFail
        );
        // fail classes tie -> remote_fail (the actionable class)
        assert_eq!(
            aggregate_platform_verdict(&[PV::LocalFail, PV::RemoteFail]),
            PV::RemoteFail
        );
        // no strict majority -> ok (one bad port must not trip a platform)
        assert_eq!(
            aggregate_platform_verdict(&[PV::RemoteFail, PV::Ok, PV::Ok]),
            PV::Ok
        );
        // skipped ports leave the denominator: 1 skipped + 1 fail =
        // 1/1 valid failing -> fail
        assert_eq!(
            aggregate_platform_verdict(&[PV::Skipped, PV::RemoteFail]),
            PV::RemoteFail
        );
        // all-skipped = no evidence at all
        assert_eq!(
            aggregate_platform_verdict(&[PV::Skipped, PV::Skipped]),
            PV::Skipped
        );
    }

    #[test]
    fn common_mode_suspect_threshold_table() {
        // n<=4 requires unanimity (integer behavior of the >=80% rule)
        for n in 1..=4u32 {
            assert!(!common_mode_suspect(n - 1, n), "n={n} at n-1");
            assert!(common_mode_suspect(n, n), "n={n} unanimous");
        }
        // n=5: 4/5 = 80% fires, 3/5 does not
        assert!(common_mode_suspect(4, 5));
        assert!(!common_mode_suspect(3, 5));
        // n=10: 8/10 fires, 7/10 does not
        assert!(common_mode_suspect(8, 10));
        assert!(!common_mode_suspect(7, 10));
        // no probed ports -> never suspect
        assert!(!common_mode_suspect(0, 0));
    }

    #[test]
    fn window_stats_valid_denominators_and_suspect_truncation() {
        // ring: ok, local_fail, suspect, remote_fail, local_fail
        let ring = ring_of(&[
            PV::Ok,
            PV::LocalFail,
            PV::EnvironmentSuspect,
            PV::RemoteFail,
            PV::LocalFail,
        ]);
        let st = window_stats(&ring, PV::LocalFail);
        assert_eq!(st.samples, 4, "suspect excluded from denominator");
        assert_eq!(st.fails, 3);
        assert_eq!(st.consecutive_fails, 2, "suspect truncates the run");
        assert!(st.tick_failed);
        let st2 = window_stats(&ring, PV::EnvironmentSuspect);
        assert!(!st2.tick_failed, "a suspect tick is never a platform fail");
        // all-barrier window: zero valid evidence
        let ring2 = ring_of(&[PV::EnvironmentSuspect, PV::Skipped]);
        let st3 = window_stats(&ring2, PV::Skipped);
        assert_eq!(st3.samples, 0);
        assert_eq!(st3.fails, 0);
        assert_eq!(st3.consecutive_fails, 0);
    }

    #[test]
    fn verdict_streak_truncates_at_suspect() {
        let ring = ring_of(&[PV::LocalFail, PV::EnvironmentSuspect, PV::LocalFail, PV::LocalFail]);
        assert_eq!(verdict_streak(&ring, PV::LocalFail), 2);
        assert_eq!(verdict_streak(&ring, PV::EnvironmentSuspect), 0);
        let ring2 = ring_of(&[PV::LocalFail, PV::EnvironmentSuspect, PV::EnvironmentSuspect]);
        assert_eq!(verdict_streak(&ring2, PV::EnvironmentSuspect), 2);
    }

    #[test]
    fn ok_share_uses_valid_denominator_or_none() {
        let ring = ring_of(&[PV::Ok, PV::EnvironmentSuspect, PV::RemoteFail]);
        assert_eq!(ok_share(&ring), Some(0.5));
        let empty = ring_of(&[PV::EnvironmentSuspect, PV::Skipped]);
        assert_eq!(ok_share(&empty), None);
    }

    #[test]
    fn bounded_detail_caps_at_64_bytes_char_safe() {
        let long = "x".repeat(200);
        let d = bounded_detail(&long);
        assert!(d.len() <= MAX_VERDICT_DETAIL + 4);
        assert!(d.ends_with('…'));
        // multibyte boundary: 64B cut inside a char must step back
        let mb = "a".repeat(63) + "中中";
        let d2 = bounded_detail(&mb);
        assert!(d2.len() <= MAX_VERDICT_DETAIL + 4);
        assert!(d2.is_char_boundary(d2.len() - '…'.len_utf8()));
        let short = "p18080 eg st=200 lat=812ms";
        assert_eq!(bounded_detail(short), short);
    }

    #[test]
    fn signal_verdicts_serde_roundtrip() {
        let mut sec = OrchestrationSection::default();
        sec.signal_verdicts
            .insert("P1".to_string(), ProbeVerdict::EnvironmentSuspect);
        sec.suspect_streak = 3;
        let v = serde_json::to_value(&sec).unwrap();
        assert_eq!(
            v["signal_verdicts"]["P1"],
            serde_json::json!("environment_suspect")
        );
        assert_eq!(v["suspect_streak"], serde_json::json!(3));
        let back: OrchestrationSection = serde_json::from_value(v).unwrap();
        assert_eq!(
            back.signal_verdicts["P1"],
            ProbeVerdict::EnvironmentSuspect
        );
        // absent fields default clean (zero-migration discipline)
        let bare: OrchestrationSection = serde_json::from_str("{}").unwrap();
        assert!(bare.signal_verdicts.is_empty());
        assert_eq!(bare.suspect_streak, 0);
    }
}
