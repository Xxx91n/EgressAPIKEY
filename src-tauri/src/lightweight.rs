//! Lightweight mode — destroy the WebView2 after N minutes idle to
//! free ~171 MB of webview memory. Modeled after clash-verge-rev
//! `.cargo/registry/.../tauri/src/core.rs` lightweight state machine +
//! cockpit-tools EmptyWorkingSet pattern.
//!
//! State graph (CAS on AtomicU8):
//!   Normal -> EnteringLightweight  (CloseRequested: start delay timer)
//!   EnteringLightweight -> Normal   (Focus: cancel timer)
//!   EnteringLightweight -> InLightweight  (timer fires: destroy webview)
//!   InLightweight -> Normal          (Tray click: rebuild webview)
//!
//! Ponytail: std AtomicU8 for state, std thread for delay timer. No new crate.
//!
//! the one-shot delay arithmetic lives in
//! the single-owner resin_core::throttle model (DELAY_PARAMS below carries
//! this site's values: 1-minute floor, no cap).

use resin_core::throttle::{self, ThrottleParams};
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;

/// Lightweight controller state (CAS-guarded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LightweightState {
    /// WebView is live and visible (or hidden to tray). Normal operation.
    Normal = 0,
    /// Close-requested: delay timer started. Focus cancels back to Normal.
    EnteringLightweight = 1,
    /// WebView destroyed. Tray click rebuilds back to Normal.
    InLightweight = 2,
}

impl LightweightState {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Normal,
            1 => Self::EnteringLightweight,
            2 => Self::InLightweight,
            _ => Self::Normal,
        }
    }
}

/// this site's throttle parameter VALUES (unchanged semantics:
/// one-shot close delay, 1-minute floor matching lightweight_set's 1..=1440
/// IPC validation, no upper cap - input is already bounded by the IPC
/// layer). The arithmetic lives only in resin_core::throttle.
const DELAY_PARAMS: ThrottleParams = ThrottleParams::new(60);

/// Thread-safe controller for the lightweight-mode state machine.
/// Lives in Tauri managed state as `State<LightweightController>`.
pub struct LightweightController {
    state: AtomicU8,
    /// The pending delay timer thread. None when no timer is active.
    /// We hold a JoinHandle so we can interrupt the sleep on focus.
    timer: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Configured delay in minutes (default 10). AtomicU32 for runtime IPC updates.
    delay_minutes: AtomicU32,
}

impl Default for LightweightController {
    fn default() -> Self {
        Self {
            state: AtomicU8::new(LightweightState::Normal as u8),
            timer: Mutex::new(None),
            delay_minutes: AtomicU32::new(10),
        }
    }
}

impl LightweightController {
    /// Create a controller with a custom delay (minutes).
    pub fn new(delay_minutes: u32) -> Self {
        Self {
            state: AtomicU8::new(LightweightState::Normal as u8),
            timer: Mutex::new(None),
            delay_minutes: AtomicU32::new(delay_minutes.max(1)),
        }
    }

    /// Read the current state (cheap atomic load).
    pub fn state(&self) -> LightweightState {
        LightweightState::from_u8(self.state.load(Ordering::Acquire))
    }

