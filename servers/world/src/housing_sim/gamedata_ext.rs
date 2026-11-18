use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use icarus::HousingFurniture::HousingFurnitureSheet;
use icarus::HousingPreset::HousingPresetSheet;
use icarus::HousingRenovation::HousingRenovationSheet;
use icarus::HousingTrainingDoll::HousingTrainingDollSheet;
use icarus::HousingUnitedExterior::HousingUnitedExteriorSheet;
use icarus::HousingYardObject::HousingYardObjectSheet;
use icarus::Stain::StainSheet;
use icarus::VaseFlower::VaseFlowerSheet;
use kawari::common::{Position, internal_housing_row};
use kawari::config::get_config;
use kawari::ipc::zone::{CommonSpawn, PlotSize};
use physis::Language;
use physis::savedata::chardat::CustomizeData;

use super::fixtures::{StoredHouseExterior, StoredHouseInterior};
use crate::GameData;

const HOUSING_FURNITURE_ROW_PREFIX: u32 = 0x0003_0000;
const HOUSING_YARD_OBJECT_ROW_PREFIX: u32 = 0x0002_0000;
const HOUSING_YARD_OBJECT_USAGE_STRIKING_DUMMY: u8 = 16;
const DEFAULT_STRIKING_DUMMY_BNPC_BASE_ID: u32 = 541;

#[derive(Clone, Debug)]
pub struct HousingStrikingDummyNpcData {
    pub base_id: u32,
    pub model_chara: u16,
    pub battalion: u8,
    pub customize: CustomizeData,
    pub rank: u8,
    pub equip: CommonSpawn,
}

#[derive(Clone, Copy, Debug)]
pub struct HousingRenovationContext {
    pub row_id: u16,
    pub territory_type_id: u16,
    pub plot_size: PlotSize,
    pub style_key: &'static str,
}

/// All 21 verified indoor renovation shells (`HousingRenovation` rows `1..=21`).
pub const HOUSING_RENOVATION_CONTEXTS: [HousingRenovationContext; 21] = [
    HousingRenovationContext {
        row_id: 1,
        territory_type_id: 282,
        plot_size: PlotSize::Small,
        style_key: "mist",
    },
    HousingRenovationContext {
        row_id: 2,
        territory_type_id: 283,
        plot_size: PlotSize::Medium,
        style_key: "mist",
    },
    HousingRenovationContext {
        row_id: 3,
        territory_type_id: 284,
        plot_size: PlotSize::Large,
        style_key: "mist",
    },
    HousingRenovationContext {
        row_id: 4,
        territory_type_id: 342,
        plot_size: PlotSize::Small,
        style_key: "lavender beds",
    },
    HousingRenovationContext {
        row_id: 5,
        territory_type_id: 343,
        plot_size: PlotSize::Medium,
        style_key: "lavender beds",
    },
    HousingRenovationContext {
        row_id: 6,
        territory_type_id: 344,
        plot_size: PlotSize::Large,
        style_key: "lavender beds",
    },
    HousingRenovationContext {
        row_id: 7,
        territory_type_id: 345,
        plot_size: PlotSize::Small,
        style_key: "goblet",
    },
    HousingRenovationContext {
        row_id: 8,
        territory_type_id: 346,
        plot_size: PlotSize::Medium,
        style_key: "goblet",
    },
    HousingRenovationContext {
        row_id: 9,
        territory_type_id: 347,
        plot_size: PlotSize::Large,
        style_key: "goblet",
    },
    HousingRenovationContext {
        row_id: 10,
        territory_type_id: 649,
        plot_size: PlotSize::Small,
        style_key: "shirogane",
    },
    HousingRenovationContext {
        row_id: 11,
        territory_type_id: 650,
        plot_size: PlotSize::Medium,
        style_key: "shirogane",
    },
    HousingRenovationContext {
        row_id: 12,
        territory_type_id: 651,
        plot_size: PlotSize::Large,
        style_key: "shirogane",
    },
    HousingRenovationContext {
        row_id: 13,
        territory_type_id: 980,
        plot_size: PlotSize::Small,
        style_key: "empyreum",
    },
    HousingRenovationContext {
        row_id: 14,
        territory_type_id: 981,
        plot_size: PlotSize::Medium,
        style_key: "empyreum",
    },
    HousingRenovationContext {
        row_id: 15,
        territory_type_id: 982,
        plot_size: PlotSize::Large,
        style_key: "empyreum",
    },
    HousingRenovationContext {
        row_id: 16,
        territory_type_id: 1249,
        plot_size: PlotSize::Small,
        style_key: "minimalist",
    },
    HousingRenovationContext {
        row_id: 17,
        territory_type_id: 1250,
        plot_size: PlotSize::Medium,
        style_key: "minimalist",
    },
    HousingRenovationContext {
        row_id: 18,
        territory_type_id: 1251,
        plot_size: PlotSize::Large,
        style_key: "minimalist",
    },
    HousingRenovationContext {
        row_id: 19,
        territory_type_id: 1374,
        plot_size: PlotSize::Small,
        style_key: "dark minimalist",
    },
    HousingRenovationContext {
        row_id: 20,
        territory_type_id: 1375,
        plot_size: PlotSize::Medium,
        style_key: "dark minimalist",
    },
    HousingRenovationContext {
        row_id: 21,
        territory_type_id: 1376,
        plot_size: PlotSize::Large,
        style_key: "dark minimalist",
    },
];

