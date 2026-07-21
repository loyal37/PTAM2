use std::collections::{HashMap, HashSet};

use image::{Rgba, RgbaImage, imageops};

use crate::models::{
    AppError, AppResult, AtlasBuild, AtlasPlacement, BuildOptions, ProjectSnapshot, SlotGridInfo,
    TextureAsset, validate_canvas,
};

pub fn build_atlas(
    project: &ProjectSnapshot,
    options: &BuildOptions,
    require_all_assignments: bool,
) -> AppResult<AtlasBuild> {
    match &project.base {
        Some(base) => build_on_base(project, base, options, require_all_assignments),
        None => build_regular(&project.textures, options),
    }
}

pub fn get_slot_grid(base: &TextureAsset, textures: &[TextureAsset]) -> AppResult<SlotGridInfo> {
    let first = textures
        .first()
        .ok_or_else(|| AppError::Invalid("底图模式下，请先添加至少一张待合并贴图。".into()))?;
    let (slot_width, slot_height) = first.image.dimensions();
    if textures
        .iter()
        .any(|texture| texture.image.dimensions() != (slot_width, slot_height))
    {
        return Err(AppError::Invalid(
            "底图槽位模式要求所有待合并贴图尺寸一致。".into(),
        ));
    }
    if !base.image.width().is_multiple_of(slot_width)
        || !base.image.height().is_multiple_of(slot_height)
    {
        return Err(AppError::Invalid(format!(
            "底图尺寸 {} x {} 无法按贴图尺寸 {} x {} 整齐划分槽位。",
            base.image.width(),
            base.image.height(),
            slot_width,
            slot_height
        )));
    }
    Ok(SlotGridInfo {
        slot_width,
        slot_height,
        columns: base.image.width() / slot_width,
        rows: base.image.height() / slot_height,
    })
}

fn build_regular(textures: &[TextureAsset], options: &BuildOptions) -> AppResult<AtlasBuild> {
    if textures.is_empty() {
        return Err(AppError::Invalid("请先添加至少一张贴图。".into()));
    }
    let cell_width = textures
        .iter()
        .map(|texture| texture.image.width())
        .max()
        .unwrap_or(1);
    let cell_height = textures
        .iter()
        .map(|texture| texture.image.height())
        .max()
        .unwrap_or(1);
    let (columns, rows) = calculate_grid(
        u32::try_from(textures.len()).map_err(|_| AppError::Invalid("贴图数量过多。".into()))?,
        &options.layout_mode,
        options.columns,
    )?;
    let required_width = checked_extent(columns, cell_width, options.padding)?;
    let required_height = checked_extent(rows, cell_height, options.padding)?;
    let canvas_width = options.canvas_width.unwrap_or(required_width);
    let canvas_height = options.canvas_height.unwrap_or(required_height);
    if canvas_width < required_width || canvas_height < required_height {
        return Err(AppError::Invalid(format!(
            "当前布局至少需要 {required_width} x {required_height}，设置的画布为 {canvas_width} x {canvas_height}。"
        )));
    }
    validate_canvas(canvas_width, canvas_height)?;

    let mut atlas = RgbaImage::from_pixel(canvas_width, canvas_height, Rgba([0, 0, 0, 0]));
    let mut placements = Vec::with_capacity(textures.len());
    for (index, texture) in textures.iter().enumerate() {
        let index = u32::try_from(index).map_err(|_| AppError::Invalid("贴图数量过多。".into()))?;
        let column = index % columns;
        let row = index / columns;
        let x = column * (cell_width + options.padding);
        let y = row * (cell_height + options.padding);
        imageops::overlay(&mut atlas, &texture.image, i64::from(x), i64::from(y));
        placements.push(placement(texture, x, y, None));
    }

    Ok(AtlasBuild {
        image: atlas,
        placements,
        grid: None,
    })
}

