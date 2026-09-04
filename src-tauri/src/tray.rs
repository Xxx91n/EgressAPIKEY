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
    AppHandle, Manager,
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
    pub show: &'static str,
    pub quit: &'static str,
    pub tooltip: &'static str,
}

/// Ticket 16 / ADR-0054 §E: per-locale copy for the one-shot drift notice.
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
            quit: "Quit",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Zh => TrayLabels {
            show: "显示窗口",
            quit: "退出",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Ja => TrayLabels {
            show: "ウィンドウを表示",
            quit: "終了",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Es => TrayLabels {
            show: "Mostrar Ventana",
            quit: "Salir",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Fr => TrayLabels {
            show: "Afficher la Fenêtre",
            quit: "Quitter",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::De => TrayLabels {
            show: "Fenster anzeigen",
            quit: "Beenden",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Ko => TrayLabels {
            show: "창 표시",
            quit: "종료",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Ru => TrayLabels {
            show: "Показать окно",
            quit: "Выйти",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Pt => TrayLabels {
            show: "Mostrar Janela",
            quit: "Sair",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Ar => TrayLabels {
            show: "إظهار النافذة",
            quit: "إنهاء",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::It => TrayLabels {
            show: "Mostra finestra",
            quit: "Esci",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Nl => TrayLabels {
            show: "Venster tonen",
            quit: "Afsluiten",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Pl => TrayLabels {
            show: "Pokaż okno",
            quit: "Zakończ",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Tr => TrayLabels {
            show: "Pencereyi göster",
            quit: "Çık",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Vi => TrayLabels {
            show: "Hiện cửa sổ",
            quit: "Thoát",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Th => TrayLabels {
            show: "แสดงหน้าต่าง",
            quit: "ออก",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Id => TrayLabels {
            show: "Tampilkan jendela",
            quit: "Keluar",
            tooltip: "EgressAPIKEY",
        },
        TrayLang::Hi => TrayLabels {
            show: "विंडो दिखाएं",
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

/// Ticket 16 / ADR-0054 §E: one-shot drift notify state machine. `armed`
/// starts true; a fire on first unacknowledged drift disarms it; the state
/// re-arms ONLY when a snapshot reports zero unacknowledged drift entries
/// ("归零后再武装"). Process-local static (like DRIFT_MEMORY / RECONCILE_MEMORY):
/// a restart re-arms naturally, which matches the per-process notify-once
/// contract. Acknowledged entities never count as notifyable drift.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DriftNotifyState {
    armed: bool,
}

impl DriftNotifyState {
    /// The process starts armed: the first drift of this run must notify.
    pub fn armed() -> Self {
        Self { armed: true }
    }
}

/// Process-local notify state (ticket 16 / ADR-0054 §E). Static so every
/// authoritative_snapshot call shares the once-per-process contract.
static DRIFT_NOTIFY_STATE: once_cell::sync::Lazy<std::sync::Mutex<DriftNotifyState>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(DriftNotifyState::armed()));

/// Pure transition of the one-shot notify state for one snapshot.
///
/// * `unacknowledged_drift` — whether THIS snapshot contains at least one
///   NOT-acknowledged drift entry (divergent or missingOnResin).
/// * Returns whether the tray should fire the one-shot notice.
///
/// Semantics (issue 16): notify exactly once per process when unacknowledged
/// drift FIRST appears; acknowledged entries never trigger; after a fire the
/// state stays disarmed until a snapshot reports zero unacknowledged drift
/// (re-arm), so a NEW drift episode can notify again. No notification loop.
pub fn evaluate_drift_notice(state: &DriftNotifyState, unacknowledged_drift: bool) -> bool {
    // Fire only on ACTUAL unacknowledged drift while armed. The re-arm on a
    // zero-drift snapshot is state bookkeeping done by the call site — it
    // must never itself emit a notification.
    unacknowledged_drift && state.armed
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
    // Ticket 17 / ADR-0055 D6: route drift joins the same counter; the
    // acknowledged exemption stays read-side (state tag untouched).
    let routes = snap
        .routes
        .iter()
        .filter(|rr| rr.state_tag() != "consistent" && !rr.acknowledged())
        .count();
    platforms + ports + routes
}

/// Fire the one-shot OS drift notification (best-effort). Returns true when
/// the notification was dispatched. Never panics and never blocks the
/// snapshot on a notification failure — a missed toast is logged, not fatal.
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
        let fire = evaluate_drift_notice(&guard, has_drift);
        if fire {
            guard.armed = false;
        } else if !has_drift {
            guard.armed = true;
        }
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
            tracing::info!(target: "tray", "drift notification fired (one-shot, re-arms after drift clears)");
            true
        }
        Err(e) => {
            tracing::warn!(target: "tray", error = %e, "drift notification dispatch failed (best-effort)");
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
        let sep = PredefinedMenuItem::separator(app)?;
        let quit = MenuItem::with_id(app, "quit", l.quit, true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&show, &sep, &quit])?;
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
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", l.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &sep, &quit])?;
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
    /// Ticket 16 / ADR-0054 §E: the notify-once state machine. First drift
    /// fires (armed -> disarm); continuing drift stays silent; zero drift
    /// re-arms; a NEW episode fires again.
    #[test]
    fn evaluate_drift_notice_once_per_episode() {
        let mut state = DriftNotifyState::armed();
        // first drift of the process fires
        assert!(evaluate_drift_notice(&state, true));
        state.armed = false;
        // same episode still drifting: silent
        assert!(!evaluate_drift_notice(&state, true));
        state.armed = false;
        // drift clears: re-arm (no fire on the clear itself)
        assert!(!evaluate_drift_notice(&state, false));
        state.armed = true;
        // new drift episode: fires again
        assert!(evaluate_drift_notice(&state, true));
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
                    b_class: "random".into(),
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
                    b_class: "random".into(),
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
                b_class: "random".into(),
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
}
