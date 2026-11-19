use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use glam::Quat;
use kawari::common::{ContainerType, HouseId, Position};
use kawari::ipc::zone::PlotSize;
use serde_derive::{Deserialize, Serialize};

use super::containers::{
    MAX_EXTERIOR_PLACED_SLOTS, MAX_INDOOR_PLACED_SLOTS, indoor_container_for_flat_slot,
    indoor_slot_capacity, outdoor_slot_capacity, placed_container_for_flat_slot,
};
use super::db::{EstateRemakeMeta, FurnitureRemakeMeta, HousingEstateRow, HousingFurnitureRow};
use super::fixtures::{StoredHouseExterior, StoredHouseInterior};
use super::gamedata_ext::{
    HousingCatalog, display_name_for_renovation, normalize_district_style_name,
    renovation_row_for_style_and_size,
};
use super::plots::plot_spec;

/// Scope selector when previewing, applying, or exporting ReMakePlace presets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresetScope {
    #[default]
    All,
    Interior,
    Exterior,
}

impl PresetScope {
    pub fn includes_interior(self) -> bool {
        matches!(self, Self::All | Self::Interior)
    }

    pub fn includes_exterior(self) -> bool {
        matches!(self, Self::All | Self::Exterior)
    }

    pub fn from_str_loose(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "all" | "both" => Some(Self::All),
            "interior" | "indoor" | "indoors" | "in" => Some(Self::Interior),
            "exterior" | "outdoor" | "outdoors" | "out" | "yard" => Some(Self::Exterior),
            _ => None,
        }
    }
}

/// Complete ReMakePlace / MakePlace `.json` schema.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ReMakePlaceLayout {
    pub player_transform: ReMakePlaceTransform,
    pub house_size: String,
    pub interior_scale: f32,
    pub interior_fixture: Vec<ReMakePlaceFixture>,
    pub interior_furniture: Vec<ReMakePlaceFurniture>,
    pub exterior_scale: f32,
    pub exterior_fixture: Vec<ReMakePlaceFixture>,
    pub exterior_furniture: Vec<ReMakePlaceFurniture>,
    pub properties: BTreeMap<String, serde_json::Value>,
}

impl Default for ReMakePlaceLayout {
    fn default() -> Self {
        Self {
            player_transform: ReMakePlaceTransform::default(),
            house_size: String::new(),
            interior_scale: 1.0,
            interior_fixture: Vec::new(),
            interior_furniture: Vec::new(),
            exterior_scale: 1.0,
            exterior_fixture: Vec::new(),
            exterior_furniture: Vec::new(),
            properties: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ReMakePlaceTransform {
    pub location: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Default for ReMakePlaceTransform {
    fn default() -> Self {
        Self {
            location: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ReMakePlaceFixture {
    pub name: String,
    pub item_id: u32,
    pub level: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ReMakePlaceFurniture {
    pub name: String,
    pub item_id: u32,
    pub transform: ReMakePlaceTransform,
    #[serde(default)]
    pub properties: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub attachments: Vec<ReMakePlaceFurniture>,
}

/// Summary produced by dry-run preview or post-apply reporting.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReMakePlacePreviewReport {
    pub house_size: String,
    pub preset_plot_size: Option<u8>,
    pub target_renovation_row_id: Option<u16>,
    pub renovation_territory_changed: bool,
    pub interior_input_count: usize,
    pub interior_resolved_count: usize,
    pub interior_truncated_count: usize,
    pub interior_unresolved_items: Vec<String>,
    pub exterior_input_count: usize,
    pub exterior_resolved_count: usize,
    pub exterior_truncated_count: usize,
    pub exterior_unresolved_items: Vec<String>,
    pub interior_fixtures_matched: usize,
    pub exterior_fixtures_matched: usize,
    pub warnings: Vec<String>,
}

/// Result produced when preparing a ReMakePlace apply batch against an estate.
#[derive(Clone, Debug)]
pub struct PreparedReMakePlaceApply {
    pub preview: ReMakePlacePreviewReport,
    pub updated_estate: HousingEstateRow,
    pub replace_indoor: bool,
    pub replace_outdoor: bool,
    pub furniture_rows: Vec<HousingFurnitureRow>,
}

/// Decodes raw file bytes (UTF-8, UTF-8 BOM, UTF-16LE, or UTF-16BE) into a `ReMakePlaceLayout`.
pub fn parse_layout_bytes(bytes: &[u8]) -> Result<ReMakePlaceLayout, String> {
    let text = decode_text_with_bom(bytes)?;
    serde_json::from_str::<ReMakePlaceLayout>(&text)
        .map_err(|e| format!("Failed to parse ReMakePlace JSON: {e}"))
}

/// Reads and decodes a ReMakePlace `.json` file from disk.
pub fn read_layout_file(path: &Path) -> Result<ReMakePlaceLayout, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("Failed to read preset file '{}': {e}", path.display()))?;
    parse_layout_bytes(&bytes)
}

/// Decodes UTF-8 / UTF-8 BOM / UTF-16LE / UTF-16BE text cleanly.
pub fn decode_text_with_bom(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return std::str::from_utf8(&bytes[3..])
            .map(ToString::to_string)
            .map_err(|e| format!("Invalid UTF-8 BOM JSON: {e}"));
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16(&units).map_err(|e| format!("Invalid UTF-16LE JSON: {e}"));
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16(&units).map_err(|e| format!("Invalid UTF-16BE JSON: {e}"));
    }
    std::str::from_utf8(bytes)
        .map(ToString::to_string)
        .map_err(|e| format!("Invalid UTF-8 JSON: {e}"))
}

/// Converts MakePlace `[X, Y_depth, Z_up]` & quaternion rotation into FFXIV `(Position, rotation_radians)`.
pub fn makeplace_transform_to_ffxiv(
    transform: &ReMakePlaceTransform,
    scale: f32,
) -> (Position, f32) {
    let s = if scale.abs() > 1e-4 { scale } else { 1.0 };
    let pos = Position(glam::Vec3A::new(
        transform.location[0] / s,
        transform.location[2] / s,
        transform.location[1] / s,
    ));
    let q = transform.rotation;
    let rot = -compute_z_angle(q);
    (pos, rot)
}

/// Matches ReMakePlace `QuaternionExtensions.ComputeZAngle(q)` verbatim.
pub fn compute_z_angle(q: [f32; 4]) -> f32 {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let siny_cosp = 2.0 * (w * z + x * y);
    let cosy_cosp = 1.0 - 2.0 * (y * y + z * z);
    siny_cosp.atan2(cosy_cosp)
}

/// Converts FFXIV local coordinates and Y rotation back to MakePlace `[X, Z, Y]` & `[0, 0, sin(-rot/2), cos(-rot/2)]`.
pub fn ffxiv_to_makeplace_transform(
    pos: Position,
    rotation: f32,
    scale: f32,
    preserved_quat: Option<[f32; 4]>,
    preserved_scale: Option<[f32; 3]>,
) -> ReMakePlaceTransform {
    let s = if scale.abs() > 1e-4 { scale } else { 1.0 };
    let zero_clamp = |v: f32| if v.abs() < 0.001 { 0.0 } else { v };
    let q = if let Some(pq) = preserved_quat {
        pq
    } else {
        let quat = Quat::from_rotation_z(-rotation);
        [quat.x, quat.y, quat.z, quat.w]
    };
    ReMakePlaceTransform {
        location: [
            zero_clamp(pos.0.x * s),
            zero_clamp(pos.0.z * s),
            zero_clamp(pos.0.y * s),
        ],
        rotation: q,
        scale: preserved_scale.unwrap_or([1.0, 1.0, 1.0]),
    }
}

/// Parses 6-char `RRGGBB` or 8-char `RRGGBBAA` hex color string into `[r, g, b]`.
pub fn parse_hex_rgb(raw: &str) -> Option<[u8; 3]> {
    let trimmed = raw.trim().trim_start_matches('#');
    if !trimmed.is_ascii() || trimmed.len() < 6 {
        return None;
    }
    let r = u8::from_str_radix(&trimmed[0..2], 16).ok()?;
    let g = u8::from_str_radix(&trimmed[2..4], 16).ok()?;
    let b = u8::from_str_radix(&trimmed[4..6], 16).ok()?;
    Some([r, g, b])
}

fn parse_house_size(raw: &str) -> Option<PlotSize> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "small" | "cottage" | "s" => Some(PlotSize::Small),
        "medium" | "house" | "m" => Some(PlotSize::Medium),
        "large" | "mansion" | "l" => Some(PlotSize::Large),
        _ => None,
    }
}

fn size_to_makeplace_string(plot_size: PlotSize, is_apartment: bool) -> &'static str {
    if is_apartment {
        return "Apartment";
    }
    match plot_size {
        PlotSize::Small => "Small",
        PlotSize::Medium => "Medium",
        PlotSize::Large => "Large",
    }
}

fn flatten_furniture<'a>(
    items: &'a [ReMakePlaceFurniture],
    parent_idx: Option<usize>,
    out: &mut Vec<(&'a ReMakePlaceFurniture, Option<usize>)>,
) {
    for item in items {
        let current_idx = out.len();
        out.push((item, parent_idx));
        if !item.attachments.is_empty() {
            flatten_furniture(&item.attachments, Some(current_idx), out);
        }
    }
}

fn resolve_fixture_stain(fixture: &ReMakePlaceFixture, catalog: &HousingCatalog) -> u8 {
    let (stain, _) = resolve_stain_from_props(&fixture.properties, catalog);
    if stain != 0 {
        return stain;
    }
    if fixture.color.trim().is_empty() {
        return 0;
    }
    parse_hex_rgb(&fixture.color)
        .and_then(|rgb| catalog.closest_housing_stain(rgb))
        .unwrap_or(0)
}

fn resolve_stain_from_props(
    props: &BTreeMap<String, serde_json::Value>,
    catalog: &HousingCatalog,
) -> (u8, Option<String>) {
    if let Some(explicit) = props.get("stainId").and_then(|v| v.as_u64())
        && let Ok(sid) = u8::try_from(explicit)
    {
        return (sid, None);
    }
    let Some(color_hex) = props.get("color").and_then(|v| v.as_str()) else {
        return (0, None);
    };
    let stain = parse_hex_rgb(color_hex)
        .and_then(|rgb| catalog.closest_housing_stain(rgb))
        .unwrap_or(0);
    (stain, Some(color_hex.to_string()))
}

fn resolve_material_from_props(
    furniture_item_id: u32,
    props: &BTreeMap<String, serde_json::Value>,
    catalog: &HousingCatalog,
) -> (u16, Option<u32>) {
    let Some(mat_val) = props.get("material") else {
        return (0, None);
    };
    let mat_item_id = mat_val
        .get("itemId")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .or_else(|| {
            mat_val
                .get("name")
                .and_then(|v| v.as_str())
                .and_then(|name| catalog.resolve_item_by_id_or_name(0, name))
                .map(|m| m.item_id)
        })
        .unwrap_or(0);
    if mat_item_id == 0 {
        return (0, None);
    }
    let vase_row = catalog.resolve_vase_flower_row_id(furniture_item_id, mat_item_id);
    (vase_row, Some(mat_item_id))
}

fn exterior_part_slot_from_type(kind: &str) -> Option<usize> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "roof" => Some(0),
        "exterior wall" | "walls" | "wall" => Some(1),
        "window" | "windows" => Some(2),
        "door" => Some(3),
        "roof decor" | "roof_fixture" => Some(4),
        "exterior wall decor" | "wall decor" | "wall_fixture" => Some(5),
        "placard" | "sign" | "above_door_banner" => Some(6),
        "fence" => Some(7),
        _ => None,
    }
}

