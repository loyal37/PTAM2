use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::Path;

use ddsfile::{D3DFormat, Dds, NewD3dParams};
use image::{DynamicImage, ImageFormat as FileImageFormat, RgbaImage};
use image_dds::{ImageFormat, Mipmaps, Quality, SurfaceRgba8};
use rayon::prelude::*;

use crate::image_io::atomic_write;
use crate::models::{
    AppError, AppResult, SlotAssignment, SlotDiagnostic, SlotGridInfo, TextureAsset,
};

struct PreparedSlot {
    slot: u32,
    texture: String,
    bytes: Vec<u8>,
    method: &'static str,
}

pub struct PatchOutcome {
    pub diagnostics: Vec<SlotDiagnostic>,
    pub preserved_outside_slots: bool,
    pub format: String,
}

pub fn parse_quality(value: &str) -> AppResult<Quality> {
    match value {
        "fast" => Ok(Quality::Fast),
        "normal" => Ok(Quality::Normal),
        "slow" => Ok(Quality::Slow),
        other => Err(AppError::Invalid(format!("未知 DDS 压缩质量：{other}"))),
    }
}

pub fn patch_blocker(
    base: &TextureAsset,
    grid: SlotGridInfo,
    canvas_width: u32,
    canvas_height: u32,
) -> Option<String> {
    if !base
        .path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("dds"))
    {
        return Some("底图不是 DDS".into());
    }
    if (canvas_width, canvas_height) != base.image.dimensions() {
        return Some("最终画布尺寸与 DDS 底图不同".into());
    }
    let dds = match read_dds(&base.path) {
        Ok(dds) => dds,
        Err(error) => return Some(format!("无法解析底图 DDS：{error}")),
    };
    if dds.get_num_mipmap_levels() != 1 {
        return Some(format!(
            "底图包含 {} 级 mipmap",
            dds.get_num_mipmap_levels()
        ));
    }
    if dds.get_depth() > 1 || dds.get_num_array_layers() > 1 {
        return Some("底图不是普通二维单层 DDS".into());
    }
    if !grid.slot_width.is_multiple_of(4)
        || !grid.slot_height.is_multiple_of(4)
        || !dds.get_width().is_multiple_of(4)
        || !dds.get_height().is_multiple_of(4)
    {
        return Some("底图或槽位尺寸不是 4 像素整数倍".into());
    }
    let format = match image_dds::dds_image_format(&dds) {
        Ok(format) => format,
        Err(info) => return Some(format!("不支持的底图格式：{info:?}")),
    };
    if block_size(format).is_none() {
        return Some("底图压缩格式不支持槽位补丁".into());
    }
    None
}

