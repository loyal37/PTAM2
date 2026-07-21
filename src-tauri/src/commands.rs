use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use image::imageops::FilterType;
use serde::Serialize;
use tauri::State;

use crate::atlas::build_atlas;
use crate::dds_codec::{parse_quality, patch_blocker, patch_dds_slots, write_dds, write_png};
use crate::image_io::{
    atomic_write, canonical_key, ensure_extension, image_data_url, load_rgba, load_texture_asset,
    texture_info,
};
use crate::models::{
    AddTexturesResponse, AppError, AppResult, AppState, ExportReport, ExportRequest, LoadFailure,
    PreviewResponse, ProjectSnapshot, TextureInfo,
};

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn next_id(store: &mut crate::models::TextureStore) -> u64 {
    if store.next_id == 0 {
        store.next_id = 1;
    }
    let id = store.next_id;
    store.next_id += 1;
    id
}

fn snapshot(state: &AppState) -> AppResult<ProjectSnapshot> {
    let store = state.0.read().map_err(|_| AppError::State)?;
    Ok(ProjectSnapshot {
        textures: store.textures.clone(),
        base: store.base.clone(),
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStateResponse {
    textures: Vec<TextureInfo>,
    base: Option<TextureInfo>,
}

#[tauri::command]
pub async fn get_project_state(state: State<'_, AppState>) -> Result<ProjectStateResponse, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let project = snapshot(&state)?;
        let textures = project
            .textures
            .iter()
            .map(texture_info)
            .collect::<AppResult<Vec<_>>>()?;
        let base = project.base.as_ref().map(texture_info).transpose()?;
        Ok::<_, AppError>(ProjectStateResponse { textures, base })
    })
    .await
    .map_err(message)?
    .map_err(message)
}

#[tauri::command]
pub async fn add_textures(
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<AddTexturesResponse, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut store = state.0.write().map_err(|_| AppError::State)?;
        let mut existing: HashSet<String> = store
            .textures
            .iter()
            .map(|texture| canonical_key(&texture.path))
            .collect();
        let mut textures = Vec::new();
        let mut errors = Vec::new();
        let mut duplicate_count = 0;
        for raw_path in paths {
            let path = PathBuf::from(&raw_path);
            let key = canonical_key(&path);
            if existing.contains(&key) {
                duplicate_count += 1;
                continue;
            }
            let id = next_id(&mut store);
            match load_texture_asset(&path, id).and_then(|asset| {
                let info = texture_info(&asset)?;
                Ok((asset, info))
            }) {
                Ok((asset, info)) => {
                    store.textures.push(asset);
                    textures.push(info);
                    existing.insert(key);
                }
                Err(error) => errors.push(LoadFailure {
                    path: raw_path,
                    message: error.to_string(),
                }),
            }
        }
        Ok::<_, AppError>(AddTexturesResponse {
            textures,
            errors,
            duplicate_count,
        })
    })
    .await
    .map_err(message)?
    .map_err(message)
}

#[tauri::command]
pub fn remove_textures(ids: Vec<u64>, state: State<'_, AppState>) -> Result<(), String> {
    let ids: HashSet<u64> = ids.into_iter().collect();
    let mut store = state.0.write().map_err(|_| message(AppError::State))?;
    store.textures.retain(|texture| !ids.contains(&texture.id));
    Ok(())
}

#[tauri::command]
pub fn clear_textures(state: State<'_, AppState>) -> Result<(), String> {
    state
        .0
        .write()
        .map_err(|_| message(AppError::State))?
        .textures
        .clear();
    Ok(())
}

#[tauri::command]
pub async fn resize_textures(
    width: u32,
    height: u32,
    state: State<'_, AppState>,
) -> Result<Vec<TextureInfo>, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if width == 0 || height == 0 || width > 32_768 || height > 32_768 {
            return Err(AppError::Invalid("贴图宽高必须在 1 到 32768 之间。".into()));
        }
        let mut store = state.0.write().map_err(|_| AppError::State)?;
        for texture in &mut store.textures {
            if texture.image.dimensions() != (width, height) {
                texture.image =
                    image::imageops::resize(&texture.image, width, height, FilterType::Lanczos3);
            }
        }
        store
            .textures
            .iter()
            .map(texture_info)
            .collect::<AppResult<Vec<_>>>()
    })
    .await
    .map_err(message)?
    .map_err(message)
}