fn exterior_type_for_slot(slot: usize) -> &'static str {
    match slot {
        0 => "Roof",
        1 => "Exterior Wall",
        2 => "Window",
        3 => "Door",
        4 => "Roof Decor",
        5 => "Exterior Wall Decor",
        6 => "Placard",
        7 => "Fence",
        _ => "Unknown",
    }
}

/// Validates and compiles a `ReMakePlaceLayout` against `estate` & `catalog` into DB updates and furniture rows.
pub fn prepare_apply_layout(
    estate: &HousingEstateRow,
    layout: &ReMakePlaceLayout,
    catalog: &HousingCatalog,
    scope: PresetScope,
    actor_content_id: u64,
) -> PreparedReMakePlaceApply {
    prepare_apply_layout_with_existing(estate, layout, catalog, scope, actor_content_id, &[])
}

/// Validates and compiles a `ReMakePlaceLayout` while preserving matching existing indoor slots to enable zero-reload live diffs.
pub fn prepare_apply_layout_with_existing(
    estate: &HousingEstateRow,
    layout: &ReMakePlaceLayout,
    catalog: &HousingCatalog,
    scope: PresetScope,
    actor_content_id: u64,
    existing_indoor_rows: &[HousingFurnitureRow],
) -> PreparedReMakePlaceApply {
    let mut updated_estate = estate.clone();
    let target_plot_size =
        PlotSize::from_repr(estate.plot_size.clamp(0, 2) as u8).unwrap_or_default();
    let preset_plot_size = parse_house_size(&layout.house_size);
    let mut warnings = Vec::new();

    if let Some(ps) = preset_plot_size
        && ps != target_plot_size
        && !estate.is_apartment
    {
        warnings.push(format!(
            "Preset houseSize '{:?}' differs from estate size '{:?}'",
            ps, target_plot_size
        ));
    }

    let mut stored_interior = StoredHouseInterior::from_json(&estate.interior_json);
    let prev_renovation_row = stored_interior.renovation_row_id;
    let mut interior_fixtures_matched = 0usize;
    let mut target_renovation_row_id = None;

    if scope.includes_interior() {
        for fixture in &layout.interior_fixture {
            let kind_l = fixture.kind.trim().to_ascii_lowercase();
            let level_l = fixture.level.trim().to_ascii_lowercase();
            if kind_l == "district" {
                if !estate.is_apartment
                    && let Some(row_id) =
                        renovation_row_for_style_and_size(&fixture.name, target_plot_size)
                {
                    stored_interior.renovation_row_id = row_id;
                    target_renovation_row_id = Some(row_id);
                    interior_fixtures_matched += 1;
                } else if !estate.is_apartment
                    && normalize_district_style_name(&fixture.name).is_some()
                {
                    interior_fixtures_matched += 1;
                }
                continue;
            }

            let meta = catalog.resolve_item_by_id_or_name(fixture.item_id, &fixture.name);
            let add_data = meta.map(|m| m.additional_data).unwrap_or(0);
            let stain = resolve_fixture_stain(fixture, catalog);

            let floor_idx = if level_l.contains("ground") {
                Some(0usize)
            } else if level_l.contains("upper") || level_l.contains("top") {
                Some(1usize)
            } else if level_l.contains("base") || level_l.contains("cellar") {
                Some(2usize)
            } else if level_l.contains("main") || level_l.contains("central") {
                Some(3usize)
            } else {
                None
            };

            match kind_l.as_str() {
                "wall" | "walls" => {
                    if add_data != 0 {
                        match floor_idx {
                            Some(0) | None => stored_interior.ground_walls = add_data,
                            Some(1) => stored_interior.top_walls = add_data,
                            Some(2) => stored_interior.cellar_walls = add_data,
                            _ => {}
                        }
                        interior_fixtures_matched += 1;
                    }
                }
                "floor" | "flooring" => {
                    if add_data != 0 {
                        match floor_idx {
                            Some(0) | None => stored_interior.ground_floor = add_data,
                            Some(1) => stored_interior.top_floor = add_data,
                            Some(2) => stored_interior.cellar_floor = add_data,
                            _ => {}
                        }
                        interior_fixtures_matched += 1;
                    }
                }
                "light" | "lighting" | "chandelier" => {
                    if add_data != 0 {
                        match floor_idx {
                            Some(0) | None => stored_interior.ground_chandelier = add_data,
                            Some(1) => stored_interior.top_chandelier = add_data,
                            Some(2) => stored_interior.cellar_chandelier = add_data,
                            Some(3) if target_plot_size == PlotSize::Large => {
                                stored_interior.grand_chandelier = add_data;
                            }
                            _ => {}
                        }
                        interior_fixtures_matched += 1;
                    }
                }
                "window" | "windows" => {
                    let window_id = meta
                        .and_then(|m| {
                            catalog
                                .united_parts_for_item(m.item_id)
                                .map(|parts| parts[2])
                                .or_else(|| catalog.exterior_part_id_for_item(m.item_id))
                        })
                        .unwrap_or(0);
                    if window_id != 0 {
                        stored_interior.window_style = window_id;
                        if stain != 0 {
                            stored_interior.window_stain = stain;
                        }
                        interior_fixtures_matched += 1;
                    }
                }
                "door" => {
                    let door_id = meta
                        .and_then(|m| {
                            catalog
                                .united_parts_for_item(m.item_id)
                                .map(|parts| parts[3])
                                .or_else(|| catalog.exterior_part_id_for_item(m.item_id))
                        })
                        .unwrap_or(0);
                    if door_id != 0 {
                        stored_interior.door_style = door_id;
                        if stain != 0 {
                            stored_interior.door_stain = stain;
                        }
                        interior_fixtures_matched += 1;
                    }
                }
                _ => {}
            }
        }
        if target_plot_size != PlotSize::Large {
            stored_interior.grand_chandelier = 0;
        }
        updated_estate.interior_json = stored_interior.to_json();
    }

    let mut stored_exterior = StoredHouseExterior::from_json(&estate.exterior_json);
    let mut exterior_fixtures_matched = 0usize;
    if scope.includes_exterior() && !estate.is_apartment {
        for fixture in &layout.exterior_fixture {
            let Some(meta) = catalog.resolve_item_by_id_or_name(fixture.item_id, &fixture.name)
            else {
                continue;
            };
            let stain = resolve_fixture_stain(fixture, catalog);
            if let Some(parts) = catalog.united_parts_for_item(meta.item_id) {
                stored_exterior.apply_united_parts(parts);
                if stain != 0 {
                    stored_exterior.set_all_stains(stain);
                }
                exterior_fixtures_matched += 1;
                continue;
            }
            if let Some(part_id) = catalog.exterior_part_id_for_item(meta.item_id)
                && let Some(slot) = exterior_part_slot_from_type(&fixture.kind)
                && stored_exterior.set_part_id(slot, part_id)
            {
                if stain != 0 {
                    stored_exterior.set_stain(slot, stain);
                }
                exterior_fixtures_matched += 1;
            }
        }
        updated_estate.exterior_json = stored_exterior.to_json();
    }

    let mut estate_meta: EstateRemakeMeta =
        serde_json::from_str(&estate.remake_meta_json).unwrap_or_default();
    if !layout.house_size.is_empty() {
        estate_meta.house_size = Some(layout.house_size.clone());
    }
    if scope.includes_interior() {
        estate_meta.interior_scale = Some(layout.interior_scale);
        estate_meta.player_transform = serde_json::to_value(&layout.player_transform).ok();
    }
    if scope.includes_exterior() {
        estate_meta.exterior_scale = Some(layout.exterior_scale);
    }
    if !layout.properties.is_empty() {
        estate_meta.properties = layout.properties.clone();
    }
    updated_estate.remake_meta_json =
        serde_json::to_string(&estate_meta).unwrap_or_else(|_| "{}".to_string());

    let mut furniture_rows = Vec::new();

    let mut interior_input_count = 0usize;
    let mut interior_resolved_count = 0usize;
    let mut interior_truncated_count = 0usize;
    let mut interior_unresolved_items = Vec::new();

    if scope.includes_interior() {
        struct ResolvedIndoorEntry<'a> {
            flat_idx: usize,
            parent_idx: Option<usize>,
            item_id: u32,
            cat_id: u16,
            pos: Position,
            rot: f32,
            stain: u8,
            raw_color_hex: Option<String>,
            material_key: u16,
            mat_item_id: Option<u32>,
            extra_props: BTreeMap<String, serde_json::Value>,
            raw_rotation_quat: [f32; 4],
            raw_scale: [f32; 3],
            _marker: std::marker::PhantomData<&'a ()>,
        }

        let mut flat = Vec::new();
        flatten_furniture(&layout.interior_furniture, None, &mut flat);
        interior_input_count = flat.len();
        let cap = indoor_slot_capacity(target_plot_size, estate.is_apartment)
            .min(MAX_INDOOR_PLACED_SLOTS);
        let mut flat_to_assigned_slot: Vec<Option<u16>> = vec![None; flat.len()];
        let mut resolved_entries = Vec::with_capacity(flat.len());
        let mut candidates = Vec::with_capacity(flat.len());

        for (idx, (entry, parent_idx)) in flat.iter().enumerate() {
            let Some(meta) = catalog.resolve_item_by_id_or_name(entry.item_id, &entry.name) else {
                interior_unresolved_items
                    .push(format!("{} (itemId={})", entry.name, entry.item_id));
                continue;
            };
            let Some(cat_id) = catalog.furniture_catalog_id(meta.item_id, true) else {
                interior_unresolved_items.push(format!(
                    "{} (non-indoor itemId={})",
                    entry.name, meta.item_id
                ));
                continue;
            };

            let (pos, rot) = makeplace_transform_to_ffxiv(&entry.transform, layout.interior_scale);
            let (stain, raw_color_hex) = resolve_stain_from_props(&entry.properties, catalog);
            let (material_key, mat_item_id) =
                resolve_material_from_props(meta.item_id, &entry.properties, catalog);

            let mut extra_props = entry.properties.clone();
            extra_props.remove("color");
            extra_props.remove("material");

            candidates.push(super::live_diff::SlotMatchCandidate {
                catalog_id: cat_id,
                item_id: meta.item_id,
                pos_x: pos.0.x,
                pos_y: pos.0.y,
                pos_z: pos.0.z,
                rotation: rot,
                stain,
                material_item_key: material_key,
            });
            resolved_entries.push(ResolvedIndoorEntry {
                flat_idx: idx,
                parent_idx: *parent_idx,
                item_id: meta.item_id,
                cat_id,
                pos,
                rot,
                stain,
                raw_color_hex,
                material_key,
                mat_item_id,
                extra_props,
                raw_rotation_quat: entry.transform.rotation,
                raw_scale: entry.transform.scale,
                _marker: std::marker::PhantomData,
            });
        }

        let assigned_slots = super::live_diff::assign_indoor_slots_preserving_existing(
            existing_indoor_rows,
            &candidates,
            cap,
        );

        for (entry, assigned_opt) in resolved_entries.iter().zip(assigned_slots.iter()) {
            if let Some(flat_slot) = *assigned_opt {
                flat_to_assigned_slot[entry.flat_idx] = Some(flat_slot);
            }
        }

        let mut indoor_rows: Vec<(u16, HousingFurnitureRow)> =
            Vec::with_capacity(resolved_entries.len());
        for (entry, assigned_opt) in resolved_entries.into_iter().zip(assigned_slots.into_iter()) {
            let Some(flat_slot) = assigned_opt else {
                interior_truncated_count += 1;
                continue;
            };
            let Some((container, page_slot)) = indoor_container_for_flat_slot(flat_slot) else {
                interior_truncated_count += 1;
                continue;
            };
            let parent_slot = entry.parent_idx.and_then(|pi| flat_to_assigned_slot[pi]);

            let fmeta = FurnitureRemakeMeta {
                parent_slot,
                raw_color_hex: entry.raw_color_hex,
                raw_rotation_quat: Some(entry.raw_rotation_quat),
                scale: if entry.raw_scale != [1.0, 1.0, 1.0] {
                    Some(entry.raw_scale)
                } else {
                    None
                },
                material_item_id: entry.mat_item_id,
                extra_properties: entry.extra_props,
            };

            indoor_rows.push((
                flat_slot,
                HousingFurnitureRow {
                    land_ident: estate.land_ident,
                    container_type: container as i32,
                    slot: page_slot as i32,
                    item_id: entry.item_id as i32,
                    catalog_id: entry.cat_id as i32,
                    material_item_key: entry.material_key as i32,
                    stain: entry.stain as i32,
                    placed: true,
                    pos_x: entry.pos.0.x,
                    pos_y: entry.pos.0.y,
                    pos_z: entry.pos.0.z,
                    rotation: entry.rot,
                    object_data_json: "{}".to_string(),
                    remake_meta_json: serde_json::to_string(&fmeta)
                        .unwrap_or_else(|_| "{}".to_string()),
                    created_by_content_id: actor_content_id as i64,
                    updated_at: 0,
                },
            ));
            interior_resolved_count += 1;
        }
        indoor_rows.sort_by_key(|(flat_slot, _)| *flat_slot);
        furniture_rows.extend(indoor_rows.into_iter().map(|(_, row)| row));
    }

    let mut exterior_input_count = 0usize;
    let mut exterior_resolved_count = 0usize;
    let mut exterior_truncated_count = 0usize;
    let mut exterior_unresolved_items = Vec::new();

    if scope.includes_exterior() && !estate.is_apartment {
        let mut flat = Vec::new();
        flatten_furniture(&layout.exterior_furniture, None, &mut flat);
        exterior_input_count = flat.len();
        let cap = outdoor_slot_capacity(target_plot_size).min(MAX_EXTERIOR_PLACED_SLOTS);
        let plot_origin = plot_spec(
            estate.territory_type_id as u16,
            estate.division as u8,
            estate.plot_index as u8,
        );
        if plot_origin.is_none() {
            warnings.push(format!(
                "No Plots.Map entry found for territory {} division {} plot {}; exterior coords left untransformed",
                estate.territory_type_id, estate.division, estate.plot_index
            ));
        }

        let mut flat_to_assigned_slot: Vec<Option<u16>> = vec![None; flat.len()];
        let mut next_slot = 0u16;

        for (idx, (entry, parent_idx)) in flat.iter().enumerate() {
            let Some(meta) = catalog.resolve_item_by_id_or_name(entry.item_id, &entry.name) else {
                exterior_unresolved_items
                    .push(format!("{} (itemId={})", entry.name, entry.item_id));
                continue;
            };
            let Some(cat_id) = catalog.furniture_catalog_id(meta.item_id, false) else {
                exterior_unresolved_items.push(format!(
                    "{} (non-outdoor itemId={})",
                    entry.name, meta.item_id
                ));
                continue;
            };
            if (next_slot as usize) >= cap {
                exterior_truncated_count += 1;
                continue;
            }
            flat_to_assigned_slot[idx] = Some(next_slot);
            let parent_slot = parent_idx.and_then(|pi| flat_to_assigned_slot[pi]);

            let (local_pos, local_rot) =
                makeplace_transform_to_ffxiv(&entry.transform, layout.exterior_scale);
            let (world_pos, world_rot) = if let Some(spec) = plot_origin {
                spec.local_to_world(local_pos, local_rot)
            } else {
                (local_pos, local_rot)
            };

            let (stain, raw_color_hex) = resolve_stain_from_props(&entry.properties, catalog);
            let mut extra_props = entry.properties.clone();
            extra_props.remove("color");

            let fmeta = FurnitureRemakeMeta {
                parent_slot,
                raw_color_hex,
                raw_rotation_quat: Some(entry.transform.rotation),
                scale: if entry.transform.scale != [1.0, 1.0, 1.0] {
                    Some(entry.transform.scale)
                } else {
                    None
                },
                material_item_id: None,
                extra_properties: extra_props,
            };

            furniture_rows.push(HousingFurnitureRow {
                land_ident: estate.land_ident,
                container_type: ContainerType::HousingExteriorPlacedItems as i32,
                slot: next_slot as i32,
                item_id: meta.item_id as i32,
                catalog_id: cat_id as i32,
                material_item_key: 0,
                stain: stain as i32,
                placed: true,
                pos_x: world_pos.0.x,
                pos_y: world_pos.0.y,
                pos_z: world_pos.0.z,
                rotation: world_rot,
                object_data_json: "{}".to_string(),
                remake_meta_json: serde_json::to_string(&fmeta)
                    .unwrap_or_else(|_| "{}".to_string()),
                created_by_content_id: actor_content_id as i64,
                updated_at: 0,
            });
            next_slot += 1;
            exterior_resolved_count += 1;
        }
    }

    let renovation_territory_changed = scope.includes_interior()
        && target_renovation_row_id.is_some_and(|new_id| new_id != prev_renovation_row);

    PreparedReMakePlaceApply {
        preview: ReMakePlacePreviewReport {
            house_size: if layout.house_size.is_empty() {
                size_to_makeplace_string(target_plot_size, estate.is_apartment).to_string()
            } else {
                layout.house_size.clone()
            },
            preset_plot_size: preset_plot_size.map(|s| s as u8),
            target_renovation_row_id,
            renovation_territory_changed,
            interior_input_count,
            interior_resolved_count,
            interior_truncated_count,
            interior_unresolved_items,
            exterior_input_count,
            exterior_resolved_count,
            exterior_truncated_count,
            exterior_unresolved_items,
            interior_fixtures_matched,
            exterior_fixtures_matched,
            warnings,
        },
        updated_estate,
        replace_indoor: scope.includes_interior(),
        replace_outdoor: scope.includes_exterior() && !estate.is_apartment,
        furniture_rows,
    }
}