pub fn patch_dds_slots(
    base: &TextureAsset,
    target: &Path,
    textures: &[TextureAsset],
    assignments: &[SlotAssignment],
    grid: SlotGridInfo,
    quality: Quality,
) -> AppResult<PatchOutcome> {
    if assignments.len() != textures.len() {
        return Err(AppError::Invalid("DDS 补丁要求全部贴图已分配槽位。".into()));
    }
    let (base_dds, mut raw, header_size) = read_dds_raw(&base.path)?;
    let format = image_dds::dds_image_format(&base_dds)
        .map_err(|info| AppError::Dds(format!("不支持的底图格式：{info:?}")))?;
    let block_size =
        block_size(format).ok_or_else(|| AppError::Dds("底图不是受支持的 BC 压缩格式。".into()))?;
    let texture_by_id: HashMap<u64, &TextureAsset> = textures
        .iter()
        .map(|texture| (texture.id, texture))
        .collect();

    let prepared: AppResult<Vec<PreparedSlot>> = assignments
        .par_iter()
        .map(|assignment| {
            let texture = texture_by_id.get(&assignment.texture_id).ok_or_else(|| {
                AppError::Invalid(format!("找不到贴图 ID {}。", assignment.texture_id))
            })?;
            if assignment.slot == 0 || assignment.slot > grid.total_slots() {
                return Err(AppError::Invalid(format!(
                    "槽位必须在 1 到 {} 之间。",
                    grid.total_slots()
                )));
            }
            if let Some(bytes) = matching_dds_level(texture, format, grid)? {
                Ok(PreparedSlot {
                    slot: assignment.slot,
                    texture: texture.name.clone(),
                    bytes,
                    method: "direct-copy",
                })
            } else {
                let surface = SurfaceRgba8::from_image(&texture.image)
                    .encode(format, quality, Mipmaps::Disabled)
                    .map_err(|error| AppError::Dds(error.to_string()))?;
                Ok(PreparedSlot {
                    slot: assignment.slot,
                    texture: texture.name.clone(),
                    bytes: surface.data,
                    method: "native-reencode",
                })
            }
        })
        .collect();
    let prepared = prepared?;

    let original = raw.clone();
    let blocks_per_row = base_dds.get_width().div_ceil(4) as usize;
    let slot_blocks_per_row = grid.slot_width.div_ceil(4) as usize;
    let slot_block_rows = grid.slot_height.div_ceil(4) as usize;
    let row_len = slot_blocks_per_row * block_size;
    let mut modified_ranges = Vec::with_capacity(prepared.len() * slot_block_rows);

    for slot in &prepared {
        let required = slot_block_rows * row_len;
        if slot.bytes.len() < required {
            return Err(AppError::Dds(format!(
                "{} 的压缩数据不完整。",
                slot.texture
            )));
        }
        let zero_based = slot.slot - 1;
        let pixel_x = (zero_based % grid.columns) * grid.slot_width;
        let pixel_y = (zero_based / grid.columns) * grid.slot_height;
        for block_row in 0..slot_block_rows {
            let source_start = block_row * row_len;
            let target_start = header_size
                + (((pixel_y / 4) as usize + block_row) * blocks_per_row + (pixel_x / 4) as usize)
                    * block_size;
            let target_end = target_start + row_len;
            if target_end > raw.len() {
                return Err(AppError::Dds("底图压缩数据长度不足。".into()));
            }
            raw[target_start..target_end]
                .copy_from_slice(&slot.bytes[source_start..source_start + row_len]);
            modified_ranges.push(target_start..target_end);
        }
    }

    let preserved_outside_slots =
        original
            .iter()
            .zip(&raw)
            .enumerate()
            .all(|(index, (before, after))| {
                before == after || modified_ranges.iter().any(|range| range.contains(&index))
            });
    if !preserved_outside_slots {
        return Err(AppError::Dds("内部校验发现槽位外字节发生变化。".into()));
    }
    atomic_write(target, &raw)?;

    Ok(PatchOutcome {
        diagnostics: prepared
            .into_iter()
            .map(|slot| SlotDiagnostic {
                slot: slot.slot,
                texture: slot.texture,
                method: slot.method.into(),
            })
            .collect(),
        preserved_outside_slots,
        format: format_name(format).into(),
    })
}

fn matching_dds_level(
    texture: &TextureAsset,
    format: ImageFormat,
    grid: SlotGridInfo,
) -> AppResult<Option<Vec<u8>>> {
    if !texture
        .path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("dds"))
    {
        return Ok(None);
    }
    let dds = match read_dds(&texture.path) {
        Ok(dds) => dds,
        Err(_) => return Ok(None),
    };
    let source_format = match image_dds::dds_image_format(&dds) {
        Ok(format) => format,
        Err(_) => return Ok(None),
    };
    if source_format != format
        || dds.get_width() != grid.slot_width
        || dds.get_height() != grid.slot_height
        || dds.get_depth() > 1
        || dds.get_num_array_layers() > 1
    {
        return Ok(None);
    }
    let expected = grid.slot_width.div_ceil(4) as usize
        * grid.slot_height.div_ceil(4) as usize
        * block_size(format).unwrap_or(0);
    if dds.data.len() < expected {
        return Ok(None);
    }
    Ok(Some(dds.data[..expected].to_vec()))
}

pub fn write_png(path: &Path, image: &RgbaImage) -> AppResult<()> {
    let mut cursor = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image.clone()).write_to(&mut cursor, FileImageFormat::Png)?;
    atomic_write(path, &cursor.into_inner())
}

pub fn write_dds(
    path: &Path,
    image: &RgbaImage,
    output_format: &str,
    quality: Quality,
) -> AppResult<String> {
    let (format, legacy_dxt5) = match output_format {
        "dxt5" => (ImageFormat::BC3RgbaUnorm, true),
        "bc7-linear" => (ImageFormat::BC7RgbaUnorm, false),
        "bc7-srgb" => (ImageFormat::BC7RgbaUnormSrgb, false),
        other => return Err(AppError::Invalid(format!("未知 DDS 导出格式：{other}"))),
    };
    let surface = SurfaceRgba8::from_image(image)
        .encode(format, quality, Mipmaps::Disabled)
        .map_err(|error| AppError::Dds(error.to_string()))?;
    let dds = if legacy_dxt5 {
        let mut dds = Dds::new_d3d(NewD3dParams {
            height: image.height(),
            width: image.width(),
            depth: None,
            format: D3DFormat::DXT5,
            mipmap_levels: None,
            caps2: None,
        })
        .map_err(|error| AppError::Dds(error.to_string()))?;
        dds.data = surface.data;
        dds
    } else {
        surface
            .to_dds()
            .map_err(|error| AppError::Dds(error.to_string()))?
    };
    let mut bytes = Vec::new();
    dds.write(&mut bytes)
        .map_err(|error| AppError::Dds(error.to_string()))?;
    atomic_write(path, &bytes)?;
    Ok(format_name(format).into())
}