#[tauri::command]
pub async fn set_base_texture(
    path: String,
    state: State<'_, AppState>,
) -> Result<TextureInfo, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(path);
        let mut store = state.0.write().map_err(|_| AppError::State)?;
        let id = next_id(&mut store);
        let asset = load_texture_asset(&path, id)?;
        let info = texture_info(&asset)?;
        store.base = Some(asset);
        Ok::<_, AppError>(info)
    })
    .await
    .map_err(message)?
    .map_err(message)
}

#[tauri::command]
pub fn clear_base_texture(state: State<'_, AppState>) -> Result<(), String> {
    state.0.write().map_err(|_| message(AppError::State))?.base = None;
    Ok(())
}

#[tauri::command]
pub async fn read_background_image(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let image = load_rgba(Path::new(&path))?;
        image_data_url(&image, 2560)
    })
    .await
    .map_err(message)?
    .map_err(message)
}

#[tauri::command]
pub async fn build_preview(
    options: crate::models::BuildOptions,
    state: State<'_, AppState>,
) -> Result<PreviewResponse, String> {
    let project = snapshot(state.inner()).map_err(message)?;
    tauri::async_runtime::spawn_blocking(move || {
        let build = build_atlas(&project, &options, false)?;
        Ok::<_, AppError>(PreviewResponse {
            width: build.image.width(),
            height: build.image.height(),
            preview_data_url: image_data_url(&build.image, 2048)?,
            placements: build.placements,
            grid: build.grid,
        })
    })
    .await
    .map_err(message)?
    .map_err(message)
}

#[tauri::command]
pub async fn export_atlas(
    request: ExportRequest,
    state: State<'_, AppState>,
) -> Result<ExportReport, String> {
    let project = snapshot(state.inner()).map_err(message)?;
    tauri::async_runtime::spawn_blocking(move || export_project(project, request))
        .await
        .map_err(message)?
        .map_err(message)
}

fn export_project(project: ProjectSnapshot, request: ExportRequest) -> AppResult<ExportReport> {
    let started = Instant::now();
    let quality = parse_quality(&request.quality)?;
    let build = build_atlas(&project, &request.options, true)?;
    let is_png = request.format == "png";
    let output = ensure_extension(
        Path::new(&request.output_path),
        if is_png { "png" } else { "dds" },
    );
    let mut mode = "full-encode".to_owned();
    let mut actual_format = if is_png {
        "PNG".to_owned()
    } else {
        request.format.clone()
    };
    let mut preserved_outside_slots = false;
    let mut diagnostics = Vec::new();

    if !is_png {
        if let (Some(base), Some(grid)) = (&project.base, build.grid) {
            if let Some(reason) =
                patch_blocker(base, grid, build.image.width(), build.image.height())
            {
                mode = format!("full-encode: {reason}");
                actual_format = write_dds(&output, &build.image, &request.format, quality)?;
            } else {
                let outcome = patch_dds_slots(
                    base,
                    &output,
                    &project.textures,
                    &request.options.assignments,
                    grid,
                    quality,
                )?;
                mode = "binary-patch".into();
                actual_format = outcome.format;
                preserved_outside_slots = outcome.preserved_outside_slots;
                diagnostics = outcome.diagnostics;
            }
        } else {
            mode = if project.base.is_some() {
                "full-encode: 无可用槽位网格".into()
            } else {
                "full-encode: 无 DDS 底图".into()
            };
            actual_format = write_dds(&output, &build.image, &request.format, quality)?;
        }
    } else {
        write_png(&output, &build.image)?;
    }

    let json_path = if request.export_json {
        let path = output.with_extension("json");
        let payload = serde_json::json!({
            "width": build.image.width(),
            "height": build.image.height(),
            "items": build.placements,
        });
        let bytes = serde_json::to_vec_pretty(&payload)
            .map_err(|error| AppError::Invalid(error.to_string()))?;
        atomic_write(&path, &bytes)?;
        Some(path.to_string_lossy().into_owned())
    } else {
        None
    };

    Ok(ExportReport {
        output_path: output.to_string_lossy().into_owned(),
        json_path,
        mode,
        format: actual_format,
        width: build.image.width(),
        height: build.image.height(),
        elapsed_ms: started.elapsed().as_millis(),
        preserved_outside_slots,
        diagnostics,
    })
}
