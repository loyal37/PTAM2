use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_CANVAS_PIXELS: u64 = 134_217_728;
pub const MAX_CANVAS_EDGE: u32 = 32_768;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Invalid(String),
    #[error("文件操作失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("图像处理失败：{0}")]
    Image(#[from] image::ImageError),
    #[error("DDS 处理失败：{0}")]
    Dds(String),
    #[error("内部状态暂时不可用")]
    State,
}

pub type AppResult<T> = Result<T, AppError>;

#[derive(Clone)]
pub struct TextureAsset {
    pub id: u64,
    pub path: PathBuf,
    pub name: String,
    pub format: String,
    pub image: RgbaImage,
}

#[derive(Clone, Default)]
pub struct ProjectSnapshot {
    pub textures: Vec<TextureAsset>,
    pub base: Option<TextureAsset>,
}

#[derive(Default)]
pub struct TextureStore {
    pub next_id: u64,
    pub textures: Vec<TextureAsset>,
    pub base: Option<TextureAsset>,
}

#[derive(Clone, Default)]
pub struct AppState(pub Arc<RwLock<TextureStore>>);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureInfo {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub thumbnail_data_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadFailure {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTexturesResponse {
    pub textures: Vec<TextureInfo>,
    pub errors: Vec<LoadFailure>,
    pub duplicate_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotAssignment {
    pub texture_id: u64,
    pub slot: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildOptions {
    pub layout_mode: String,
    pub padding: u32,
    pub columns: u32,
    pub canvas_width: Option<u32>,
    pub canvas_height: Option<u32>,
    #[serde(default)]
    pub assignments: Vec<SlotAssignment>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotGridInfo {
    pub slot_width: u32,
    pub slot_height: u32,
    pub columns: u32,
    pub rows: u32,
}

impl SlotGridInfo {
    pub fn total_slots(self) -> u32 {
        self.columns.saturating_mul(self.rows)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AtlasPlacement {
    pub texture_id: u64,
    pub name: String,
    pub path: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub slot: Option<u32>,
}

pub struct AtlasBuild {
    pub image: RgbaImage,
    pub placements: Vec<AtlasPlacement>,
    pub grid: Option<SlotGridInfo>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResponse {
    pub width: u32,
    pub height: u32,
    pub preview_data_url: String,
    pub placements: Vec<AtlasPlacement>,
    pub grid: Option<SlotGridInfo>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub output_path: String,
    pub format: String,
    pub quality: String,
    pub export_json: bool,
    pub options: BuildOptions,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotDiagnostic {
    pub slot: u32,
    pub texture: String,
    pub method: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub output_path: String,
    pub json_path: Option<String>,
    pub mode: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub elapsed_ms: u128,
    pub preserved_outside_slots: bool,
    pub diagnostics: Vec<SlotDiagnostic>,
}

pub fn validate_canvas(width: u32, height: u32) -> AppResult<()> {
    if width == 0 || height == 0 {
        return Err(AppError::Invalid("画布宽高必须大于 0。".into()));
    }
    if width > MAX_CANVAS_EDGE || height > MAX_CANVAS_EDGE {
        return Err(AppError::Invalid(format!(
            "画布边长不能超过 {MAX_CANVAS_EDGE} 像素。"
        )));
    }
    if u64::from(width) * u64::from(height) > MAX_CANVAS_PIXELS {
        return Err(AppError::Invalid(
            "画布像素总量过大，已阻止本次操作以避免内存耗尽。".into(),
        ));
    }
    Ok(())
}
