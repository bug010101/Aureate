//! `aur-app` — the Tauri shell around the domain crates.

pub mod api;
pub mod state;

use tauri::Manager;

pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state::AppState::default())
        .setup(|app| {
            let fonts = state::resolve_font_paths(app.handle());
            app.state::<state::AppState>().set_fonts(fonts);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api::open_vault,
            api::vault_info,
            api::open_file,
            api::load_prefs,
            api::save_prefs,
            api::list_entries,
            api::sync_index,
            api::read_doc,
            api::save_doc,
            api::create_doc,
            api::delete_entry,
            api::rename_entry,
            api::create_card,
            api::save_card,
            api::update_card_status,
            api::cards,
            api::open_card,
            api::backlinks,
            api::ledger,
            api::search,
            api::save_script,
            api::preview,
            api::preview_math,
            api::export_pdf,
            api::export_folder,
            api::export_cardbook,
            api::typst_source,
            api::read_image,
            api::reveal,
            api::open_external,
            api::font_report,
        ]);

    builder
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
                let _ = window.unminimize();
            }
        }))
        .run(tauri::generate_context!())
        .expect("Aureate failed to start");
}