/// Exports `estate` and its placed furniture back into a lossless ReMakePlace layout JSON struct.
pub fn export_estate_to_layout(
    estate: &HousingEstateRow,
    placed_furniture: &[HousingFurnitureRow],
    catalog: &HousingCatalog,
    scope: PresetScope,
) -> ReMakePlaceLayout {
    let estate_meta: EstateRemakeMeta =
        serde_json::from_str(&estate.remake_meta_json).unwrap_or_default();
    let plot_size = PlotSize::from_repr(estate.plot_size.clamp(0, 2) as u8).unwrap_or_default();

    let mut layout = ReMakePlaceLayout {
        player_transform: estate_meta
            .player_transform
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        house_size: estate_meta.house_size.unwrap_or_else(|| {
            size_to_makeplace_string(plot_size, estate.is_apartment).to_string()
        }),
        interior_scale: estate_meta.interior_scale.unwrap_or(1.0),
        exterior_scale: estate_meta.exterior_scale.unwrap_or(1.0),
        properties: estate_meta.properties,
        ..Default::default()
    };

    if scope.includes_interior() {
        let interior = StoredHouseInterior::from_json(&estate.interior_json);
        let floor_fixtures = [
            ("Ground Floor", "Wall", Some(73u8), interior.ground_walls),
            ("Ground Floor", "Floor", Some(74), interior.ground_floor),
            (
                "Ground Floor",
                "Light",
                Some(75),
                interior.ground_chandelier,
            ),
            ("Upper Floor", "Wall", Some(73), interior.top_walls),
            ("Upper Floor", "Floor", Some(74), interior.top_floor),
            ("Upper Floor", "Light", Some(75), interior.top_chandelier),
            ("Basement", "Wall", Some(73), interior.cellar_walls),
            ("Basement", "Floor", Some(74), interior.cellar_floor),
            ("Basement", "Light", Some(75), interior.cellar_chandelier),
            ("Main", "Light", Some(75), interior.grand_chandelier),
        ];
        for (level, kind, cat, add_data) in floor_fixtures {
            if add_data == 0 {
                continue;
            }
            if estate.is_apartment && level != "Ground Floor" {
                continue;
            }
            if let Some(meta) = catalog.item_for_additional_data(add_data, cat) {
                layout.interior_fixture.push(ReMakePlaceFixture {
                    name: meta.name.clone(),
                    item_id: meta.item_id,
                    level: level.to_string(),
                    kind: kind.to_string(),
                    ..Default::default()
                });
            }
        }
        if interior.renovation_row_id != 0 {
            let name = display_name_for_renovation(interior.renovation_row_id);
            if !name.is_empty() {
                layout.interior_fixture.push(ReMakePlaceFixture {
                    name: name.to_string(),
                    item_id: 0,
                    level: String::new(),
                    kind: "District".to_string(),
                    ..Default::default()
                });
            }
        }
        if interior.window_style != 0
            && let Some(meta) = catalog.item_for_additional_data(interior.window_style as u32, None)
        {
            layout.interior_fixture.push(ReMakePlaceFixture {
                name: meta.name.clone(),
                item_id: meta.item_id,
                level: "Ground Floor".to_string(),
                kind: "Window".to_string(),
                ..Default::default()
            });
        }
        if interior.door_style != 0
            && let Some(meta) = catalog.item_for_additional_data(interior.door_style as u32, None)
        {
            layout.interior_fixture.push(ReMakePlaceFixture {
                name: meta.name.clone(),
                item_id: meta.item_id,
                level: "Ground Floor".to_string(),
                kind: "Door".to_string(),
                ..Default::default()
            });
        }

        layout.interior_furniture =
            export_scope_furniture(placed_furniture, true, layout.interior_scale, None, catalog);
    }

    if scope.includes_exterior() && !estate.is_apartment {
        let exterior = StoredHouseExterior::from_json(&estate.exterior_json);
        if exterior.roof_id != 0
            && let Some(united) = catalog.united_item_for_roof_part(exterior.roof_id)
        {
            layout.exterior_fixture.push(ReMakePlaceFixture {
                name: united.name.clone(),
                item_id: united.item_id,
                level: String::new(),
                kind: "Exterior Wall".to_string(),
                ..Default::default()
            });
        } else {
            let parts = exterior.part_ids();
            for (slot, part_id) in parts.into_iter().enumerate() {
                if part_id == 0 {
                    continue;
                }
                if let Some(meta) =
                    catalog.item_for_additional_data(part_id as u32, Some(65 + slot as u8))
                {
                    layout.exterior_fixture.push(ReMakePlaceFixture {
                        name: meta.name.clone(),
                        item_id: meta.item_id,
                        level: String::new(),
                        kind: exterior_type_for_slot(slot).to_string(),
                        ..Default::default()
                    });
                }
            }
        }

        let plot_origin = plot_spec(
            estate.territory_type_id as u16,
            estate.division as u8,
            estate.plot_index as u8,
        );
        layout.exterior_furniture = export_scope_furniture(
            placed_furniture,
            false,
            layout.exterior_scale,
            plot_origin,
            catalog,
        );
    }

    layout
}