fn read_dds(path: &Path) -> AppResult<Dds> {
    let file = File::open(path)?;
    Dds::read(&mut BufReader::new(file)).map_err(|error| AppError::Dds(error.to_string()))
}

fn read_dds_raw(path: &Path) -> AppResult<(Dds, Vec<u8>, usize)> {
    let raw = std::fs::read(path)?;
    let dds =
        Dds::read(&mut Cursor::new(&raw)).map_err(|error| AppError::Dds(error.to_string()))?;
    let header_size = if raw.get(84..88) == Some(b"DX10") {
        148
    } else {
        128
    };
    if raw.len() < header_size + dds.data.len() {
        return Err(AppError::Dds("DDS 文件长度与头信息不一致。".into()));
    }
    Ok((dds, raw, header_size))
}

fn block_size(format: ImageFormat) -> Option<usize> {
    match format {
        ImageFormat::BC1RgbaUnorm | ImageFormat::BC1RgbaUnormSrgb => Some(8),
        ImageFormat::BC2RgbaUnorm
        | ImageFormat::BC2RgbaUnormSrgb
        | ImageFormat::BC3RgbaUnorm
        | ImageFormat::BC3RgbaUnormSrgb
        | ImageFormat::BC7RgbaUnorm
        | ImageFormat::BC7RgbaUnormSrgb => Some(16),
        _ => None,
    }
}

fn format_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::BC1RgbaUnorm => "BC1_UNORM",
        ImageFormat::BC1RgbaUnormSrgb => "BC1_UNORM_SRGB",
        ImageFormat::BC2RgbaUnorm => "BC2_UNORM",
        ImageFormat::BC2RgbaUnormSrgb => "BC2_UNORM_SRGB",
        ImageFormat::BC3RgbaUnorm => "BC3_UNORM / DXT5",
        ImageFormat::BC3RgbaUnormSrgb => "BC3_UNORM_SRGB",
        ImageFormat::BC7RgbaUnorm => "BC7_UNORM",
        ImageFormat::BC7RgbaUnormSrgb => "BC7_UNORM_SRGB",
        _ => "DDS",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use image::{Rgba, RgbaImage};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn binary_patch_preserves_header_and_untouched_slot_bytes() {
        let directory = tempdir().unwrap();
        let base_path = directory.path().join("base.dds");
        let source_path = directory.path().join("source.dds");
        let output_path = directory.path().join("output.dds");
        let base_image = RgbaImage::from_pixel(8, 4, Rgba([220, 20, 20, 255]));
        let source_image = RgbaImage::from_pixel(4, 4, Rgba([20, 220, 20, 255]));
        write_dds(&base_path, &base_image, "dxt5", Quality::Fast).unwrap();
        write_dds(&source_path, &source_image, "dxt5", Quality::Fast).unwrap();

        let base = TextureAsset {
            id: 99,
            path: base_path.clone(),
            name: "base.dds".into(),
            format: "DDS".into(),
            image: base_image,
        };
        let source = TextureAsset {
            id: 1,
            path: source_path,
            name: "source.dds".into(),
            format: "DDS".into(),
            image: source_image,
        };
        let grid = SlotGridInfo {
            slot_width: 4,
            slot_height: 4,
            columns: 2,
            rows: 1,
        };
        let outcome = patch_dds_slots(
            &base,
            &output_path,
            &[source],
            &[SlotAssignment {
                texture_id: 1,
                slot: 2,
            }],
            grid,
            Quality::Fast,
        )
        .unwrap();
        assert!(outcome.preserved_outside_slots);
        assert_eq!(outcome.diagnostics[0].method, "direct-copy");

        let before = std::fs::read(base_path).unwrap();
        let after = std::fs::read(output_path).unwrap();
        assert_eq!(&before[..128], &after[..128]);
        assert_eq!(&before[128..144], &after[128..144]);
        assert_ne!(&before[144..160], &after[144..160]);
    }

    #[test]
    fn png_write_is_atomic_and_readable() {
        let directory = tempdir().unwrap();
        let path = PathBuf::from(directory.path()).join("atlas.png");
        let image = RgbaImage::from_pixel(2, 2, Rgba([1, 2, 3, 4]));
        write_png(&path, &image).unwrap();
        let replacement = RgbaImage::from_pixel(3, 1, Rgba([4, 3, 2, 1]));
        write_png(&path, &replacement).unwrap();
        assert_eq!((3, 1), image::open(path).unwrap().to_rgba8().dimensions());
    }
}
