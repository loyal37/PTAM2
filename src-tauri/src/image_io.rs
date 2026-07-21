use std::fs::{self, File};
use std::io::{BufReader, Cursor, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ddsfile::Dds;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, RgbaImage};
use tempfile::NamedTempFile;

use crate::models::{AppError, AppResult, TextureAsset, TextureInfo};

pub fn load_texture_asset(path: &Path, id: u64) -> AppResult<TextureAsset> {
    if !path.is_file() {
        return Err(AppError::Invalid(format!("文件不存在：{}", path.display())));
    }
    let image = load_rgba(path)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("未命名贴图")
        .to_owned();
    let format = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("image")
        .to_uppercase();
    Ok(TextureAsset {
        id,
        path: path.to_path_buf(),
        name,
        format,
        image,
    })
}

pub fn load_rgba(path: &Path) -> AppResult<RgbaImage> {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("dds"))
    {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let dds = Dds::read(&mut reader).map_err(|error| AppError::Dds(error.to_string()))?;
        if dds.get_depth() > 1 || dds.get_num_array_layers() > 1 {
            return Err(AppError::Invalid(
                "暂不支持体积纹理、纹理数组或立方体 DDS。".into(),
            ));
        }
        return image_dds::image_from_dds(&dds, 0)
            .map_err(|error| AppError::Dds(error.to_string()));
    }

    image::open(path)
        .map(DynamicImage::into_rgba8)
        .map_err(AppError::from)
}

pub fn texture_info(asset: &TextureAsset) -> AppResult<TextureInfo> {
    Ok(TextureInfo {
        id: asset.id,
        name: asset.name.clone(),
        path: asset.path.to_string_lossy().into_owned(),
        width: asset.image.width(),
        height: asset.image.height(),
        format: asset.format.clone(),
        thumbnail_data_url: image_data_url(&asset.image, 112)?,
    })
}

pub fn image_data_url(image: &RgbaImage, max_edge: u32) -> AppResult<String> {
    let scaled = if image.width().max(image.height()) > max_edge {
        image::imageops::resize(
            image,
            scaled_width(image, max_edge),
            scaled_height(image, max_edge),
            FilterType::Lanczos3,
        )
    } else {
        image.clone()
    };
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(scaled).write_to(&mut bytes, ImageFormat::Png)?;
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(bytes.into_inner())
    ))
}

fn scaled_width(image: &RgbaImage, max_edge: u32) -> u32 {
    if image.width() >= image.height() {
        max_edge
    } else {
        ((u64::from(image.width()) * u64::from(max_edge)) / u64::from(image.height())).max(1) as u32
    }
}

fn scaled_height(image: &RgbaImage, max_edge: u32) -> u32 {
    if image.height() >= image.width() {
        max_edge
    } else {
        ((u64::from(image.height()) * u64::from(max_edge)) / u64::from(image.width())).max(1) as u32
    }
}

pub fn canonical_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
}

pub fn ensure_extension(path: &Path, extension: &str) -> PathBuf {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
    {
        path.to_path_buf()
    } else {
        path.with_extension(extension)
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir()?);
    fs::create_dir_all(&parent)?;
    let mut temp = NamedTempFile::new_in(&parent)?;
    temp.write_all(bytes)?;
    temp.as_file_mut().sync_all()?;
    temp.persist(path)
        .map_err(|error| AppError::Io(error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn data_url_downscales_without_changing_aspect_ratio() {
        let image = RgbaImage::from_pixel(400, 200, Rgba([1, 2, 3, 255]));
        let url = image_data_url(&image, 100).unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn ensure_extension_replaces_mismatched_suffix() {
        assert_eq!(
            PathBuf::from("atlas.dds"),
            ensure_extension(Path::new("atlas.png"), "dds")
        );
    }
}