fn export_scope_furniture(
    placed_furniture: &[HousingFurnitureRow],
    indoors: bool,
    scale: f32,
    plot_origin: Option<&super::plots::PlotLayoutSpec>,
    catalog: &HousingCatalog,
) -> Vec<ReMakePlaceFurniture> {
    let mut scoped: Vec<(u16, &HousingFurnitureRow, FurnitureRemakeMeta)> = placed_furniture
        .iter()
        .filter_map(|row| {
            let ctype = super::containers::container_from_i32(row.container_type)?;
            let flat = if indoors {
                super::containers::is_interior_placed_container(ctype)
                    .then(|| {
                        super::containers::flat_slot_for_placed_container(ctype, row.slot as u16)
                    })
                    .flatten()?
            } else {
                (ctype == ContainerType::HousingExteriorPlacedItems).then_some(row.slot as u16)?
            };
            let meta: FurnitureRemakeMeta =
                serde_json::from_str(&row.remake_meta_json).unwrap_or_default();
            Some((flat, row, meta))
        })
        .collect();
    scoped.sort_by_key(|(flat, _, _)| *flat);

    let mut items_by_slot: BTreeMap<u16, ReMakePlaceFurniture> = BTreeMap::new();
    let mut parent_links: Vec<(u16, Option<u16>)> = Vec::with_capacity(scoped.len());

    for (flat, row, fmeta) in scoped {
        let item_id = row.item_id.max(0) as u32;
        let name = catalog
            .items_by_id
            .get(&item_id)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| format!("Item #{item_id}"));

        let world_pos = Position(glam::Vec3A::new(row.pos_x, row.pos_y, row.pos_z));
        let (local_pos, local_rot) = if let Some(spec) = plot_origin {
            spec.world_to_local(world_pos, row.rotation)
        } else {
            (world_pos, row.rotation)
        };

        let mut props = fmeta.extra_properties.clone();
        if let Some(ref raw_hex) = fmeta.raw_color_hex {
            props.insert(
                "color".to_string(),
                serde_json::Value::String(raw_hex.clone()),
            );
        } else if row.stain > 0
            && let Some(hex) = catalog.stain_rgba_hex(row.stain as u8)
        {
            props.insert("color".to_string(), serde_json::Value::String(hex));
        }

        if row.material_item_key > 0 {
            if let Some((mat_id, mat_name)) =
                catalog.material_item_for_vase_flower(row.material_item_key as u16)
            {
                props.insert(
                    "material".to_string(),
                    serde_json::json!({ "name": mat_name, "itemId": mat_id }),
                );
            } else if let Some(mat_id) = fmeta.material_item_id {
                let mat_name = catalog
                    .items_by_id
                    .get(&mat_id)
                    .map(|m| m.name.as_str())
                    .unwrap_or("");
                props.insert(
                    "material".to_string(),
                    serde_json::json!({ "name": mat_name, "itemId": mat_id }),
                );
            }
        }

        let entry = ReMakePlaceFurniture {
            name,
            item_id,
            transform: ffxiv_to_makeplace_transform(
                local_pos,
                local_rot,
                scale,
                fmeta.raw_rotation_quat,
                fmeta.scale,
            ),
            properties: props,
            attachments: Vec::new(),
        };
        items_by_slot.insert(flat, entry);
        parent_links.push((flat, fmeta.parent_slot));
    }

    // Reconstruct attachments bottom-up so grandchildren attach cleanly before their parents attach to roots.
    let mut children_map: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
    let mut roots = Vec::new();
    for (slot, parent) in &parent_links {
        if let Some(p) = parent
            && items_by_slot.contains_key(p)
            && *p != *slot
        {
            children_map.entry(*p).or_default().push(*slot);
        } else {
            roots.push(*slot);
        }
    }

    fn take_subtree(
        slot: u16,
        items: &mut BTreeMap<u16, ReMakePlaceFurniture>,
        children_map: &BTreeMap<u16, Vec<u16>>,
    ) -> Option<ReMakePlaceFurniture> {
        let mut node = items.remove(&slot)?;
        if let Some(child_slots) = children_map.get(&slot) {
            for &cs in child_slots {
                if let Some(child_node) = take_subtree(cs, items, children_map) {
                    node.attachments.push(child_node);
                }
            }
        }
        Some(node)
    }

    let mut result = Vec::new();
    for root_slot in roots {
        if let Some(node) = take_subtree(root_slot, &mut items_by_slot, &children_map) {
            result.push(node);
        }
    }
    // Any cyclic leftovers get appended cleanly as top-level items.
    while let Some((&remaining_slot, _)) = items_by_slot.iter().next() {
        if let Some(node) = take_subtree(remaining_slot, &mut items_by_slot, &children_map) {
            result.push(node);
        }
    }
    result
}