pub fn default_renovation_row_for_district_and_size(
    district_territory_id: u16,
    plot_size: PlotSize,
) -> u16 {
    let base = match district_territory_id {
        339 => 1,  // Mist
        340 => 4,  // Lavender Beds
        341 => 7,  // Goblet
        641 => 10, // Shirogane
        979 => 13, // Empyreum
        _ => 16,   // Minimalist
    };
    base + (plot_size as u16)
}

pub fn normalize_district_style_name(name: &str) -> Option<&'static str> {
    let lower = name.trim().to_ascii_lowercase();
    if lower.contains("dark") && lower.contains("minimal") {
        Some("dark minimalist")
    } else if lower.contains("minimal") {
        Some("minimalist")
    } else if lower.contains("mist") {
        Some("mist")
    } else if lower.contains("lavender") {
        Some("lavender beds")
    } else if lower.contains("goblet") {
        Some("goblet")
    } else if lower.contains("shirogane") {
        Some("shirogane")
    } else if lower.contains("empyreum") || lower.contains("ishgard") {
        Some("empyreum")
    } else {
        None
    }
}

pub fn renovation_row_for_style_and_size(style_name: &str, plot_size: PlotSize) -> Option<u16> {
    let key = normalize_district_style_name(style_name)?;
    HOUSING_RENOVATION_CONTEXTS
        .iter()
        .find(|ctx| ctx.style_key == key && ctx.plot_size == plot_size)
        .map(|ctx| ctx.row_id)
}

pub fn renovation_territory_for_row(row_id: u16) -> Option<u16> {
    HOUSING_RENOVATION_CONTEXTS
        .iter()
        .find(|ctx| ctx.row_id == row_id)
        .map(|ctx| ctx.territory_type_id)
}

pub fn renovation_row_for_territory(zone_id: u16) -> Option<u16> {
    HOUSING_RENOVATION_CONTEXTS
        .iter()
        .find(|ctx| ctx.territory_type_id == zone_id)
        .map(|ctx| ctx.row_id)
}

pub fn display_name_for_renovation(row_id: u16) -> &'static str {
    let Some(ctx) = HOUSING_RENOVATION_CONTEXTS
        .iter()
        .find(|c| c.row_id == row_id)
    else {
        return "";
    };
    match ctx.style_key {
        "mist" => "Mist",
        "lavender beds" => "Lavender Beds",
        "goblet" => "Goblet",
        "shirogane" => "Shirogane",
        "empyreum" => "Empyreum",
        "minimalist" => "Minimalist",
        "dark minimalist" => "Dark Minimalist",
        _ => "",
    }
}

#[derive(Clone, Debug)]
pub struct CatalogItemMeta {
    pub item_id: u32,
    pub name: String,
    pub ui_category: u8,
    pub additional_data: u32,
}

