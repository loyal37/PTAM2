mod atlas;
mod commands;
mod dds_codec;
mod image_io;
mod models;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .manage(models::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_project_state,
            commands::add_textures,
            commands::remove_textures,
            commands::clear_textures,
            commands::resize_textures,
            commands::set_base_texture,
            commands::clear_base_texture,
            commands::read_background_image,
            commands::build_preview,
            commands::export_atlas,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run PTAM2");
}