/// Validates a path inside allowed preset/export directories to prevent arbitrary path traversal when browsing over HTTP or Lua.
pub fn resolve_safe_preset_path(raw: &str, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Empty preset path".to_string());
    }
    let candidate = Path::new(trimmed);
    let canon_roots: Vec<PathBuf> = roots
        .iter()
        .filter_map(|r| std::fs::canonicalize(r).ok())
        .collect();
    let is_within_roots =
        |canon_path: &Path| canon_roots.iter().any(|root| canon_path.starts_with(root));

    if candidate.is_absolute() {
        if let Ok(canon) = std::fs::canonicalize(candidate) {
            if is_within_roots(&canon) {
                return Ok(canon);
            }
            return Err(format!(
                "Preset path '{}' is outside configured preset roots",
                candidate.display()
            ));
        }
        return Err(format!(
            "Preset file '{}' does not exist",
            candidate.display()
        ));
    }

    let mut attempts = vec![candidate.to_path_buf()];
    if candidate.extension().is_none() {
        attempts.push(PathBuf::from(format!("{trimmed}.json")));
    }
    for root in &canon_roots {
        for rel in &attempts {
            if let Ok(canon) = std::fs::canonicalize(root.join(rel))
                && canon.starts_with(root)
            {
                return Ok(canon);
            }
        }
    }

    Err(format!("Preset file '{raw}' not found in configured roots"))
}