#[derive(Clone, Debug)]
pub struct StainMeta {
    pub id: u8,
    pub color_argb: u32,
    pub housing_applicable: bool,
}

#[derive(Clone, Debug)]
pub struct DefaultDistrictPreset {
    pub place_name: u16,
    pub plot_size: PlotSize,
    pub exterior: StoredHouseExterior,
    pub interior: StoredHouseInterior,
}

#[derive(Clone, Debug, Default)]
pub struct HousingCatalog {
    pub items_by_id: HashMap<u32, CatalogItemMeta>,
    pub items_by_lower_name: HashMap<String, u32>,
    pub items_by_additional_data: HashMap<u32, u32>,
    pub items_by_additional_and_cat: HashMap<(u32, u8), u32>,
    pub stains: Vec<StainMeta>,
    pub stains_by_id: HashMap<u8, u32>,
    pub united_parts_by_row: HashMap<u32, [u16; 8]>,
    pub united_row_by_roof: HashMap<u16, u32>,
    pub interactable_indoor_catalogs: HashMap<u16, bool>,
    pub interactable_outdoor_catalogs: HashMap<u16, bool>,
    pub striking_dummy_bnpc_by_catalog: HashMap<u16, u32>,
    pub vase_flower_by_row: HashMap<u16, u32>,
    pub vase_flower_rows_by_item: HashMap<u32, Vec<u16>>,
    pub renovation_names: HashMap<u16, String>,
    pub presets: Vec<DefaultDistrictPreset>,
}

static GLOBAL_CATALOG: OnceLock<Arc<HousingCatalog>> = OnceLock::new();

pub fn cached_housing_catalog(game_data: &mut GameData) -> Arc<HousingCatalog> {
    GLOBAL_CATALOG
        .get_or_init(|| Arc::new(HousingCatalog::from_game_data(game_data)))
        .clone()
}

macro_rules! read_sheet_flexible {
    ($sheet:ty, $res:expr, $lang:expr) => {
        <$sheet>::read_from($res, Language::None)
            .ok()
            .or_else(|| <$sheet>::read_from($res, $lang).ok())
    };
}

fn place_name_id_for_district(district_territory_id: u16) -> Option<u16> {
    match district_territory_id {
        339 => Some(24),   // Mist
        340 => Some(35),   // Lavender Beds
        341 => Some(66),   // Goblet
        641 => Some(2617), // Shirogane
        979 => Some(3929), // Empyreum
        _ => None,
    }
}

