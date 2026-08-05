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
use tauri::{AppHandle, Manager, tray::{TrayIconBuilder, MouseButton, MouseButtonState, TrayIconEvent}};

/// Supported tray label locales (18 base locales). MUST stay in lockstep with
/// `src/locales/<lc>/common.json` keys `tray.*` and the `Locale` union in
/// `src/store/appStore.ts`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayLang { En, Zh, Ja, Es, Fr, De, Ko, Ru, Pt, Ar, It, Nl, Pl, Tr, Vi, Th, Id, Hi }

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
        TrayLang::En => TrayLabels { show: "Show Window", quit: "Quit", tooltip: "EgressAPIKEY" },
        TrayLang::Zh => TrayLabels { show: "显示窗口", quit: "退出", tooltip: "EgressAPIKEY" },
        TrayLang::Ja => TrayLabels { show: "ウィンドウを表示", quit: "終了", tooltip: "EgressAPIKEY" },
        TrayLang::Es => TrayLabels { show: "Mostrar Ventana", quit: "Salir", tooltip: "EgressAPIKEY" },
        TrayLang::Fr => TrayLabels { show: "Afficher la Fenêtre", quit: "Quitter", tooltip: "EgressAPIKEY" },
        TrayLang::De => TrayLabels { show: "Fenster anzeigen", quit: "Beenden", tooltip: "EgressAPIKEY" },
        TrayLang::Ko => TrayLabels { show: "창 표시", quit: "종료", tooltip: "EgressAPIKEY" },
        TrayLang::Ru => TrayLabels { show: "Показать окно", quit: "Выйти", tooltip: "EgressAPIKEY" },
        TrayLang::Pt => TrayLabels { show: "Mostrar Janela", quit: "Sair", tooltip: "EgressAPIKEY" },
        TrayLang::Ar => TrayLabels { show: "إظهار النافذة", quit: "إنهاء", tooltip: "EgressAPIKEY" },
        TrayLang::It => TrayLabels { show: "Mostra finestra", quit: "Esci", tooltip: "EgressAPIKEY" },
        TrayLang::Nl => TrayLabels { show: "Venster tonen", quit: "Afsluiten", tooltip: "EgressAPIKEY" },
        TrayLang::Pl => TrayLabels { show: "Pokaż okno", quit: "Zakończ", tooltip: "EgressAPIKEY" },
        TrayLang::Tr => TrayLabels { show: "Pencereyi göster", quit: "Çık", tooltip: "EgressAPIKEY" },
        TrayLang::Vi => TrayLabels { show: "Hiện cửa sổ", quit: "Thoát", tooltip: "EgressAPIKEY" },
        TrayLang::Th => TrayLabels { show: "แสดงหน้าต่าง", quit: "ออก", tooltip: "EgressAPIKEY" },
        TrayLang::Id => TrayLabels { show: "Tampilkan jendela", quit: "Keluar", tooltip: "EgressAPIKEY" },
        TrayLang::Hi => TrayLabels { show: "विंडो दिखाएं", quit: "बाहर निकलें", tooltip: "EgressAPIKEY" },
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

pub fn current_lang(app: &AppHandle) -> TrayLang {
    use tauri_plugin_store::StoreExt;
    let Ok(store) = app.store("settings.json") else { return TrayLang::En; };
    let lang = store.get("lang").and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "en".into());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every TrayLang variant must produce a non-empty `show`/`quit` label.
    /// Catches a future row left as "" or a typo'd empty literal.
    #[test]
    fn labels_all_variants_non_empty() {
        let all = [
            TrayLang::En, TrayLang::Zh, TrayLang::Ja, TrayLang::Es, TrayLang::Fr, TrayLang::De,
            TrayLang::Ko, TrayLang::Ru, TrayLang::Pt, TrayLang::Ar, TrayLang::It, TrayLang::Nl,
            TrayLang::Pl, TrayLang::Tr, TrayLang::Vi, TrayLang::Th, TrayLang::Id, TrayLang::Hi,
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
            TrayLang::En, TrayLang::Zh, TrayLang::Ja, TrayLang::Es, TrayLang::Fr, TrayLang::De,
            TrayLang::Ko, TrayLang::Ru, TrayLang::Pt, TrayLang::Ar, TrayLang::It, TrayLang::Nl,
            TrayLang::Pl, TrayLang::Tr, TrayLang::Vi, TrayLang::Th, TrayLang::Id, TrayLang::Hi,
        ];
        let shows: Vec<&str> = all.iter().map(|lc| labels(*lc).show).collect();
        let mut dedup = shows.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(dedup.len(), shows.len(),
            "show labels must be distinct per locale; duplicates found");
    }

    /// The locale-key table must cover every frontend base locale (the 18 in
    /// `src/store/appStore.ts` Locale union). Unknown/garbage -> English.
    #[test]
    fn lang_for_str_covers_all_base_locales() {
        let base = ["en", "zh", "ja", "es", "fr", "de", "ko", "ru", "pt", "ar",
                    "it", "nl", "pl", "tr", "vi", "th", "id", "hi"];
        // Each maps to the expected variant (round-trip on the canonical key).
        let expect = [TrayLang::En, TrayLang::Zh, TrayLang::Ja, TrayLang::Es, TrayLang::Fr,
            TrayLang::De, TrayLang::Ko, TrayLang::Ru, TrayLang::Pt, TrayLang::Ar, TrayLang::It,
            TrayLang::Nl, TrayLang::Pl, TrayLang::Tr, TrayLang::Vi, TrayLang::Th, TrayLang::Id,
            TrayLang::Hi];
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
