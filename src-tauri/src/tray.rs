//! System tray with i18n labels (18 base locales) + click-to-show GUI.
//!
//! Label source: a static table in `labels()` (the 5 base locales already
//! shipped by the frontend i18n catalog). The active language is picked from
//! the persisted `tauri-plugin-store` "lang" key when the tray is built, and
//! is refreshed live by the `tray_refresh_labels` Tauri command whenever the
//! user changes the UI locale from Settings — no tray rebuild required.
//!
//! Single-instance already brings the running window to front when a second
//! GUI exe is launched (see main.rs). Clicking the tray icon itself also
//! refocuses; this is the QoL fallback for users who minimized to tray.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri_plugin_notification::NotificationExt;
use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

/// Supported tray label locales (18 base locales). MUST stay in lockstep with
/// `src/locales/<lc>/common.json` keys `tray.*` and the `Locale` union in
/// `src/store/appStore.ts`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayLang {
    En,
    Zh,
    Ja,
    Es,
    Fr,
    De,
    Ko,
    Ru,
    Pt,
    Ar,
    It,
    Nl,
    Pl,
    Tr,
    Vi,
    Th,
    Id,
    Hi,
}

/// Per-locale tray label set.
pub struct TrayLabels {
    pub show: 'static str,
    pub converge_status: 'static str,
    pub quit: 'static str,
    pub tooltip: 'static str,
}

/// ADR-0060 (revising ADR-0054 §E): per-locale copy for the drift-episode
/// edge notice.
pub struct DriftNotice {
    pub title: &'static str,
    pub body: &'static str,
}