impl HousingCatalog {
    /// Builds an indexed housing catalog from loaded `GameData`.
    pub fn from_game_data(game_data: &mut GameData) -> Self {
        let mut catalog = Self::default();
        let lang = get_config().world.language();

        for (id, row) in (&game_data.item_sheet).into_iter().flatten_subrows() {
            let name = row.Name.to_string();
            let meta = CatalogItemMeta {
                item_id: id,
                name: name.clone(),
                ui_category: row.ItemUICategory,
                additional_data: row.AdditionalData,
            };
            if !name.is_empty() {
                catalog
                    .items_by_lower_name
                    .entry(name.to_ascii_lowercase())
                    .or_insert(id);
            }
            if row.AdditionalData != 0 {
                catalog
                    .items_by_additional_data
                    .entry(row.AdditionalData)
                    .or_insert(id);
                catalog
                    .items_by_additional_and_cat
                    .entry((row.AdditionalData, row.ItemUICategory))
                    .or_insert(id);
            }
            catalog.items_by_id.insert(id, meta);
        }

        if let Some(sheet) = read_sheet_flexible!(StainSheet, &mut game_data.resource, lang) {
            for (row_id, row) in (&sheet).into_iter().flatten_subrows() {
                let Ok(id) = u8::try_from(row_id) else {
                    continue;
                };
                if id == 0 {
                    continue;
                }
                catalog.stains_by_id.insert(id, row.Color);
                catalog.stains.push(StainMeta {
                    id,
                    color_argb: row.Color,
                    housing_applicable: row.IsHousingApplicable,
                });
            }
        }

        if let Some(sheet) =
            read_sheet_flexible!(HousingUnitedExteriorSheet, &mut game_data.resource, lang)
        {
            for (row_id, row) in (&sheet).into_iter().flatten_subrows() {
                let parts = [
                    (row.Roof & 0xFFFF) as u16,
                    (row.Walls & 0xFFFF) as u16,
                    (row.Windows & 0xFFFF) as u16,
                    (row.Door & 0xFFFF) as u16,
                    (row.OptionalRoof & 0xFFFF) as u16,
                    (row.OptionalWall & 0xFFFF) as u16,
                    (row.OptionalSignboard & 0xFFFF) as u16,
                    (row.Fence & 0xFFFF) as u16,
                ];
                if parts[0] != 0 {
                    catalog.united_row_by_roof.entry(parts[0]).or_insert(row_id);
                }
                catalog.united_parts_by_row.insert(row_id, parts);
            }
        }

        if let Some(sheet) =
            read_sheet_flexible!(HousingFurnitureSheet, &mut game_data.resource, lang)
        {
            for (row_id, row) in (&sheet).into_iter().flatten_subrows() {
                let cat = (row_id & 0xFFFF) as u16;
                if row.UsageType != 0 {
                    catalog.interactable_indoor_catalogs.insert(cat, true);
                }
            }
        }

        let training_doll_sheet =
            read_sheet_flexible!(HousingTrainingDollSheet, &mut game_data.resource, lang);

        if let Some(sheet) =
            read_sheet_flexible!(HousingYardObjectSheet, &mut game_data.resource, lang)
        {
            for (row_id, row) in (&sheet).into_iter().flatten_subrows() {
                let cat = (row_id & 0xFFFF) as u16;
                if row.UsageType != 0 {
                    catalog.interactable_outdoor_catalogs.insert(cat, true);
                }
                if row.UsageType == HOUSING_YARD_OBJECT_USAGE_STRIKING_DUMMY {
                    let base_id = training_doll_sheet
                        .as_ref()
                        .and_then(|tds| tds.row(row.UsageParameter))
                        .map(|td| td.Base)
                        .filter(|&b| b != 0)
                        .unwrap_or(DEFAULT_STRIKING_DUMMY_BNPC_BASE_ID);
                    catalog.striking_dummy_bnpc_by_catalog.insert(cat, base_id);
                }
            }
        }

        if let Some(sheet) = read_sheet_flexible!(VaseFlowerSheet, &mut game_data.resource, lang) {
            for (row_id, row) in (&sheet).into_iter().flatten_subrows() {
                let Ok(short_id) = u16::try_from(row_id) else {
                    continue;
                };
                if short_id == 0 || row.Item == 0 {
                    continue;
                }
                catalog.vase_flower_by_row.insert(short_id, row.Item);
                catalog
                    .vase_flower_rows_by_item
                    .entry(row.Item)
                    .or_default()
                    .push(short_id);
            }
        }

        if let Some(sheet) =
            read_sheet_flexible!(HousingRenovationSheet, &mut game_data.resource, lang)
        {
            for (row_id, row) in (&sheet).into_iter().flatten_subrows() {
                if let Ok(r) = u16::try_from(row_id) {
                    catalog.renovation_names.insert(r, row.Name.to_string());
                }
            }
        }

        if let Some(sheet) = read_sheet_flexible!(HousingPresetSheet, &mut game_data.resource, lang)
        {
            for (_row_id, row) in (&sheet).into_iter().flatten_subrows() {
                let Some(plot_size) = PlotSize::from_repr(row.HousingSize) else {
                    continue;
                };
                let mut exterior = StoredHouseExterior::default();
                let ext_pairs = [
                    (0, row.ExteriorRoof),
                    (1, row.ExteriorWall),
                    (2, row.ExteriorWindow),
                    (3, row.ExteriorDoor),
                ];
                for (slot, raw_item) in ext_pairs {
                    if raw_item > 0
                        && let Some(part_id) = catalog.exterior_part_id_for_item(raw_item as u32)
                    {
                        exterior.set_part_id(slot, part_id);
                    }
                }
                let mut interior = StoredHouseInterior::default();
                let int_pairs = [
                    (0, row.InteriorWall),
                    (1, row.InteriorFlooring),
                    (2, row.InteriorLighting),
                    (3, row.OtherFloorWall),
                    (4, row.OtherFloorFlooring),
                    (5, row.OtherFloorLighting),
                    (6, row.BasementWall),
                    (7, row.BasementFlooring),
                    (8, row.BasementLighting),
                    (9, row.MansionLighting),
                ];
                for (idx, raw_item) in int_pairs {
                    if raw_item > 0
                        && let Some(meta) = catalog.items_by_id.get(&(raw_item as u32))
                        && meta.additional_data != 0
                    {
                        interior.set_slot_by_index(idx, meta.additional_data);
                    }
                }
                catalog.presets.push(DefaultDistrictPreset {
                    place_name: row.PlaceName,
                    plot_size,
                    exterior,
                    interior,
                });
            }
        }

        catalog
    }

