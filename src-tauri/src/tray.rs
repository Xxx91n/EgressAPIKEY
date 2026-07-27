//! System tray with i18n labels (en/zh/ja/es/fr) + click-to-show GUI.
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
use tauri::{AppHandle, Manager, tray::{TrayIconBuilder, MouseButton, MouseButtonState, TrayIconEvent}};

/// Supported tray label locales. MUST stay in lockstep with
/// `src/locales/<lc>/common.json` keys `tray.*`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TrayLang { En, Zh, Ja, Es, Fr }

/// Per-locale tray label set.
pub struct TrayLabels {
    pub show: &'static str,
    pub quit: &'static str,
    pub tooltip: &'static str,
}

/// Static, all-frontends-approved label table. Adding a locale in the frontend
/// i18n catalog MUST add a matching row here in the same commit (matches the
/// `pnpm i18n:check` gate on the frontend side).
pub fn labels(lc: TrayLang) -> TrayLabels {
    match lc {
        TrayLang::En => TrayLabels { show: "Show Window", quit: "Quit", tooltip: "ai-api-route" },
        TrayLang::Zh => TrayLabels { show: "显示窗口", quit: "退出", tooltip: "ai-api-route" },
        TrayLang::Ja => TrayLabels { show: "ウィンドウを表示", quit: "終了", tooltip: "ai-api-route" },
        TrayLang::Es => TrayLabels { show: "Mostrar Ventana", quit: "Salir", tooltip: "ai-api-route" },
        TrayLang::Fr => TrayLabels { show: "Afficher la Fenêtre", quit: "Quitter", tooltip: "ai-api-route" },
    }
}

/// Read the active frontend locale from tauri-plugin-store ("lang") and map it
/// to a `TrayLang`. Default English if unreadable or unknown; never panics.
pub fn current_lang(app: &AppHandle) -> TrayLang {
    use tauri_plugin_store::StoreExt;
    let Ok(store) = app.store("settings.json") else { return TrayLang::En; };
    let lang = store.get("lang").and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "en".into());
    match lang.as_str() {
        "zh" => TrayLang::Zh,
        "ja" => TrayLang::Ja,
        "es" => TrayLang::Es,
        "fr" => TrayLang::Fr,
        _ => TrayLang::En,
    }
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
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(l.tooltip));
        let show = MenuItem::with_id(app, "show", l.show, true, None::<&str>)?;
        let sep = PredefinedMenuItem::separator(app)?;
        let quit = MenuItem::with_id(app, "quit", l.quit, true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&show, &sep, &quit])?;
        tray.set_menu(Some(menu))?;
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
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
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