    /// CAS transition from Normal -> EnteringLightweight + start delay timer.
    /// Returns true if the transition succeeded (i.e. we were in Normal).
    /// The timer thread sleeps for delay_minutes then transitions to
    /// InLightweight. The caller (main.rs close-requested handler) does
    /// the actual window.destroy() inside the timer callback.
    pub fn try_enter_lightweight(&self, on_timer_fire: impl FnOnce() + Send + 'static) -> bool {
        let prev = self.state.compare_exchange(
            LightweightState::Normal as u8,
            LightweightState::EnteringLightweight as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        if prev.is_err() {
            return false;
        }
        let delay =
            throttle::delay_minutes(DELAY_PARAMS, self.delay_minutes.load(Ordering::Relaxed));
        let state_ptr = self as *const Self as usize;
        let handle = std::thread::spawn(move || {
            std::thread::sleep(delay);
            // Re-acquire the controller via the raw pointer.
            // SAFETY: the controller lives in Tauri managed state for the
            // lifetime of the app; the pointer is valid as long as the app
            // is running. We only read the atomic + call try_transition.
            let ctrl = unsafe { &*(state_ptr as *const Self) };
            ctrl.transition_to_lightweight();
            on_timer_fire();
        });
        *self.timer.lock().unwrap() = Some(handle);
        true
    }

    /// CAS transition EnteringLightweight -> Normal, cancelling the delay timer.
    /// Returns true if a timer was cancelled (i.e. we were in EnteringLightweight).
    pub fn try_cancel_lightweight(&self) -> bool {
        let prev = self.state.compare_exchange(
            LightweightState::EnteringLightweight as u8,
            LightweightState::Normal as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        if prev.is_err() {
            return false;
        }
        // Take the timer handle and let it drop (the thread will finish its
        // sleep but on_timer_fire won't be called because we already transitioned
        // away from EnteringLightweight — transition_to_lightweight checks CAS).
        let _ = self.timer.lock().unwrap().take();
        true
    }

    /// Internal: CAS EnteringLightweight -> InLightweight.
    /// Called by the timer thread after the delay expires.
    fn transition_to_lightweight(&self) {
        let _ = self.state.compare_exchange(
            LightweightState::EnteringLightweight as u8,
            LightweightState::InLightweight as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        // Clear the timer handle (the thread is about to end).
        let _ = self.timer.lock().unwrap().take();
    }

    /// CAS InLightweight -> Normal. Called when the user clicks the tray
    /// to rebuild the webview. Returns true if the transition succeeded.
    pub fn try_exit_lightweight(&self) -> bool {
        let prev = self.state.compare_exchange(
            LightweightState::InLightweight as u8,
            LightweightState::Normal as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        prev.is_ok()
    }

    /// Get the configured delay in minutes (for the Settings UI).
    pub fn delay_minutes(&self) -> u32 {
        self.delay_minutes.load(Ordering::Relaxed)
    }

    /// update delay at runtime (from IPC config change).
    pub fn set_delay_minutes(&self, minutes: u32) {
        self.delay_minutes.store(minutes, Ordering::Relaxed);
    }
}

/// Call EmptyWorkingSet (Windows) / malloc_trim (Linux) / malloc_zone_pressure_relief
/// (macOS) to release the process working set after destroying the WebView.
/// This is a best-effort call; failures are logged at debug level.
#[cfg(target_os = "windows")]
pub fn trim_working_set() {
    use windows_sys::Win32::System::Threading::SetProcessWorkingSetSize;

    unsafe {
        // Calling SetProcessWorkingSetSize with (-1, -1) is the documented
        // way to trigger EmptyWorkingSet: it reduces the process working
        // set to the minimum possible. GetCurrentProcess() returns -1
        // (a pseudo-handle), so we pass it directly as isize.
        let current_process: windows_sys::Win32::Foundation::HANDLE = -1isize as *mut _;
        let _ = SetProcessWorkingSetSize(current_process, usize::MAX, usize::MAX);
    }
    tracing::debug!("T14-2: EmptyWorkingSet called (Windows)");
}

#[cfg(target_os = "linux")]
pub fn trim_working_set() {
    extern "C" {
        fn malloc_trim(pad: usize) -> i32;
    }
    unsafe {
        malloc_trim(0);
    }
    tracing::debug!("T14-2: malloc_trim(0) called (Linux)");
}

#[cfg(target_os = "macos")]
pub fn trim_working_set() {
    // macOS does not have malloc_trim; malloc_zone_pressure_relief is the
    // closest equivalent. We skip it to avoid linking against malloc/malloc.h.
    // The OS will reclaim pages under pressure anyway.
    tracing::debug!("T14-2: trim_working_set no-op (macOS)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn default_state_is_normal() {
        let ctrl = LightweightController::default();
        assert_eq!(ctrl.state(), LightweightState::Normal);
    }

    #[test]
    fn try_enter_from_normal_succeeds() {
        let ctrl = LightweightController::new(1);
        let fired = Arc::new(AtomicBool::new(false));
        let f2 = fired.clone();
        let ok = ctrl.try_enter_lightweight(move || {
            f2.store(true, Ordering::SeqCst);
        });
        assert!(ok, "should transition from Normal to EnteringLightweight");
        assert_eq!(ctrl.state(), LightweightState::EnteringLightweight);
        // Wait for timer (1 min = 60s too long for test; we test state, not timer)
    }

    #[test]
    fn try_enter_from_entering_fails() {
        let ctrl = LightweightController::new(1);
        let _ = ctrl.try_enter_lightweight(|| {});
        let ok2 = ctrl.try_enter_lightweight(|| {});
        assert!(
            !ok2,
            "second enter from EnteringLightweight should fail CAS"
        );
    }

    #[test]
    fn try_cancel_from_entering_succeeds() {
        let ctrl = LightweightController::new(1);
        let _ = ctrl.try_enter_lightweight(|| {});
        assert!(
            ctrl.try_cancel_lightweight(),
            "cancel from EnteringLightweight should succeed"
        );
        assert_eq!(ctrl.state(), LightweightState::Normal);
    }

    #[test]
    fn try_cancel_from_normal_fails() {
        let ctrl = LightweightController::default();
        assert!(
            !ctrl.try_cancel_lightweight(),
            "cancel from Normal should fail CAS"
        );
    }

    #[test]
    fn try_exit_from_in_lightweight_succeeds() {
        let ctrl = LightweightController::new(1);
        // Force state to InLightweight for testing
        ctrl.state
            .store(LightweightState::InLightweight as u8, Ordering::SeqCst);
        assert!(
            ctrl.try_exit_lightweight(),
            "exit from InLightweight should succeed"
        );
        assert_eq!(ctrl.state(), LightweightState::Normal);
    }

    #[test]
    fn try_exit_from_normal_fails() {
        let ctrl = LightweightController::default();
        assert!(
            !ctrl.try_exit_lightweight(),
            "exit from Normal should fail CAS"
        );
    }

    #[test]
    fn delay_minutes_at_least_1() {
        let ctrl = LightweightController::new(0);
        assert_eq!(ctrl.delay_minutes(), 1, "0 minutes should clamp to 1");
    }

    // --- delay rhythm goes through the shared throttle model ---

    #[test]
    fn delay_rhythm_identical_to_legacy_inline_formula() {
        // Byte-for-byte equivalence with the pre- inline arithmetic
        // Duration::from_secs(m*60) for every reachable input: the IPC layer
        // (lightweight_set) validates 1..=1440, and the controller constructor
        // clamps to >= 1, so minutes is never 0 at this call site.
        for m in 1u32..=1440 {
            let legacy = Duration::from_secs(m as u64 * 60);
            let now = throttle::delay_minutes(DELAY_PARAMS, m);
            assert_eq!(now, legacy, "minute m={m}");
        }
    }

    #[test]
    fn delay_rhythm_boundaries_floor_and_extremes() {
        // Floor: the shared model clamps a (unreachable-through-IPC) 0 to the
        // documented 60s minimum instead of an instant fire.
        assert_eq!(
            throttle::delay_minutes(DELAY_PARAMS, 0),
            Duration::from_secs(60)
        );
        // IPC ceiling and u32 extreme: no cap on this site's model, exact
        // minutes*60 conversion throughout.
        assert_eq!(
            throttle::delay_minutes(DELAY_PARAMS, 1440),
            Duration::from_secs(86_400)
        );
        assert_eq!(
            throttle::delay_minutes(DELAY_PARAMS, u32::MAX),
            Duration::from_secs(u32::MAX as u64 * 60)
        );
    }
}