    /// Lightweight deterministic catalog for unit tests without local SqPack game files.
    pub fn mock_for_tests() -> Self {
        let mut catalog = Self::default();
        let seed_items = [
            (28901, "White Rectangular Partition", 77u8, 0x0003_014Bu32),
            (16935, "Picture Frame (Gilded)", 79, 0x0003_0091),
            (28931, "Small Angler's Canvas", 79, 0x0003_0165),
            (21873, "Alpine Chair", 57, 0x0003_0012),
            (20701, "Shirogane Castle Walls", 66, 0x0005_000A),
            (6898, "Glade Cottage Roof", 65, 24),
            (6900, "Glade Cottage Wall", 66, 25),
            (17965, "Pure White Interior Wall", 73, 66111),
            (17972, "Light Wood Flooring", 74, 65591),
            (17979, "Flat Ceiling Lamp", 75, 65848),
            (12000, "Yard Bench", 76, 0x0002_002A),
            (16940, "Gridanian Landscape", 16, 0),
        ];
        for (id, name, ui_cat, add) in seed_items {
            catalog.items_by_id.insert(
                id,
                CatalogItemMeta {
                    item_id: id,
                    name: name.to_string(),
                    ui_category: ui_cat,
                    additional_data: add,
                },
            );
            catalog
                .items_by_lower_name
                .insert(name.to_ascii_lowercase(), id);
            if add != 0 {
                catalog.items_by_additional_data.insert(add, id);
                catalog
                    .items_by_additional_and_cat
                    .insert((add, ui_cat), id);
            }
        }
        catalog.stains.push(StainMeta {
            id: 1,
            color_argb: 0xFFEFEFEF,
            housing_applicable: true,
        });
        catalog.stains.push(StainMeta {
            id: 24,
            color_argb: 0xFFC02020,
            housing_applicable: true,
        });
        catalog.stains_by_id.insert(1, 0xFFEFEFEF);
        catalog.stains_by_id.insert(24, 0xFFC02020);

        catalog
            .united_parts_by_row
            .insert(0x0005_000A, [501, 502, 503, 504, 505, 506, 507, 508]);
        catalog.united_row_by_roof.insert(501, 0x0005_000A);

        catalog.vase_flower_by_row.insert(1015, 16940);
        catalog.vase_flower_rows_by_item.insert(16940, vec![1015]);
        catalog
    }

    pub fn resolve_item_by_id_or_name(&self, item_id: u32, name: &str) -> Option<&CatalogItemMeta> {
        if item_id != 0
            && let Some(meta) = self.items_by_id.get(&item_id)
        {
            return Some(meta);
        }
        let lower = name.trim().to_ascii_lowercase();
        if lower.is_empty() {
            return None;
        }
        self.items_by_lower_name
            .get(&lower)
            .and_then(|id| self.items_by_id.get(id))
    }

    pub fn furniture_catalog_id(&self, item_id: u32, indoors: bool) -> Option<u16> {
        let meta = self.items_by_id.get(&item_id)?;
        let add = meta.additional_data;
        if add == 0 {
            return None;
        }
        let sheet_prefix = add >> 16;
        if indoors && sheet_prefix == 0x0003 {
            return Some((add & 0xFFFF) as u16);
        }
        if !indoors && sheet_prefix == 0x0002 {
            return Some((add & 0xFFFF) as u16);
        }
        if matches!(meta.ui_category, 57 | 76 | 77 | 78 | 79 | 80) {
            return Some((add & 0xFFFF) as u16);
        }
        None
    }