/// Static, all-frontends-approved label table. Adding a locale in the frontend
/// i18n catalog MUST add a matching row here in the same commit (matches the
/// `pnpm i18n:check` gate on the frontend side).
pub fn labels(lc: TrayLang) -> TrayLabels {
    match lc {
        TrayLang::En => TrayLabels {
            show: "Show Window",
            converge_status: "Converge Status",
            quit: "Quit",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Zh => TrayLabels {
            show: "显示窗口",
            converge_status: "收敛状态",
            quit: "退出",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Ja => TrayLabels {
            show: "ウィンドウを表示",
            converge_status: "収束ステータス",
            quit: "終了",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Es => TrayLabels {
            show: "Mostrar Ventana",
            converge_status: "Estado de Convergencia",
            quit: "Salir",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Fr => TrayLabels {
            show: "Afficher la Fenêtre",
            converge_status: "État de Convergence",
            quit: "Quitter",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::De => TrayLabels {
            show: "Fenster anzeigen",
            converge_status: "Konvergenzstatus",
            quit: "Beenden",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Ko => TrayLabels {
            show: "창 표시",
            converge_status: "수렴 상태",
            quit: "종료",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Ru => TrayLabels {
            show: "Показать окно",
            converge_status: "Статус конвергенции",
            quit: "Выйти",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Pt => TrayLabels {
            show: "Mostrar Janela",
            converge_status: "Estado de Convergência",
            quit: "Sair",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Ar => TrayLabels {
            show: "إظهار النافذة",
            converge_status: "حالة التقارب",
            quit: "إنهاء",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::It => TrayLabels {
            show: "Mostra finestra",
            converge_status: "Stato di Convergenza",
            quit: "Esci",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Nl => TrayLabels {
            show: "Venster tonen",
            converge_status: "Convergentiestatus",
            quit: "Afsluiten",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Pl => TrayLabels {
            show: "Pokaż okno",
            converge_status: "Status konwergencji",
            quit: "Zakończ",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Tr => TrayLabels {
            show: "Pencereyi göster",
            converge_status: "Yakınsama Durumu",
            quit: "Çık",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Vi => TrayLabels {
            show: "Hiện cửa sổ",
            converge_status: "Trạng thái Hội tụ",
            quit: "Thoát",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Th => TrayLabels {
            show: "แสดงหน้าต่าง",
            converge_status: "สถานะการลู่เข้า",
            quit: "ออก",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Id => TrayLabels {
            show: "Tampilkan jendela",
            converge_status: "Status Konvergensi",
            quit: "Keluar",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Hi => TrayLabels {
            show: "विंडो दिखाएं",
            converge_status: "कन्वर्ज स्थिति",
            quit: "बाहर निकलें",
            tooltip: "EgressAPIKEY",
        },
    }
}

/// Per-locale drift notice copy. 18 rows MUST stay in lockstep with the
/// `tray.driftTitle` / `tray.driftBody` keys in `src/locales/<lc>/common.json`
/// (same lockstep contract as labels()). Trailing-locale rows fall back to
/// English so a missing row degrades, never panics.
pub fn drift_notice(lc: TrayLang) -> DriftNotice {
    match lc {
        TrayLang::En => DriftNotice {
            title: "Config drift detected",
            body: "Your effective configuration drifted from the whitebox. Open Effective Config to review.",
        },
        TrayLang::Zh => DriftNotice {
            title: "检测到配置漂移",
            body: "实际生效配置已偏离白盒期望态。打开生效配置视图查看详情。",
        },
        TrayLang::Ja => DriftNotice {
            title: "設定ドリフトを検出",
            body: "有効な設定が白紙箱の期待状態から逸脱しました。有効設定ビューで確認してください。",
        },
        TrayLang::Es => DriftNotice {
            title: "Deriva de configuración detectada",
            body: "La configuración efectiva se desvió de la caja blanca. Abre Configuración efectiva para revisarla.",
        },
        TrayLang::Fr => DriftNotice {
            title: "Dérive de configuration détectée",
            body: "La configuration effective diverge de la whitebox. Ouvrez Configuration effective pour vérifier.",
        },
        TrayLang::De => DriftNotice {
            title: "Konfigurationsdrift erkannt",
            body: "Die effektive Konfiguration weicht von der Whitebox ab. Öffnen Sie \"Effektive Konfiguration\", um sie zu prüfen.",
        },
        TrayLang::Ko => DriftNotice {
            title: "설정 드리프트 감지됨",
            body: "실제 적용된 설정이 화이트박스와 어긋납니다. 유효 설정 화면에서 확인하세요.",
        },
        TrayLang::Ru => DriftNotice {
            title: "Обнаружено расхождение конфигурации",
            body: "Действующая конфигурация отклонилась от whitebox. Откройте «Эффективную конфигурацию» для проверки.",
        },
        TrayLang::Pt => DriftNotice {
            title: "Deriva de configuração detetada",
            body: "A configuração efetiva divergiu da whitebox. Abra Configuração efetiva para rever.",
        },
        TrayLang::Ar => DriftNotice {
            title: "تم رصد انحراف في الإعدادات",
            body: "انحرفت الإعدادات الفعلية عن المربع الأبيض. افتح \"الإعدادات الفعلية\" للمراجعة.",
        },
        TrayLang::Hi => DriftNotice {
            title: "कॉन्फ़िगरेशन ड्रिफ्ट पाई गई",
            body: "प्रभावी कॉन्फ़िगरेशन व्हाइटबॉक्स से भटक गया है। समीक्षा के लिए प्रभावी कॉन्फ़िगरेशन खोलें।",
        },
        TrayLang::Id => DriftNotice {
            title: "Drift konfigurasi terdeteksi",
            body: "Konfigurasi efektif menyimpang dari whitebox. Buka Konfigurasi Efektif untuk ditinjau.",
        },
        TrayLang::It => DriftNotice {
            title: "Deriva di configurazione rilevata",
            body: "La configurazione effettiva è divergente dalla whitebox. Apri Configurazione effettiva per verificare.",
        },
        TrayLang::Nl => DriftNotice {
            title: "Configuratiedrift gedetecteerd",
            body: "De effectieve configuratie wijkt af van de whitebox. Open Effectieve configuratie om te controleren.",
        },
        TrayLang::Pl => DriftNotice {
            title: "Wykryto dryf konfiguracji",
            body: "Konfiguracja efektywna odbiega od whiteboxa. Otwórz Konfigurację efektywną, aby sprawdzić.",
        },
        TrayLang::Th => DriftNotice {
            title: "ตรวจพบการเลื่อนค่าการตั้งค่า",
            body: "การตั้งค่าที่มีผลจริงคลาดเคลื่อนจาก whitebox เปิดหน้าการตั้งค่าที่มีผลจริงเพื่อตรวจสอบ",
        },
        TrayLang::Tr => DriftNotice {
            title: "Yapılandırma sürüklenmesi algılandı",
            body: "Etkin yapılandırma whitebox'tan saptı. İncelemek için Etkin Yapılandırma'yı açın.",
        },
        TrayLang::Vi => DriftNotice {
            title: "Phát hiện cấu hình trôi dạt",
            body: "Cấu hình hiệu lực đã lệch khỏi whitebox. Mở Cấu hình hiệu lực để xem lại.",
        },
    }
}

/// Read the active frontend locale from tauri-plugin-store ("lang") and map it
/// to a `TrayLang`. Default English if unreadable or unknown; never panics.
/// Map a frontend locale string ("en","zh",...) to a `TrayLang`. Extracted as a
/// pure fn so the locale-key table is unit-testable without an `AppHandle`
/// (which needs a full Tauri runtime). `current_lang` reads the persisted
/// "lang" key and delegates here. Unknown/empty -> English; never panics.
pub fn lang_for_str(s: &str) -> TrayLang {
    match s {
        "en" | "" => TrayLang::En,
        "zh" => TrayLang::Zh,
        "ja" => TrayLang::Ja,
        "es" => TrayLang::Es,
        "fr" => TrayLang::Fr,
        "de" => TrayLang::De,
        "ko" => TrayLang::Ko,
        "ru" => TrayLang::Ru,
        "pt" => TrayLang::Pt,
        "ar" => TrayLang::Ar,
        "it" => TrayLang::It,
        "nl" => TrayLang::Nl,
        "pl" => TrayLang::Pl,
        "tr" => TrayLang::Tr,
        "vi" => TrayLang::Vi,
        "th" => TrayLang::Th,
        "id" => TrayLang::Id,
        "hi" => TrayLang::Hi,
        _ => TrayLang::En,
    }
}

/// ADR-0060 (revising ADR-0054 §E): drift-episode edge notify state. Holds
/// only the previous snapshot's "any unacknowledged drift" boolean; the
/// notification fires on that predicate's false→true rising edge. A
/// process-local static (like DRIFT_MEMORY / RECONCILE_MEMORY): a restart
/// resets to the no-drift baseline (ArgoCD recomputes current state every
/// pass with no cross-process memory), so drift already present at boot
/// notifies once. Acknowledged entities never count as notifyable drift.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DriftNotifyState {
    prev_has_drift: bool,
}

/// Process-local notify state (ADR-0054 §E as revised by ADR-0060). Static so
/// every authoritative_snapshot call shares the per-drift-episode contract.
static DRIFT_NOTIFY_STATE: once_cell::sync::Lazy<std::sync::Mutex<DriftNotifyState>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(DriftNotifyState::default()));

/// Pure edge predicate of the drift-episode notify state (ADR-0060).
///
/// * `has_drift` — whether THIS snapshot contains at least one
///   NOT-acknowledged drift entry (divergent or missingOnResin).
/// * `prev_has_drift` — the previous snapshot's value of the same predicate.
/// * Returns whether the tray should fire the drift notice: only on the
///   false→true rising edge of a drift episode (ArgoCD notifications
///   when+oncePer / AWS Config compliance-transition isomorph).
///
/// Semantics (revising): notify once per drift EPISODE —
/// the first snapshot with unacknowledged drift fires; sustained drift is
/// silent; the falling edge (drift cleared, including via acknowledged
/// exemptions) only updates the baseline and never emits; drift reappearing
/// after a clear starts a new episode and fires again. No notification loop.
pub fn should_fire_drift_notice(has_drift: bool, prev_has_drift: bool) -> bool {
    has_drift && !prev_has_drift
}

/// Count unacknowledged drift entries (strategy + ports halves of the
/// snapshot). Acknowledged entities are exempt (ADR-0054 §D/§E): they stay
/// visible in the view but never trigger the notice.
pub fn count_unacknowledged_drift_entries(
    snap: &resin_core::AuthoritativeSnapshot,
) -> usize {
    let platforms = snap
        .platforms
        .iter()
        .filter(|p| p.state_tag() != "consistent" && !p.acknowledged())
        .count();
    let ports = snap
        .ports
        .iter()
        .filter(|pp| pp.state_tag() != "consistent" && !pp.acknowledged())
        .count();
// ADR-0055 D6: route drift joins the same counter; the
    // acknowledged exemption stays read-side (state tag untouched).
    let routes = snap
        .routes
        .iter()
        .filter(|rr| rr.state_tag() != "consistent" && !rr.acknowledged())
        .count();
    platforms + ports + routes
}

/// Fire the drift-episode OS notification (best-effort, rising edge per
/// ADR-0060). Returns true when the notification was dispatched. Never panics
/// and never blocks the snapshot on a notification failure — a missed toast
/// is logged, not fatal.
pub fn fire_drift_notification(app: &AppHandle, snap: &resin_core::AuthoritativeSnapshot) -> bool {
    // Sidecar down = absence is not drift (ADR-0051): enabled entries report
    // missing_on_resin because there is nothing to compare against. Notifying
    // "your config drifted" while the engine is simply offline would be noise.
    if !snap.resin_reachable {
        return false;
    }
    let has_drift = count_unacknowledged_drift_entries(snap) > 0;
    let should_fire = {
        let mut guard = DRIFT_NOTIFY_STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let fire = should_fire_drift_notice(has_drift, guard.prev_has_drift);
        // Unconditional baseline update on BOTH edges (ADR-0060 F2): the
        // falling edge only moves the baseline, it never emits.
        guard.prev_has_drift = has_drift;
        fire
    };
    if !should_fire {
        return false;
    }
    let lc = current_lang(app);
    let n = drift_notice(lc);
    let sent = app
        .notification()
        .builder()
        .title(n.title)
        .body(n.body)
        .show();
    match sent {
        Ok(_) => {
            tracing::info!(target: "tray", "drift notification fired (drift-episode rising edge, silent while sustained)");
            true
        }
        Err(e) => {
            tracing::warn!(target: "tray", error = %e, "drift notification dispatch failed (best-effort)");
            false
        }
    }
}

/// the tray mirrors the
/// top-level ConvergePhase derived by every `authoritative_snapshot` call
/// (ADR-0058 surface, read-only — the tray never writes config):
///   - Converged   → silent (default window icon, base tooltip);
///   - PendingApply / Drifted → amber (checkpoint C: drift is amber);
///   - ApplyFailed → solid red icon, held visible until the next GREEN apply
/// clears the state (checkpoint C: long-lived visibility);
///   - Unknown (sidecar unreachable) → grey icon, mirroring the GUI dot.
/// Pure state, no I/O: the AppHandle-dependent paint is `apply_converge_mirror`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConvergeMirrorState {
    /// None = not yet painted (boot baseline keeps the default icon).
    prev_phase: Option<resin_core::ConvergePhase>,
}

/// Process-local mirror state, shared by every authoritative_snapshot pass
/// (same discipline as DRIFT_NOTIFY_STATE). Restart recomputes from the
/// boot baseline — no cross-process memory (ArgoCD current-state semantics).
static CONVERGE_MIRROR_STATE: once_cell::sync::Lazy<std::sync::Mutex<ConvergeMirrorState>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(ConvergeMirrorState::default()));

/// Rising-edge predicate (ADR-0060 isomorph): repaint only when the phase
/// CHANGED. A Converged plateau stays silent (no repaint churn every 5s
/// poll); Converged → Drifted repaints; the boot baseline (None) always
/// paints once so the first snapshot after launch carries the real state.
pub fn should_repaint_converge_mirror(
    prev: Option<resin_core::ConvergePhase>,
    next: resin_core::ConvergePhase,
) -> bool {
    prev != Some(next)
}

/// Stable short tag per phase for the tooltip mirror. The richer
/// `{Phase} · rev N/M` format is composed by `converge_mirror_tooltip`.
pub fn converge_mirror_tag(phase: resin_core::ConvergePhase) -> &'static str {
    match phase {
        resin_core::ConvergePhase::NeverApplied => "never applied",
        resin_core::ConvergePhase::PendingApply => "pending apply",
        resin_core::ConvergePhase::ApplyFailed => "apply failed",
        resin_core::ConvergePhase::Converged => "converged",
        resin_core::ConvergePhase::Drifted => "drifted",
        resin_core::ConvergePhase::Unknown => "state unknown",
    }
}

/// Tooltip suffix for a non-silent phase (Converged renders no suffix —
/// checkpoint C: silence is the healthy state, noise only on deviation).
pub fn converge_mirror_tooltip_suffix(phase: resin_core::ConvergePhase) -> Option<&'static str> {
    match phase {
        resin_core::ConvergePhase::Converged => None,
        other => Some(converge_mirror_tag(other)),
    }
}

/// Compose the mirrored tooltip: base locale tooltip + optional state tag
/// and the generation pair `rev N/M` (checkpoint B: abbreviation format
/// `{Phase} · rev {N}/{M}`). Converged stays silent (checkpoint C) —
/// the base tooltip only; every other phase carries its tag + rev pair.
/// Pure so the composition is unit-testable without a tray handle.
fn converge_mirror_tooltip(
    base: &str,
    phase: resin_core::ConvergePhase,
    gen: u64,
    applied_gen: u64,
) -> String {
    match converge_mirror_tooltip_suffix(phase) {
        Some(tag) => format!("{} — {} · rev {}/{}", base, tag, gen, applied_gen),
        None => base.to_string(),
    }
}

/// Paint one 32x32 RGBA block icon at runtime (Ghost safety-net precedent in
/// sidecar.rs `mark_tray_status` — no asset files, no new deps).
fn solid_icon(r: u8, g: u8, b: u8) -> tauri::image::Image<'static> {
    let mut rgba = vec![0u8; 32 * 32 * 4];
    for px in rgba.chunks_exact_mut(4) {
        px[0] = r;
        px[1] = g;
        px[2] = b;
        px[3] = 0xff;
    }
    tauri::image::Image::new_owned(rgba, 32, 32)
}

/// Best-effort paint of the converge mirror onto the tray icon + tooltip.
/// Returns true when a repaint was dispatched. Never panics and never blocks
/// the snapshot on a tray failure — a missed mirror repaint is logged.
pub fn apply_converge_mirror(
    app: &AppHandle,
    phase: resin_core::ConvergePhase,
    gen: u64,
    applied_gen: u64,
) -> bool {
    let should = {
        let mut guard = CONVERGE_MIRROR_STATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let repaint = should_repaint_converge_mirror(guard.prev_phase, phase);
        guard.prev_phase = Some(phase);
        repaint
    };
    if !should {
        return false;
    }
    let Some(tray) = app.tray_by_id("main") else {
        tracing::warn!(target: "tray", "apply_converge_mirror: tray_by_id(\"main\") returned None; mirror not painted");
        return false;
    };
    let result = match phase {
        // Checkpoint C: Converged = silent — restore the branded default icon
        // and the base tooltip. This is ALSO the path that clears a previous
        // red/amber mirror once the world converges again.
        resin_core::ConvergePhase::Converged => {
            let base = labels(current_lang(app)).tooltip;
            match app.default_window_icon() {
                Some(icon) => tray
                    .set_icon(Some(icon.clone()))
                    .and_then(|_| tray.set_tooltip(Some(base))),
                None => tray.set_tooltip(Some(base)),
            }
        }
        // Drifted / PendingApply: amber mirror (checkpoint C: drift is amber).
        resin_core::ConvergePhase::Drifted | resin_core::ConvergePhase::PendingApply => {
            let tip = converge_mirror_tooltip(&labels(current_lang(app)).tooltip, phase, gen, applied_gen);
            tray.set_icon(Some(solid_icon(0xf5, 0x9e, 0x0b)))
                .and_then(|_| tray.set_tooltip(Some(tip)))
        }
        // ApplyFailed: solid red, held until the next GREEN apply flips the
        // phase (checkpoint C: long-lived visibility, not an edge toast).
        resin_core::ConvergePhase::ApplyFailed => {
            let tip = converge_mirror_tooltip(&labels(current_lang(app)).tooltip, phase);
            tray.set_icon(Some(solid_icon(0xd8, 0x2c, 0x2c)))
                .and_then(|_| tray.set_tooltip(Some(tip)))
        }
        // Unknown (sidecar unreachable) and NeverApplied (pre-apply baseline):
        // grey mirror — honest "cannot assert", a deviation but not a failure.
        resin_core::ConvergePhase::Unknown | resin_core::ConvergePhase::NeverApplied => {
            let tip = converge_mirror_tooltip(&labels(current_lang(app)).tooltip, phase);
            tray.set_icon(Some(solid_icon(0x9c, 0xa3, 0xaf)))
                .and_then(|_| tray.set_tooltip(Some(tip)))
        }
    };
    match result {
        Ok(()) => {
            tracing::info!(target: "tray", phase = phase.tag(), "converge mirror painted");
            true
        }
        Err(e) => {
            tracing::warn!(target: "tray", error = ?e, phase = phase.tag(), "converge mirror paint failed (best-effort)");
            false
        }
    }
}

pub fn current_lang(app: &AppHandle) -> TrayLang {
    use tauri_plugin_store::StoreExt;
    let Ok(store) = app.store("settings.json") else {
        return TrayLang::En;
    };
    let lang = store
        .get("lang")
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "en".into());
    lang_for_str(&lang)
}

/// Re-apply the labels in-place after a language change. Called from the
/// `tray_refresh_labels` Tauri command whenever the user switches locale in
/// Settings (Re10). Implemented as a full `set_menu` swap because Tauri 2's
/// `TrayIcon` does not expose a menu getter; rebuilding a 2-item menu is a
/// sub-millisecond op, so this is cheaper than holding Weak<MenuItem> handles
/// across plugin boundaries and just as correct.
pub fn apply_labels(app: &AppHandle) -> tauri::Result<()> {
    let lc = current_lang(app);
    let l = labels(lc);
    // P25-item4: observable diagnostics. The user reported the tray right-click
    // menu shows the PREVIOUS locale after a language switch in Settings. The
    // SettingsView awaits saveLocale then invokes this cmd, so by the time we
    // run current_lang the persisted "lang" SHOULD already be the new value.
    // Without this log we cannot tell whether (a) the invoke never reached
    // here, (b) current_lang read a stale cached value, or (c) tray_by_id
    // returned None so the menu was never rebuilt. All three produce the same
    // user-visible symptom, so we record all three inputs here.
    tracing::info!(target: "tray", lang=?lc, tooltip=l.tooltip, "apply_labels: refreshing tray menu");
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(l.tooltip));
        let show = MenuItem::with_id(app, "show", l.show, true, None::<&str>)?;
        let converge = MenuItem::with_id(app, "converge_status", l.converge_status, true, None::<&str>)?;
        let sep = PredefinedMenuItem::separator(app)?;
        let quit = MenuItem::with_id(app, "quit", l.quit, true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&show, &converge, &sep, &quit])?;
        tray.set_menu(Some(menu))?;
        tracing::info!(target: "tray", show=l.show, quit=l.quit, "apply_labels: menu rebuilt ok");
    } else {
        tracing::warn!(target: "tray", "apply_labels: tray_by_id(\"main\") returned None; menu not rebuilt");
    }
    Ok(())
}

/// Build the tray. Labels are written in the active locale from the start so
/// the first paint is already localised; subsequent language changes call
/// `apply_labels` only.
pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let lc = current_lang(app);
    let l = labels(lc);
    let show = MenuItem::with_id(app, "show", l.show, true, None::<&str>)?;
    let converge = MenuItem::with_id(app, "converge_status", l.converge_status, true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", l.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &converge, &sep, &quit])?;
    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .tooltip(l.tooltip)
        // Bug fix (#5): on Windows the default `show_menu_on_left_click(true)`
        // makes the context menu briefly appear on left-click before our
        // on_tray_icon_event show+focus handler overrides it — the user saw
        // the menu "flash" into view then vanish. Disable native left-click
        // menu so a left click runs only our show+focus handler; the context
        // menu still opens on right-click (the OS default for tray icons).
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "converge_status" => {
                // Checkpoint A: show + focus the window, then emit
                // an event the frontend listens to and navigates to
                // EffectiveConfigView (Tauri event system, not IPC).
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                let _ = app.emit("tray://converge-status", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        // Re10: left-click the tray icon = show+focus the GUI (it is the most
        // common "I minimized to tray, bring it back" gesture).
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.unminimize();
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    /// ADR-0060: the drift-episode edge predicate. Rising edge fires;
    /// sustained drift stays silent; the falling edge never fires; a new
    /// episode after a clear fires again.
    #[test]
    fn should_fire_drift_notice_edge_predicate() {
        // rising edge: no drift -> drift fires
        assert!(should_fire_drift_notice(true, false));
        // sustained drift (flat true): silent
        assert!(!should_fire_drift_notice(true, true));
        // falling edge (flat false, and drift cleared): silent
        assert!(!should_fire_drift_notice(false, false));
        assert!(!should_fire_drift_notice(false, true));
    }

    /// ADR-0060 F5 acknowledge self-consistency: exempting an entity moves
    /// the baseline (falling edge) but must never re-arm into a re-notice;
    /// the NEXT genuinely new drift after the exemption fires again.
    #[test]
    fn acknowledge_updates_baseline_without_renotice() {
        let mut state = DriftNotifyState::default();
        // episode 1: drift appears -> fires
        assert!(should_fire_drift_notice(true, state.prev_has_drift));
        state.prev_has_drift = true;
        // user acknowledges the only drifting entity -> has_drift falls:
        // baseline updates, NO re-notice
        assert!(!should_fire_drift_notice(false, state.prev_has_drift));
        state.prev_has_drift = false;
        // a brand-new unacknowledged drift afterwards -> fires (new episode)
        assert!(should_fire_drift_notice(true, state.prev_has_drift));
    }

    /// A snapshot whose only drift entries are acknowledged MUST NOT notify:
    /// the counter filters them out entirely (ADR-0054 §D/§E).
    #[test]
    fn count_unacknowledged_drift_entries_ignores_acknowledged() {
        use resin_core::snapshot::{PortSnapshot, StrategySnapshot};
        let snap = resin_core::AuthoritativeSnapshot {
            strategy_version: 1,
            platforms: vec![
                StrategySnapshot::Divergent {
                    platform_name: "ack".into(),
                    platform_id: "id".into(),
                    whitebox_regions: vec!["us".into()],
                    resin_regions: vec!["hk".into()],
                    resin_allocation_policy: "BALANCED".into(),
                    b_class: "BALANCED".into(),
                    a_class: "region".into(),
                    manual_nodes: vec![],
                    subscriptions: vec![],
                    divergent_since: None,
                    acknowledged: true,
                },
                StrategySnapshot::MissingOnResin {
                    platform_name: "live".into(),
                    platform_id: String::new(),
                    regions: vec![],
                    a_class: "region".into(),
                    b_class: "BALANCED".into(),
                    manual_nodes: vec![],
                    subscriptions: vec![],
                    divergent_since: None,
                    acknowledged: false,
                },
            ],
            ports: vec![
                PortSnapshot::MissingOnResin {
                    port: 17990,
                    platform_name: "p".into(),
                    protocol: "socks5".into(),
                    account: "a".into(),
                    label: String::new(),
                    auth_required: false,
                    divergent_since: None,
                    acknowledged: true,
                },
                PortSnapshot::Consistent {
                    port: 17991,
                    platform_name: "p".into(),
                    protocol: "socks5".into(),
                    account: "a".into(),
                    label: String::new(),
                    enabled: true,
                    auth_required: false,
                    acknowledged: false,
                },
            ],
            routes: vec![],
            subscriptions: vec![],
            resin_reachable: true,
            last_checked_at: 42,
            strategy_generation: 0,
            strategy_applied_generation: 0,
            converge_phase: resin_core::ConvergePhase::NeverApplied,
            last_apply_at: None,
            last_apply_error: None,
        };
        // only the unacknowledged missing platform counts; the acknowledged
        // divergent platform and the acknowledged missing port are exempt,
        // the consistent port never counted.
        assert_eq!(count_unacknowledged_drift_entries(&snap), 1);
        // all-acknowledged snapshot => zero => notify state sees "no drift".
        let acked = resin_core::AuthoritativeSnapshot {
            platforms: vec![StrategySnapshot::Divergent {
                platform_name: "ack".into(),
                platform_id: "id".into(),
                whitebox_regions: vec!["us".into()],
                resin_regions: vec!["hk".into()],
                resin_allocation_policy: "BALANCED".into(),
                b_class: "BALANCED".into(),
                a_class: "region".into(),
                manual_nodes: vec![],
                subscriptions: vec![],
                divergent_since: None,
                acknowledged: true,
            }],
            ports: vec![PortSnapshot::MissingOnResin {
                port: 17990,
                platform_name: "p".into(),
                protocol: "socks5".into(),
                account: "a".into(),
                label: String::new(),
                auth_required: false,
                divergent_since: None,
                acknowledged: true,
            }],
            routes: vec![],
            subscriptions: vec![],
            resin_reachable: true,
            last_checked_at: 43,
            strategy_generation: 0,
            strategy_applied_generation: 0,
            converge_phase: resin_core::ConvergePhase::NeverApplied,
            last_apply_at: None,
            last_apply_error: None,
            strategy_version: 1,
        };
        assert_eq!(count_unacknowledged_drift_entries(&acked), 0);
    }

    /// Every TrayLang variant must produce a non-empty drift notice copy.
    #[test]
    fn drift_notice_all_variants_non_empty() {
        let all = [
            TrayLang::En, TrayLang::Zh, TrayLang::Ja, TrayLang::Es, TrayLang::Fr, TrayLang::De,
            TrayLang::Ko, TrayLang::Ru, TrayLang::Pt, TrayLang::Ar, TrayLang::It, TrayLang::Nl,
            TrayLang::Pl, TrayLang::Tr, TrayLang::Vi, TrayLang::Th, TrayLang::Id, TrayLang::Hi,
        ];
        for lc in all {
            let n = drift_notice(lc);
            assert!(!n.title.is_empty(), "driftTitle empty for {:?}", lc);
            assert!(!n.body.is_empty(), "driftBody empty for {:?}", lc);
        }
        // distinct titles per locale (no copy-paste collision)
        let mut titles: Vec<&str> = all.iter().map(|lc| drift_notice(*lc).title).collect();
        titles.sort_unstable();
        titles.dedup();
        assert_eq!(titles.len(), all.len(), "drift titles must be distinct per locale");
    }

    use super::*;

    /// Every TrayLang variant must produce a non-empty `show`/`quit` label.
    /// Catches a future row left as "" or a typo'd empty literal.
    #[test]
    fn labels_all_variants_non_empty() {
        let all = [
            TrayLang::En,
            TrayLang::Zh,
            TrayLang::Ja,
            TrayLang::Es,
            TrayLang::Fr,
            TrayLang::De,
            TrayLang::Ko,
            TrayLang::Ru,
            TrayLang::Pt,
            TrayLang::Ar,
            TrayLang::It,
            TrayLang::Nl,
            TrayLang::Pl,
            TrayLang::Tr,
            TrayLang::Vi,
            TrayLang::Th,
            TrayLang::Id,
            TrayLang::Hi,
        ];
        for lc in all {
            let l = labels(lc);
            assert!(!l.show.is_empty(), "show label empty for variant {:?}", lc);
            assert!(!l.quit.is_empty(), "quit label empty for variant {:?}", lc);
            assert_eq!(l.tooltip, "EgressAPIKEY", "tooltip must be the app name");
        }
    }

    /// Each locale gets a distinct `show` string (translations must not
    /// collide by copy-paste). Quit strings are also distinct except where two
    /// locales intentionally share a word; we only assert show distinction.
    #[test]
    fn labels_show_distinct_per_locale() {
        let all = [
            TrayLang::En,
            TrayLang::Zh,
            TrayLang::Ja,
            TrayLang::Es,
            TrayLang::Fr,
            TrayLang::De,
            TrayLang::Ko,
            TrayLang::Ru,
            TrayLang::Pt,
            TrayLang::Ar,
            TrayLang::It,
            TrayLang::Nl,
            TrayLang::Pl,
            TrayLang::Tr,
            TrayLang::Vi,
            TrayLang::Th,
            TrayLang::Id,
            TrayLang::Hi,
        ];
        let shows: Vec<&str> = all.iter().map(|lc| labels(*lc).show).collect();
        let mut dedup = shows.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(
            dedup.len(),
            shows.len(),
            "show labels must be distinct per locale; duplicates found"
        );
    }

    /// The locale-key table must cover every frontend base locale (the 18 in
    /// `src/store/appStore.ts` Locale union). Unknown/garbage -> English.
    #[test]
    fn lang_for_str_covers_all_base_locales() {
        let base = [
            "en", "zh", "ja", "es", "fr", "de", "ko", "ru", "pt", "ar", "it", "nl", "pl", "tr",
            "vi", "th", "id", "hi",
        ];
        // Each maps to the expected variant (round-trip on the canonical key).
        let expect = [
            TrayLang::En,
            TrayLang::Zh,
            TrayLang::Ja,
            TrayLang::Es,
            TrayLang::Fr,
            TrayLang::De,
            TrayLang::Ko,
            TrayLang::Ru,
            TrayLang::Pt,
            TrayLang::Ar,
            TrayLang::It,
            TrayLang::Nl,
            TrayLang::Pl,
            TrayLang::Tr,
            TrayLang::Vi,
            TrayLang::Th,
            TrayLang::Id,
            TrayLang::Hi,
        ];
        for (k, want) in base.iter().zip(expect.iter()) {
            assert_eq!(lang_for_str(k), *want, "lang_for_str({k:?}) mismatch");
        }
    }

    /// Unknown / empty locale keys default to English and never panic.
    #[test]
    fn lang_for_str_unknown_defaults_english() {
        assert_eq!(lang_for_str(""), TrayLang::En);
        assert_eq!(lang_for_str("xx"), TrayLang::En);
        assert_eq!(lang_for_str("EN"), TrayLang::En); // case-sensitive: no implicit upper
        assert_eq!(lang_for_str("zh-CN"), TrayLang::En); // region suffix not matched
    }

    // ─── tray converge-mirror state machine ───────────

    /// Rising-edge predicate: repaint only on phase CHANGE; the boot baseline
    /// (None) always paints once; a plateau (same phase again) stays silent.
    #[test]
    fn converge_mirror_repaints_only_on_phase_change() {
        use resin_core::ConvergePhase;
        // Boot baseline paints once.
        assert!(should_repaint_converge_mirror(None, ConvergePhase::Converged));
        assert!(should_repaint_converge_mirror(None, ConvergePhase::ApplyFailed));
        // Plateau: silent (the 5s poll re-reports the same phase).
        assert!(!should_repaint_converge_mirror(
            Some(ConvergePhase::Converged),
            ConvergePhase::Converged
        ));
        assert!(!should_repaint_converge_mirror(
            Some(ConvergePhase::ApplyFailed),
            ConvergePhase::ApplyFailed
        ));
        // Every transition repaints.
        let phases = [
            ConvergePhase::NeverApplied,
            ConvergePhase::PendingApply,
            ConvergePhase::ApplyFailed,
            ConvergePhase::Converged,
            ConvergePhase::Drifted,
            ConvergePhase::Unknown,
        ];
        for a in phases {
            for b in phases {
                if a != b {
                    assert!(
                        should_repaint_converge_mirror(Some(a), b),
                        "transition {:?} -> {:?} must repaint",
                        a,
                        b
                    );
                }
            }
        }
    }

    /// Tooltip mirror: Converged is silent (checkpoint C); every other phase
    /// carries its stable tag + rev pair after the base tooltip.
    #[test]
    fn converge_mirror_tooltip_suffix_rules() {
        use resin_core::ConvergePhase;
        assert_eq!(converge_mirror_tooltip_suffix(ConvergePhase::Converged), None);
        assert_eq!(
            converge_mirror_tooltip_suffix(ConvergePhase::Drifted),
            Some("drifted")
        );
        assert_eq!(
            converge_mirror_tooltip_suffix(ConvergePhase::ApplyFailed),
            Some("apply failed")
        );
        assert_eq!(
            converge_mirror_tooltip_suffix(ConvergePhase::Unknown),
            Some("state unknown")
        );
        // Composition helper: base + tag + rev for non-silent, bare base for silent.
        assert_eq!(
            converge_mirror_tooltip("EgressAPIKEY", ConvergePhase::Converged, 5, 5),
            "EgressAPIKEY"
        );
        assert_eq!(
            converge_mirror_tooltip("EgressAPIKEY", ConvergePhase::Drifted, 5, 5),
            "EgressAPIKEY — drifted · rev 5/5"
        );
    }

    /// Checkpoint B: tooltip shows the abbreviation format
    /// `{Phase} · rev {N}/{M}` for every non-silent phase.
    #[test]
    fn converge_mirror_tooltip_rev_format() {
        use resin_core::ConvergePhase;
        // PendingApply: desired rev 6, applied rev 5
        assert_eq!(
            converge_mirror_tooltip("EgressAPIKEY", ConvergePhase::PendingApply, 6, 5),
            "EgressAPIKEY — pending apply · rev 6/5"
        );
        // ApplyFailed: same rev pair, different tag
        assert_eq!(
            converge_mirror_tooltip("EgressAPIKEY", ConvergePhase::ApplyFailed, 6, 5),
            "EgressAPIKEY — apply failed · rev 6/5"
        );
        // NeverApplied: rev 0/0 (generation 0 = fresh boot)
        assert_eq!(
            converge_mirror_tooltip("EgressAPIKEY", ConvergePhase::NeverApplied, 0, 0),
            "EgressAPIKEY — never applied · rev 0/0"
        );
        // Unknown: sidecar unreachable, but the rev pair still surfaces
        assert_eq!(
            converge_mirror_tooltip("EgressAPIKEY", ConvergePhase::Unknown, 3, 2),
            "EgressAPIKEY — state unknown · rev 3/2"
        );
        // Converged: silent (base only, no tag, no rev)
        assert_eq!(
            converge_mirror_tooltip("EgressAPIKEY", ConvergePhase::Converged, 7, 7),
            "EgressAPIKEY"
        );
    }

/// Checkpoint C (TFC lesson): ApplyFailed is long-lived — the
    /// red icon stays across consecutive snapshots (plateau = no repaint)
    /// and only clears when a GREEN apply flips the phase to Converged.
    #[test]
    fn converge_mirror_apply_failed_long_lived() {
        use resin_core::ConvergePhase;
        // Episode: apply fails → first paint (red icon).
        assert!(should_repaint_converge_mirror(None, ConvergePhase::ApplyFailed));
        // 5s later: same phase → no repaint (the red STAYS, no edge).
        assert!(!should_repaint_converge_mirror(
            Some(ConvergePhase::ApplyFailed),
            ConvergePhase::ApplyFailed
        ));
        // 10s later: still ApplyFailed → still no repaint.
        assert!(!should_repaint_converge_mirror(
            Some(ConvergePhase::ApplyFailed),
            ConvergePhase::ApplyFailed
        ));
        // User fixes the issue and re-applies → GREEN → phase flips to
        // Converged → repaint (clears the red, restores the branded icon).
        assert!(should_repaint_converge_mirror(
            Some(ConvergePhase::ApplyFailed),
            ConvergePhase::Converged
        ));
    }

    /// Every phase has a distinct, non-empty stable tag (tooltip contract).
    #[test]
    fn converge_mirror_tags_distinct_and_non_empty() {
        use resin_core::ConvergePhase;
        let phases = [
            ConvergePhase::NeverApplied,
            ConvergePhase::PendingApply,
            ConvergePhase::ApplyFailed,
            ConvergePhase::Converged,
            ConvergePhase::Drifted,
            ConvergePhase::Unknown,
        ];
        let mut tags: Vec<&str> = phases.iter().map(|p| converge_mirror_tag(*p)).collect();
        for t in &tags {
            assert!(!t.is_empty(), "tag must be non-empty");
        }
        tags.dedup();
        assert_eq!(tags.len(), phases.len(), "tags must be distinct per phase");
    }

    /// The mirror baseline advances on every evaluation (both edges), so a
    /// phase that reappears after a silent plateau repaints again — same
    /// unconditional-baseline discipline as the ADR-0060 notify state.
    #[test]
    fn converge_mirror_baseline_updates_even_when_silent() {
        use resin_core::ConvergePhase;
        // Pure predicate already covered above; this locks the struct's
        // Default: no phase yet, so the first snapshot always paints.
        let st = ConvergeMirrorState::default();
        assert_eq!(st.prev_phase, None);
        assert!(should_repaint_converge_mirror(st.prev_phase, ConvergePhase::PendingApply));
    }
}