/// Sanitizes a requested preset export filename into a `.json` filename without doubling `_json.json`.
pub fn sanitize_export_filename(raw_name: &str) -> String {
    let trimmed = raw_name.trim();
    let stem_raw =
        if trimmed.len() >= 5 && trimmed[trimmed.len() - 5..].eq_ignore_ascii_case(".json") {
            &trimmed[..trimmed.len() - 5]
        } else {
            trimmed
        };
    let safe_stem: String = stem_raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe_stem.trim_matches('_').is_empty() {
        "estate_export.json".to_string()
    } else {
        format!("{safe_stem}.json")
    }
}

/// Lists metadata for `.json` layouts inside configured preset directories.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PresetFileSummary {
    pub name: String,
    pub path: String,
    pub house_size: String,
    pub district: String,
    pub indoor_count: usize,
    pub outdoor_count: usize,
    pub modified_unix_ms: u64,
    #[serde(default)]
    pub interior_fixtures: Vec<String>,
    #[serde(default)]
    pub exterior_fixtures: Vec<String>,
}

pub fn scan_preset_directory(dir: &Path) -> Vec<PresetFileSummary> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut results = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.eq_ignore_ascii_case("json"))
            != Some(true)
        {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(layout) = parse_layout_bytes(&bytes) else {
            continue;
        };
        let district = layout
            .interior_fixture
            .iter()
            .find(|f| f.kind.eq_ignore_ascii_case("district"))
            .map(|f| f.name.clone())
            .unwrap_or_default();
        let interior_fixtures = layout
            .interior_fixture
            .iter()
            .filter(|f| {
                !f.name.trim().is_empty()
                    && !f.kind.eq_ignore_ascii_case("district")
                    && !f.kind.eq_ignore_ascii_case("side door")
            })
            .map(|f| {
                if f.kind.trim().is_empty() {
                    f.name.clone()
                } else {
                    format!("{}: {}", f.kind.trim(), f.name.trim())
                }
            })
            .collect();
        let exterior_fixtures = layout
            .exterior_fixture
            .iter()
            .filter(|f| !f.name.trim().is_empty())
            .map(|f| {
                if f.kind.trim().is_empty() {
                    f.name.clone()
                } else {
                    format!("{}: {}", f.kind.trim(), f.name.trim())
                }
            })
            .collect();
        let mut indoor_flat = Vec::new();
        flatten_furniture(&layout.interior_furniture, None, &mut indoor_flat);
        let mut outdoor_flat = Vec::new();
        flatten_furniture(&layout.exterior_furniture, None, &mut outdoor_flat);
        let modified_unix_ms = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("preset")
            .to_string();
        results.push(PresetFileSummary {
            name,
            path: path.to_string_lossy().to_string(),
            house_size: layout.house_size,
            district,
            indoor_count: indoor_flat.len(),
            outdoor_count: outdoor_flat.len(),
            modified_unix_ms,
            interior_fixtures,
            exterior_fixtures,
        });
    }
    results.sort_by(|a, b| b.modified_unix_ms.cmp(&a.modified_unix_ms));
    results
}