    pub fn exterior_part_id_for_item(&self, item_id: u32) -> Option<u16> {
        let meta = self.items_by_id.get(&item_id)?;
        if meta.additional_data == 0 {
            return None;
        }
        let sheet_prefix = meta.additional_data >> 16;
        if sheet_prefix == 0x0000 {
            Some((meta.additional_data & 0xFFFF) as u16)
        } else {
            None
        }
    }

    pub fn united_parts_for_item(&self, item_id: u32) -> Option<[u16; 8]> {
        let meta = self.items_by_id.get(&item_id)?;
        let add = meta.additional_data;
        self.united_parts_by_row
            .get(&add)
            .or_else(|| self.united_parts_by_row.get(&(add & 0xFFFF)))
            .copied()
    }

    pub fn united_item_for_roof_part(&self, roof_fixture_id: u16) -> Option<&CatalogItemMeta> {
        let row_id = *self.united_row_by_roof.get(&roof_fixture_id)?;
        let item_id = *self
            .items_by_additional_data
            .get(&row_id)
            .or_else(|| self.items_by_additional_data.get(&(0x0005_0000 | row_id)))?;
        self.items_by_id.get(&item_id)
    }

    pub fn item_for_additional_data(
        &self,
        additional_data: u32,
        ui_category: Option<u8>,
    ) -> Option<&CatalogItemMeta> {
        if additional_data == 0 {
            return None;
        }
        if let Some(cat) = ui_category
            && let Some(item_id) = self
                .items_by_additional_and_cat
                .get(&(additional_data, cat))
        {
            return self.items_by_id.get(item_id);
        }
        self.items_by_additional_data
            .get(&additional_data)
            .and_then(|id| self.items_by_id.get(id))
    }

    /// Computes closest housing-applicable stain ID (`Stain.IsHousingApplicable == true`).
    pub fn closest_housing_stain(&self, rgb: [u8; 3]) -> Option<u8> {
        let mut best_id = None;
        let mut min_dist = 2000i32;

        for stain in &self.stains {
            if !stain.housing_applicable {
                continue;
            }
            let sr = ((stain.color_argb >> 16) & 0xFF) as i32;
            let sg = ((stain.color_argb >> 8) & 0xFF) as i32;
            let sb = (stain.color_argb & 0xFF) as i32;
            let dr = sr - rgb[0] as i32;
            let dg = sg - rgb[1] as i32;
            let db = sb - rgb[2] as i32;
            let dist = ((dr * dr + dg * dg + db * db) as f64).sqrt() as i32;
            if dist < min_dist {
                min_dist = dist;
                best_id = Some(stain.id);
            }
        }
        best_id
    }

    pub fn stain_rgba_hex(&self, stain_id: u8) -> Option<String> {
        if stain_id == 0 {
            return None;
        }
        let argb = *self.stains_by_id.get(&stain_id)?;
        let r = (argb >> 16) & 0xFF;
        let g = (argb >> 8) & 0xFF;
        let b = argb & 0xFF;
        Some(format!("{r:02X}{g:02X}{b:02X}FF"))
    }

    pub fn resolve_vase_flower_row_id(&self, furniture_item_id: u32, material_item_id: u32) -> u16 {
        let Some(candidates) = self.vase_flower_rows_by_item.get(&material_item_id) else {
            return 0;
        };
        let preferred_range = match furniture_item_id {
            28931 => Some(3001..=3999),
            28932 => Some(4001..=4999),
            28933 => Some(5001..=5999),
            28934 => Some(6001..=6999),
            16935..=16937 => Some(1001..=1999),
            _ => None,
        };
        if let Some(range) = preferred_range
            && let Some(&matched) = candidates.iter().find(|&&id| range.contains(&id))
        {
            return matched;
        }
        candidates
            .iter()
            .copied()
            .find(|&id| (2001..=2999).contains(&id))
            .or_else(|| candidates.iter().copied().find(|&id| id < 1000))
            .or_else(|| candidates.first().copied())
            .unwrap_or(0)
    }