fn build_on_base(
    project: &ProjectSnapshot,
    base: &TextureAsset,
    options: &BuildOptions,
    require_all_assignments: bool,
) -> AppResult<AtlasBuild> {
    let target_width = options.canvas_width.unwrap_or(base.image.width());
    let target_height = options.canvas_height.unwrap_or(base.image.height());
    if target_width < base.image.width() || target_height < base.image.height() {
        return Err(AppError::Invalid(format!(
            "底图为 {} x {}，最终画布不能更小。",
            base.image.width(),
            base.image.height()
        )));
    }
    validate_canvas(target_width, target_height)?;
    let mut atlas = RgbaImage::from_pixel(target_width, target_height, Rgba([0, 0, 0, 0]));
    imageops::replace(&mut atlas, &base.image, 0, 0);

    if project.textures.is_empty() {
        return Ok(AtlasBuild {
            image: atlas,
            placements: Vec::new(),
            grid: None,
        });
    }

    let grid = get_slot_grid(base, &project.textures)?;
    let texture_by_id: HashMap<u64, &TextureAsset> = project
        .textures
        .iter()
        .map(|texture| (texture.id, texture))
        .collect();
    let mut assigned_textures = HashSet::new();
    let mut assigned_slots = HashSet::new();
    let mut placements = Vec::with_capacity(options.assignments.len());

    for assignment in &options.assignments {
        let texture = texture_by_id.get(&assignment.texture_id).ok_or_else(|| {
            AppError::Invalid(format!(
                "槽位引用了不存在的贴图 ID {}。",
                assignment.texture_id
            ))
        })?;
        if !assigned_textures.insert(assignment.texture_id) {
            return Err(AppError::Invalid(format!(
                "贴图 {} 被分配了多个槽位。",
                texture.name
            )));
        }
        if assignment.slot == 0 || assignment.slot > grid.total_slots() {
            return Err(AppError::Invalid(format!(
                "槽位编号必须在 1 到 {} 之间。",
                grid.total_slots()
            )));
        }
        if !assigned_slots.insert(assignment.slot) {
            return Err(AppError::Invalid(format!(
                "槽位 {} 被重复使用。",
                assignment.slot
            )));
        }
        let zero_based = assignment.slot - 1;
        let x = (zero_based % grid.columns) * grid.slot_width;
        let y = (zero_based / grid.columns) * grid.slot_height;
        imageops::replace(&mut atlas, &texture.image, i64::from(x), i64::from(y));
        placements.push(placement(texture, x, y, Some(assignment.slot)));
    }

    if require_all_assignments && assigned_textures.len() != project.textures.len() {
        return Err(AppError::Invalid(format!(
            "底图模式下需要放置全部贴图；当前已放置 {}/{} 张。",
            assigned_textures.len(),
            project.textures.len()
        )));
    }

    Ok(AtlasBuild {
        image: atlas,
        placements,
        grid: Some(grid),
    })
}

fn placement(texture: &TextureAsset, x: u32, y: u32, slot: Option<u32>) -> AtlasPlacement {
    AtlasPlacement {
        texture_id: texture.id,
        name: texture.name.clone(),
        path: texture.path.to_string_lossy().into_owned(),
        x,
        y,
        width: texture.image.width(),
        height: texture.image.height(),
        slot,
    }
}

fn checked_extent(count: u32, cell: u32, padding: u32) -> AppResult<u32> {
    count
        .checked_mul(cell)
        .and_then(|value| value.checked_add(count.saturating_sub(1).saturating_mul(padding)))
        .ok_or_else(|| AppError::Invalid("布局尺寸溢出。".into()))
}

pub fn calculate_grid(count: u32, mode: &str, fixed_columns: u32) -> AppResult<(u32, u32)> {
    if count == 0 {
        return Err(AppError::Invalid("贴图数量必须大于 0。".into()));
    }
    let result = match mode {
        "horizontal" => (count, 1),
        "vertical" => (1, count),
        "grid" => {
            let columns = fixed_columns.max(1);
            (columns, count.div_ceil(columns))
        }
        "auto" => {
            let columns = (f64::from(count).sqrt().ceil() as u32).max(1);
            (columns, count.div_ceil(columns))
        }
        other => {
            return Err(AppError::Invalid(format!("未知排列方式：{other}")));
        }
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use image::{Rgba, RgbaImage};
    use pretty_assertions::assert_eq;

    use super::*;

    fn texture(id: u64, width: u32, height: u32, color: [u8; 4]) -> TextureAsset {
        TextureAsset {
            id,
            path: PathBuf::from(format!("texture-{id}.png")),
            name: format!("texture-{id}.png"),
            format: "PNG".into(),
            image: RgbaImage::from_pixel(width, height, Rgba(color)),
        }
    }

    fn options() -> BuildOptions {
        BuildOptions {
            layout_mode: "auto".into(),
            padding: 0,
            columns: 2,
            canvas_width: None,
            canvas_height: None,
            assignments: Vec::new(),
        }
    }

    #[test]
    fn automatic_grid_is_balanced() {
        assert_eq!((3, 2), calculate_grid(5, "auto", 2).unwrap());
        assert_eq!((2, 2), calculate_grid(4, "auto", 2).unwrap());
    }

    #[test]
    fn regular_layout_keeps_padding_transparent() {
        let project = ProjectSnapshot {
            textures: vec![
                texture(1, 4, 4, [255, 0, 0, 255]),
                texture(2, 4, 4, [0, 255, 0, 255]),
            ],
            base: None,
        };
        let mut opts = options();
        opts.layout_mode = "horizontal".into();
        opts.padding = 2;
        let result = build_atlas(&project, &opts, false).unwrap();
        assert_eq!((10, 4), result.image.dimensions());
        assert_eq!([0, 0, 0, 0], result.image.get_pixel(4, 0).0);
        assert_eq!([0, 255, 0, 255], result.image.get_pixel(6, 0).0);
    }

    #[test]
    fn slot_replacement_copies_transparent_pixels_exactly() {
        let base = texture(99, 8, 4, [8, 9, 10, 255]);
        let replacement = texture(1, 4, 4, [0, 0, 0, 0]);
        let project = ProjectSnapshot {
            textures: vec![replacement],
            base: Some(base),
        };
        let mut opts = options();
        opts.assignments.push(crate::models::SlotAssignment {
            texture_id: 1,
            slot: 2,
        });
        let result = build_atlas(&project, &opts, true).unwrap();
        assert_eq!([8, 9, 10, 255], result.image.get_pixel(0, 0).0);
        assert_eq!([0, 0, 0, 0], result.image.get_pixel(4, 0).0);
    }
}