/// Ensures `HouseId` is present even if unused directly in pure conversion unit tests.
#[allow(dead_code)]
fn _unused_check(_id: HouseId, _c: Option<(ContainerType, u16)>) {
    let _ = placed_container_for_flat_slot(true, 0);
}

#[cfg(test)]
mod tests {
    use super::super::db::{EnsureEstateSpec, ensure_estate};
    use super::*;
    use crate::WorldDatabase;

    #[test]
    fn remake_place_lossless_roundtrip_utf16le_attachments_outdoor_united_material() {
        let mut world_db = WorldDatabase::new_in_memory();
        let conn = world_db.connection_mut();
        let catalog = HousingCatalog::mock_for_tests();

        let spec = EnsureEstateSpec {
            territory_type_id: 340,
            world_id: 79,
            ward_index: 0,
            division: 0,
            plot_index: 5, // Lavender Beds Plot 6 (Large)
            room_number: 0,
            is_apartment: false,
            plot_size: PlotSize::Large,
            is_fc: false,
            owner_content_id: 42,
            owner_name: "Tester".to_string(),
            estate_name: "Roundtrip House".to_string(),
            exterior_json: "{}".to_string(),
            interior_json: "{}".to_string(),
        };
        let estate = ensure_estate(conn, &spec).unwrap();

        let sample = ReMakePlaceLayout {
            house_size: "Large".to_string(),
            interior_scale: 100.0,
            exterior_scale: 100.0,
            interior_fixture: vec![
                ReMakePlaceFixture {
                    name: "Minimalist".to_string(),
                    item_id: 0,
                    level: String::new(),
                    kind: "District".to_string(),
                    ..Default::default()
                },
                ReMakePlaceFixture {
                    name: "Pure White Interior Wall".to_string(),
                    item_id: 17965,
                    level: "Ground Floor".to_string(),
                    kind: "Wall".to_string(),
                    ..Default::default()
                },
            ],
            exterior_fixture: vec![ReMakePlaceFixture {
                name: "Shirogane Castle Walls".to_string(),
                item_id: 20701,
                level: String::new(),
                kind: "Exterior Wall".to_string(),
                ..Default::default()
            }],
            interior_furniture: vec![ReMakePlaceFurniture {
                name: "White Rectangular Partition".to_string(),
                item_id: 28901,
                transform: ReMakePlaceTransform {
                    location: [250.0, -350.0, 150.0], // cm -> (2.5, 1.5, -3.5)
                    rotation: [0.0, 0.0, 0.70710677, 0.70710677],
                    scale: [1.0, 1.0, 1.0],
                },
                properties: BTreeMap::from([(
                    "color".to_string(),
                    serde_json::Value::String("C02020FF".to_string()),
                )]),
                attachments: vec![ReMakePlaceFurniture {
                    name: "Picture Frame (Gilded)".to_string(),
                    item_id: 16935,
                    transform: ReMakePlaceTransform {
                        location: [250.0, -340.0, 200.0],
                        rotation: [0.0, 0.0, 0.0, 1.0],
                        scale: [1.0, 1.0, 1.0],
                    },
                    properties: BTreeMap::from([(
                        "material".to_string(),
                        serde_json::json!({ "name": "Gridanian Landscape", "itemId": 16940 }),
                    )]),
                    attachments: Vec::new(),
                }],
            }],
            exterior_furniture: vec![ReMakePlaceFurniture {
                name: "Yard Bench".to_string(),
                item_id: 12000,
                transform: ReMakePlaceTransform {
                    location: [400.0, 200.0, 100.0], // plot-local (4.0, 1.0, 2.0)
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                properties: BTreeMap::new(),
                attachments: Vec::new(),
            }],
            ..Default::default()
        };

        // Encode as UTF-16LE with BOM and parse back.
        let json = serde_json::to_string(&sample).unwrap();
        let mut utf16le = vec![0xFFu8, 0xFE];
        for u in json.encode_utf16() {
            utf16le.extend_from_slice(&u.to_le_bytes());
        }
        let decoded = parse_layout_bytes(&utf16le).expect("UTF-16LE BOM must decode");

        let prepared = prepare_apply_layout(&estate, &decoded, &catalog, PresetScope::All, 42);
        assert_eq!(prepared.preview.interior_resolved_count, 2);
        assert_eq!(prepared.preview.exterior_resolved_count, 1);
        assert_eq!(prepared.preview.target_renovation_row_id, Some(18)); // Minimalist Large
        assert_eq!(prepared.furniture_rows[0].stain, 24);
        assert_eq!(prepared.furniture_rows[1].material_item_key, 1015);
        assert!((prepared.furniture_rows[0].pos_x - 2.5).abs() < 1e-4);
        assert!((prepared.furniture_rows[0].pos_y - 1.5).abs() < 1e-4);
        assert!((prepared.furniture_rows[0].pos_z - (-3.5)).abs() < 1e-4);

        // Verify outdoor row was transformed to Zone World space (differs from plot-local origin (143, 23.5, 2)).
        let ext_row = &prepared.furniture_rows[2];
        assert!((ext_row.pos_x - 4.0).abs() > 5.0);

        let exported = export_estate_to_layout(
            &prepared.updated_estate,
            &prepared.furniture_rows,
            &catalog,
            PresetScope::All,
        );
        assert_eq!(exported.interior_furniture.len(), 1);
        assert_eq!(exported.interior_furniture[0].attachments.len(), 1);
        assert_eq!(
            exported.interior_furniture[0].attachments[0].properties["material"]["itemId"],
            serde_json::json!(16940)
        );
        assert_eq!(
            exported.exterior_fixture[0].item_id, 20701,
            "HousingUnitedExterior should collapse on export"
        );
        // Plot-local outdoor coordinate should roundtrip cleanly!
        let ext_loc = &exported.exterior_furniture[0].transform.location;
        assert!((ext_loc[0] - 400.0).abs() < 0.05);
        assert!((ext_loc[1] - 200.0).abs() < 0.05);
        assert!((ext_loc[2] - 100.0).abs() < 0.05);
    }

    #[test]
    fn prepare_apply_layout_extracts_united_window_door_parts_and_fixture_color() {
        let mut world_db = WorldDatabase::new_in_memory();
        let conn = world_db.connection_mut();
        let catalog = HousingCatalog::mock_for_tests();

        let spec = EnsureEstateSpec {
            territory_type_id: 340,
            world_id: 79,
            ward_index: 0,
            division: 0,
            plot_index: 0, // Medium plot
            room_number: 0,
            is_apartment: false,
            plot_size: PlotSize::Medium,
            is_fc: false,
            owner_content_id: 42,
            owner_name: "Tester".to_string(),
            estate_name: "Medium Fixture House".to_string(),
            exterior_json: "{}".to_string(),
            interior_json: "{}".to_string(),
        };
        let estate = ensure_estate(conn, &spec).unwrap();

        let layout = ReMakePlaceLayout {
            house_size: "Medium".to_string(),
            interior_fixture: vec![
                ReMakePlaceFixture {
                    name: "Glade Cottage Wall".to_string(),
                    item_id: 6900,
                    level: "Main".to_string(),
                    kind: "Door".to_string(),
                    color: "C02020FF".to_string(),
                    properties: BTreeMap::new(),
                },
                ReMakePlaceFixture {
                    name: "Shirogane Castle Walls".to_string(),
                    item_id: 20701,
                    level: "Main".to_string(),
                    kind: "Window".to_string(),
                    color: "EFEFEFFF".to_string(),
                    properties: BTreeMap::new(),
                },
                ReMakePlaceFixture {
                    name: "Flat Ceiling Lamp".to_string(),
                    item_id: 17979,
                    level: "Main".to_string(),
                    kind: "Light".to_string(),
                    ..Default::default()
                },
            ],
            exterior_fixture: vec![ReMakePlaceFixture {
                name: "Shirogane Castle Walls".to_string(),
                item_id: 20701,
                level: String::new(),
                kind: "Exterior Wall".to_string(),
                color: "C02020FF".to_string(),
                properties: BTreeMap::new(),
            }],
            ..Default::default()
        };

        let prepared = prepare_apply_layout(&estate, &layout, &catalog, PresetScope::All, 42);
        let stored_int = StoredHouseInterior::from_json(&prepared.updated_estate.interior_json);
        let stored_ext = StoredHouseExterior::from_json(&prepared.updated_estate.exterior_json);

        // Door uses exterior part id (25), not masked AdditionalData; Window uses united parts[2] (503).
        assert_eq!(stored_int.door_style, 25);
        assert_eq!(stored_int.door_stain, 24);
        assert_eq!(stored_int.window_style, 503);
        assert_eq!(stored_int.window_stain, 1);
        assert_eq!(stored_int.grand_chandelier, 0);
        assert_eq!(stored_ext.roof_stain, 24);
    }

    #[test]
    fn prepare_apply_layout_with_existing_preserves_slots_and_parent_links() {
        let mut world_db = WorldDatabase::new_in_memory();
        let conn = world_db.connection_mut();
        let catalog = HousingCatalog::mock_for_tests();

        let estate = crate::housing_sim::db::ensure_estate(
            conn,
            &EnsureEstateSpec {
                territory_type_id: 340,
                world_id: 79,
                ward_index: 0,
                division: 0,
                plot_index: 0,
                room_number: 0,
                is_apartment: false,
                plot_size: PlotSize::Small,
                is_fc: false,
                owner_content_id: 42,
                owner_name: "Tester".to_string(),
                estate_name: "SlotKeep".to_string(),
                exterior_json: StoredHouseExterior::default().to_json(),
                interior_json: StoredHouseInterior::default().to_json(),
            },
        )
        .unwrap();

        let (c5, s5) = indoor_container_for_flat_slot(5).unwrap();
        let existing_parent = HousingFurnitureRow {
            land_ident: estate.land_ident,
            container_type: c5 as i32,
            slot: s5 as i32,
            item_id: 28901,
            catalog_id: 0x014B,
            material_item_key: 0,
            stain: 0,
            placed: true,
            pos_x: 1.0,
            pos_y: 0.0,
            pos_z: 1.0,
            rotation: 0.0,
            ..Default::default()
        };

        let layout = ReMakePlaceLayout {
            house_size: "Small".to_string(),
            interior_scale: 100.0,
            interior_furniture: vec![
                ReMakePlaceFurniture {
                    name: "Picture Frame (Gilded)".to_string(),
                    item_id: 16935,
                    transform: ReMakePlaceTransform {
                        location: [0.0, 0.0, 0.0],
                        rotation: [0.0, 0.0, 0.0, 1.0],
                        scale: [1.0, 1.0, 1.0],
                    },
                    ..Default::default()
                },
                ReMakePlaceFurniture {
                    name: "White Rectangular Partition".to_string(),
                    item_id: 28901,
                    transform: ReMakePlaceTransform {
                        // Slightly moved in ReMakePlace coords (+150cm X -> +1.5 FFXIV X).
                        location: [150.0, 100.0, 0.0],
                        rotation: [0.0, 0.0, 0.0, 1.0],
                        scale: [1.0, 1.0, 1.0],
                    },
                    attachments: vec![ReMakePlaceFurniture {
                        name: "Picture Frame (Gilded)".to_string(),
                        item_id: 16935,
                        transform: ReMakePlaceTransform {
                            location: [150.0, 100.0, 100.0],
                            rotation: [0.0, 0.0, 0.0, 1.0],
                            scale: [1.0, 1.0, 1.0],
                        },
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let prepared = prepare_apply_layout_with_existing(
            &estate,
            &layout,
            &catalog,
            PresetScope::Interior,
            42,
            &[existing_parent],
        );

        assert_eq!(prepared.preview.interior_resolved_count, 3);
        let flat_for = |row: &HousingFurnitureRow| {
            super::super::containers::flat_slot_for_placed_container(
                super::super::containers::container_from_i32(row.container_type).unwrap(),
                row.slot as u16,
            )
            .unwrap()
        };

        // Sorted by flat_slot:
        // slot 0 -> first Picture Frame (brand new)
        // slot 1 -> attachment Picture Frame (brand new)
        // slot 5 -> White Rectangular Partition (kept prior slot 5!)
        assert_eq!(flat_for(&prepared.furniture_rows[0]), 0);
        assert_eq!(flat_for(&prepared.furniture_rows[1]), 1);
        assert_eq!(flat_for(&prepared.furniture_rows[2]), 5);
        assert_eq!(prepared.furniture_rows[2].item_id, 28901);

        // Attachment at slot 1 must reference parent_slot = Some(5)!
        let attach_meta: FurnitureRemakeMeta =
            serde_json::from_str(&prepared.furniture_rows[1].remake_meta_json).unwrap();
        assert_eq!(attach_meta.parent_slot, Some(5));
    }

    #[test]
    fn parse_hex_rgb_rejects_multibyte_utf8_without_panicking() {
        assert_eq!(parse_hex_rgb("#EFEFEFFF"), Some([0xEF, 0xEF, 0xEF]));
        assert_eq!(parse_hex_rgb("빨간색"), None);
        assert_eq!(parse_hex_rgb("#한글색상값"), None);
    }

    #[test]
    fn resolve_safe_preset_path_blocks_traversal_and_outside_absolute_paths() {
        let temp_base = std::env::temp_dir().join(format!(
            "kawari_preset_scope_test_{}_{}",
            std::process::id(),
            fastrand::u64(..)
        ));
        let root = temp_base.join("allowed");
        let outside = temp_base.join("secret.json");
        let inside = root.join("room.json");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&outside, "{}").unwrap();
        std::fs::write(&inside, "{}").unwrap();

        let roots = vec![root.clone()];
        assert!(resolve_safe_preset_path("room", &roots).is_ok());
        assert!(resolve_safe_preset_path("../secret.json", &roots).is_err());
        assert!(resolve_safe_preset_path(outside.to_str().unwrap(), &roots).is_err());

        let _ = std::fs::remove_dir_all(&temp_base);
    }
}