    pub fn material_item_for_vase_flower(&self, vase_row_id: u16) -> Option<(u32, &str)> {
        if vase_row_id == 0 {
            return None;
        }
        let item_id = *self.vase_flower_by_row.get(&vase_row_id)?;
        let name = self
            .items_by_id
            .get(&item_id)
            .map(|m| m.name.as_str())
            .unwrap_or("");
        Some((item_id, name))
    }

    pub fn is_interactable(&self, catalog_id: u16, indoors: bool) -> bool {
        if indoors {
            self.interactable_indoor_catalogs
                .get(&catalog_id)
                .copied()
                .unwrap_or(false)
        } else {
            self.interactable_outdoor_catalogs
                .get(&catalog_id)
                .copied()
                .unwrap_or(false)
        }
    }

    pub fn striking_dummy_npc_data(
        &self,
        game_data: &mut GameData,
        catalog_id: u16,
    ) -> Option<HousingStrikingDummyNpcData> {
        let base_id = *self.striking_dummy_bnpc_by_catalog.get(&catalog_id)?;
        let (model_chara, battalion, customize, rank, equip_id, _behavior) =
            game_data.find_bnpc(base_id)?;
        let equip = game_data.get_npc_equip(equip_id as u32).unwrap_or_default();
        Some(HousingStrikingDummyNpcData {
            base_id,
            model_chara,
            battalion,
            customize,
            rank,
            equip,
        })
    }

    pub fn default_preset_for_district(
        &self,
        district_territory_id: u16,
        plot_size: PlotSize,
    ) -> Option<DefaultDistrictPreset> {
        if let Some(place_id) = place_name_id_for_district(district_territory_id)
            && let Some(found) = self
                .presets
                .iter()
                .find(|p| p.place_name == place_id && p.plot_size == plot_size)
        {
            return Some(found.clone());
        }
        if let Some(row) = internal_housing_row(district_territory_id)
            && let Some(found) = self
                .presets
                .iter()
                .filter(|p| p.plot_size == plot_size)
                .nth(row as usize)
        {
            return Some(found.clone());
        }
        self.presets
            .iter()
            .find(|p| p.plot_size == plot_size)
            .cloned()
    }

    pub fn sample_kit_item_ids(&self, kit_kind: &str) -> Vec<u32> {
        let prefix = match kit_kind.trim().to_ascii_lowercase().as_str() {
            "indoor" => Some(HOUSING_FURNITURE_ROW_PREFIX),
            "outdoor" => Some(HOUSING_YARD_OBJECT_ROW_PREFIX),
            "npc" => {
                // Vendor permits / tabletop / mender permits in HousingFurniture
                return self
                    .interactable_indoor_catalogs
                    .keys()
                    .take(15)
                    .filter_map(|&cat| {
                        self.items_by_additional_data
                            .get(&(HOUSING_FURNITURE_ROW_PREFIX | cat as u32))
                            .copied()
                    })
                    .collect();
            }
            _ => None,
        };
        let Some(prefix) = prefix else {
            return Vec::new();
        };
        self.items_by_id
            .values()
            .filter(|m| (m.additional_data & 0xFFFF_0000) == prefix)
            .take(35)
            .map(|m| m.item_id)
            .collect()
    }
}

/// Normalizes a position vector into `[f32; 3]` for JSON export.
pub fn position_to_array(pos: Position) -> [f32; 3] {
    [pos.0.x, pos.0.y, pos.0.z]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_758_gamedata_housing_catalog_loads_if_available() {
        let Some(game_data) = crate::test_game_data() else {
            return;
        };
        let catalog = HousingCatalog::from_game_data(game_data);
        assert!(!catalog.items_by_id.is_empty());
        assert!(!catalog.stains.is_empty());
        assert!(!catalog.united_parts_by_row.is_empty());
        assert!(!catalog.vase_flower_by_row.is_empty());
        assert!(catalog.closest_housing_stain([240, 240, 240]).is_some());
    }
}
