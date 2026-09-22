use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{Manager, State};
use tempfile::TempDir;

use crate::commands::export_project;
use crate::models::{AppState, BuildOptions, ExportRequest, ProjectSnapshot};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DragExportRequest {
    format: String,
    quality: String,
    export_json: bool,
    options: BuildOptions,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedExportInfo {
    token: u64,
    name: String,
}

struct PreparedExport {
    token: u64,
    revision: u64,
    key: String,
    files: Vec<PathBuf>,
    directory: Mutex<Option<TempDir>>,
}

impl PreparedExport {
    fn info(&self) -> PreparedExportInfo {
        PreparedExportInfo {
            token: self.token,
            name: self.files[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        }
    }

    fn retain_dropped_files(&self) {
        // Other applications can keep references to dropped files. Never remove
        // accepted exports when the drag ends or when PTAM2 exits.
        if let Ok(mut directory) = self.directory.lock()
            && let Some(directory) = directory.take()
        {
            let _ = directory.keep();
        }
    }
}

#[derive(Default)]
struct Cache {
    next_token: u64,
    prepared: Option<Arc<PreparedExport>>,
}

#[derive(Clone, Default)]
pub struct DragExportState(Arc<Mutex<Cache>>);

fn prepare_files(
    root: &std::path::Path,
    project: ProjectSnapshot,
    request: DragExportRequest,
    token: u64,
    revision: u64,
    key: String,
) -> Result<PreparedExport, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let directory = tempfile::Builder::new()
        .prefix("atlas-")
        .tempdir_in(root)
        .map_err(|e| e.to_string())?;
    let extension = if request.format == "png" {
        "png"
    } else {
        "dds"
    };
    let output = directory.path().join(format!("atlas.{extension}"));
    let report = export_project(
        project,
        ExportRequest {
            output_path: output.to_string_lossy().into_owned(),
            format: request.format,
            quality: request.quality,
            export_json: request.export_json,
            options: request.options,
        },
    )
    .map_err(|e| e.to_string())?;
    let mut files = vec![PathBuf::from(report.output_path)];
    if let Some(path) = report.json_path {
        files.push(path.into());
    }
    Ok(PreparedExport {
        token,
        revision,
        key,
        files,
        directory: Mutex::new(Some(directory)),
    })
}

#[tauri::command]
pub async fn prepare_drag_export(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    cache: State<'_, DragExportState>,
    request: DragExportRequest,
) -> Result<PreparedExportInfo, String> {
    let state = state.inner().clone();
    let cache = cache.inner().clone();
    let root = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("drag-exports");
    tauri::async_runtime::spawn_blocking(move || {
        // Serialize export preparation only; never hold the project lock while encoding.
        let mut cache = cache.0.lock().map_err(|_| "导出缓存不可用")?;
        let (project, revision) = {
            let store = state.0.read().map_err(|_| "项目不可用")?;
            (
                ProjectSnapshot {
                    textures: store.textures.clone(),
                    base: store.base.clone(),
                },
                store.revision,
            )
        };
        let key = serde_json::to_string(&request).map_err(|e| e.to_string())?;
        if let Some(prepared) = &cache.prepared
            && prepared.key == key
            && prepared.revision == revision
            && prepared.files.iter().all(|file| file.is_file())
        {
            return Ok(prepared.info());
        }
        cache.next_token += 1;
        let prepared = prepare_files(&root, project, request, cache.next_token, revision, key)?;
        if state.0.read().map_err(|_| "项目不可用")?.revision != revision {
            return Err("贴图已变化，请重新拖出".into());
        }
        let info = prepared.info();
        cache.prepared = Some(Arc::new(prepared));
        Ok(info)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(windows)]
fn primary_button_down() -> bool {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetAsyncKeyState(key: i32) -> i16;
    }
    // A slow encode must not start a drag after the user has released the mouse.
    unsafe { GetAsyncKeyState(1) < 0 }
}

#[tauri::command]
pub async fn start_drag_export(
    app: tauri::AppHandle,
    window: tauri::Window,
    state: State<'_, AppState>,
    cache: State<'_, DragExportState>,
    token: u64,
) -> Result<bool, String> {
    let prepared = {
        let cache = cache.0.lock().map_err(|_| "导出缓存不可用")?;
        cache
            .prepared
            .as_ref()
            .filter(|p| p.token == token)
            .cloned()
            .ok_or("导出文件已过期，请重新拖出")?
    };
    if state.0.read().map_err(|_| "项目不可用")?.revision != prepared.revision {
        return Err("贴图已变化，请重新拖出".into());
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        #[cfg(windows)]
        if !primary_button_down() {
            let _ = sender.send(Ok(false));
            return;
        }
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let result_flag = dropped.clone();
        let retained = prepared.clone();
        #[cfg(target_os = "linux")]
        let handle = match window.gtk_window() {
            Ok(handle) => handle,
            Err(error) => {
                let _ = sender.send(Err(error.to_string()));
                return;
            }
        };
        #[cfg(not(target_os = "linux"))]
        let handle = window;
        let result = drag::start_drag(
            &handle,
            drag::DragItem::Files(prepared.files.clone()),
            drag::Image::Raw(include_bytes!("../icons/64x64.png").to_vec()),
            move |result, _| {
                if matches!(result, drag::DragResult::Dropped) {
                    retained.retain_dropped_files();
                    result_flag.store(true, std::sync::atomic::Ordering::Release);
                }
            },
            drag::Options {
                mode: drag::DragMode::Copy,
                ..Default::default()
            },
        );
        let _ = sender.send(
            result
                .map(|_| dropped.load(std::sync::atomic::Ordering::Acquire))
                .map_err(|e| e.to_string()),
        );
    })
    .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || receiver.recv().map_err(|e| e.to_string())?)
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::TextureAsset;

    fn fixture() -> (ProjectSnapshot, DragExportRequest) {
        let project = ProjectSnapshot {
            textures: vec![TextureAsset {
                id: 1,
                path: "fixture.png".into(),
                name: "fixture.png".into(),
                format: "PNG".into(),
                image: Arc::new(image::RgbaImage::from_pixel(
                    4096,
                    4,
                    image::Rgba([10, 20, 30, 255]),
                )),
            }],
            base: None,
        };
        let request = DragExportRequest {
            format: "png".into(),
            quality: "fast".into(),
            export_json: true,
            options: BuildOptions {
                layout_mode: "auto".into(),
                padding: 0,
                columns: 2,
                canvas_width: None,
                canvas_height: None,
                assignments: vec![],
            },
        };
        (project, request)
    }

    #[test]
    fn staged_export_is_full_resolution_with_matching_json() {
        let root = tempfile::tempdir().unwrap();
        let (project, request) = fixture();
        let prepared = prepare_files(root.path(), project, request, 7, 3, "key".into()).unwrap();
        assert_eq!(prepared.info().token, 7);
        assert_eq!(prepared.info().name, "atlas.png");
        assert_eq!(prepared.files.len(), 2);
        let output = image::open(&prepared.files[0]).unwrap().into_rgba8();
        assert_eq!(output.dimensions(), (4096, 4));
        assert_eq!(output.get_pixel(3000, 1).0, [10, 20, 30, 255]);
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&prepared.files[1]).unwrap()).unwrap();
        assert_eq!(json["width"], 4096);
        assert_eq!(json["height"], 4);
        let staged_files = prepared.files.clone();
        drop(prepared);
        assert!(staged_files.iter().all(|file| !file.exists()));
    }

    #[test]
    fn accepted_files_survive_cache_replacement_and_shutdown() {
        let root = tempfile::tempdir().unwrap();
        let (project, request) = fixture();
        let prepared = prepare_files(root.path(), project, request, 1, 0, "key".into()).unwrap();
        let files = prepared.files.clone();
        prepared.retain_dropped_files();
        prepared.retain_dropped_files();
        drop(prepared);
        assert!(files.iter().all(|file| file.is_file()));
    }

    #[test]
    fn staged_dds_honors_format_and_optional_json() {
        let root = tempfile::tempdir().unwrap();
        let (project, mut request) = fixture();
        request.format = "bc7-srgb".into();
        request.export_json = false;
        let prepared = prepare_files(root.path(), project, request, 1, 0, "key".into()).unwrap();
        assert_eq!(prepared.info().name, "atlas.dds");
        assert_eq!(prepared.files.len(), 1);
        let output = ddsfile::Dds::read(&mut std::io::BufReader::new(
            std::fs::File::open(&prepared.files[0]).unwrap(),
        ))
        .unwrap();
        assert_eq!(
            image_dds::dds_image_format(&output).unwrap(),
            image_dds::ImageFormat::BC7RgbaUnormSrgb
        );
    }
}
