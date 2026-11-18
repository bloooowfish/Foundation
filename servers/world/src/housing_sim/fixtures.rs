use kawari::common::{HouseId, HousingFlag};
use kawari::ipc::zone::{
    ApartmentListEntry, ExteriorRemodelItemOperation, Furniture, FurnitureList, House,
    HouseExterior, HousingAppealTag, HousingExteriorAppearanceUpdate, HousingFlags,
    HousingInteriorDetails, InteriorRemodelItemOperation, PlotSize,
};
use serde_derive::{Deserialize, Serialize};

use super::containers::{
    MAX_INDOOR_PLACED_SLOTS, flat_slot_for_placed_container, indoor_slot_capacity,
};
use super::db::{HousingEstateRow, HousingFurnitureRow, plot_state_for_row};
use super::gamedata_ext::HousingCatalog;

pub const DEFAULT_MINIMAL_WALL: u32 = 66111; // Pure White Interior Wall
pub const DEFAULT_MINIMAL_FLOOR: u32 = 65591; // Light Wood Flooring
pub const DEFAULT_MINIMAL_LIGHT: u32 = 65848; // Flat Ceiling Lamp

/// Stored JSON shape for `housing_estates.exterior_json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StoredHouseExterior {
    pub roof_id: u16,
    pub walls_id: u16,
    pub windows_id: u16,
    pub door_id: u16,
    pub roof_fixture_id: u16,
    pub wall_fixture_id: u16,
    pub above_door_banner_id: u16,
    pub fence_id: u16,
    pub roof_stain: u8,
    pub walls_stain: u8,
    pub windows_stain: u8,
    pub door_stain: u8,
    pub roof_fixture_stain: u8,
    pub wall_fixture_stain: u8,
    pub above_door_banner_stain: u8,
    pub fence_stain: u8,
}

impl Default for StoredHouseExterior {
    fn default() -> Self {
        Self {
            roof_id: 24,
            walls_id: 25,
            windows_id: 26,
            door_id: 27,
            roof_fixture_id: 0,
            wall_fixture_id: 0,
            above_door_banner_id: 0,
            fence_id: 28,
            roof_stain: 0,
            walls_stain: 0,
            windows_stain: 0,
            door_stain: 0,
            roof_fixture_stain: 0,
            wall_fixture_stain: 0,
            above_door_banner_stain: 0,
            fence_stain: 0,
        }
    }
}

impl StoredHouseExterior {
    pub fn from_json(raw: &str) -> Self {
        serde_json::from_str(raw).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Creates a district + plot-size appropriate default exterior from `HousingPreset` when available.
    pub fn default_for_location(
        catalog: &HousingCatalog,
        territory_type_id: u16,
        plot_size: PlotSize,
    ) -> Self {
        if let Some(preset) = catalog.default_preset_for_district(territory_type_id, plot_size) {
            return preset.exterior;
        }
        Self::default()
    }

    pub fn to_packet_exterior(&self) -> HouseExterior {
        let mut ext = HouseExterior {
            roof_id: self.roof_id,
            walls_id: self.walls_id,
            windows_id: self.windows_id,
            door_id: self.door_id,
            roof_fixture_id: self.roof_fixture_id,
            wall_fixture_id: self.wall_fixture_id,
            above_door_banner_id: self.above_door_banner_id,
            fence_id: self.fence_id,
            ..Default::default()
        };
        ext.colors.roof = self.roof_stain;
        ext.colors.walls = self.walls_stain;
        ext.colors.windows = self.windows_stain;
        ext.colors.door = self.door_stain;
        ext.colors.roof_fixture = self.roof_fixture_stain;
        ext.colors.wall_fixture = self.wall_fixture_stain;
        ext.colors.above_door_banner = self.above_door_banner_stain;
        ext.colors.fence = self.fence_stain;
        ext
    }

    pub fn to_live_update_packet(
        &self,
        estate: &HousingEstateRow,
    ) -> HousingExteriorAppearanceUpdate {
        let mut flag_bits = HousingFlag::OPEN;
        if estate.flags & (HousingFlag::OWNED_BY_FC.bits() as i32) != 0 {
            flag_bits |= HousingFlag::OWNED_BY_FC;
        }
        if (estate.flags & 0x01) == 0 {
            flag_bits.remove(HousingFlag::OPEN);
        }

        let ext = self.to_packet_exterior();
        HousingExteriorAppearanceUpdate {
            plot_index: (estate.plot_index.clamp(0, 29)) as u8,
            pad0: [0; 7],
            size_or_type: estate.plot_size.clamp(0, 2) as u8,
            status: plot_state_for_row(estate, &ext),
            flags: flag_bits,
            pad1: [0; 5],
            fc_or_owner_data: if estate.fc_id > 0 {
                estate.fc_id as u64
            } else {
                estate.owner_content_id as u64
            },
            parts: [
                self.roof_id,
                self.walls_id,
                self.windows_id,
                self.door_id,
                self.roof_fixture_id,
                self.wall_fixture_id,
                self.above_door_banner_id,
                self.fence_id,
            ],
            stains: [
                self.roof_stain,
                self.walls_stain,
                self.windows_stain,
                self.door_stain,
                self.roof_fixture_stain,
                self.wall_fixture_stain,
                self.above_door_banner_stain,
                self.fence_stain,
            ],
        }
    }

    pub fn part_ids(&self) -> [u16; 8] {
        [
            self.roof_id,
            self.walls_id,
            self.windows_id,
            self.door_id,
            self.roof_fixture_id,
            self.wall_fixture_id,
            self.above_door_banner_id,
            self.fence_id,
        ]
    }

    pub fn set_part_id(&mut self, slot: usize, id: u16) -> bool {
        match slot {
            0 => self.roof_id = id,
            1 => self.walls_id = id,
            2 => self.windows_id = id,
            3 => self.door_id = id,
            4 => self.roof_fixture_id = id,
            5 => self.wall_fixture_id = id,
            6 => self.above_door_banner_id = id,
            7 => self.fence_id = id,
            _ => return false,
        }
        true
    }

    pub fn set_stain(&mut self, slot: usize, stain: u8) -> bool {
        match slot {
            0 => self.roof_stain = stain,
            1 => self.walls_stain = stain,
            2 => self.windows_stain = stain,
            3 => self.door_stain = stain,
            4 => self.roof_fixture_stain = stain,
            5 => self.wall_fixture_stain = stain,
            6 => self.above_door_banner_stain = stain,
            7 => self.fence_stain = stain,
            _ => return false,
        }
        true
    }

    pub fn set_all_stains(&mut self, stain: u8) {
        self.roof_stain = stain;
        self.walls_stain = stain;
        self.windows_stain = stain;
        self.door_stain = stain;
        self.roof_fixture_stain = stain;
        self.wall_fixture_stain = stain;
        self.above_door_banner_stain = stain;
        self.fence_stain = stain;
    }

    pub fn apply_united_parts(&mut self, parts: [u16; 8]) {
        self.roof_id = parts[0];
        self.walls_id = parts[1];
        self.windows_id = parts[2];
        self.door_id = parts[3];
        self.roof_fixture_id = parts[4];
        self.wall_fixture_id = parts[5];
        self.above_door_banner_id = parts[6];
        self.fence_id = parts[7];
    }
}

/// Stored JSON shape for `housing_estates.interior_json`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct StoredHouseInterior {
    pub renovation_row_id: u16,
    pub window_style: u16,
    pub window_stain: u8,
    pub door_style: u16,
    pub door_stain: u8,
    pub ground_walls: u32,
    pub ground_floor: u32,
    pub ground_chandelier: u32,
    pub top_walls: u32,
    pub top_floor: u32,
    pub top_chandelier: u32,
    pub cellar_walls: u32,
    pub cellar_floor: u32,
    pub cellar_chandelier: u32,
    pub grand_chandelier: u32,
}

impl Default for StoredHouseInterior {
    fn default() -> Self {
        Self {
            renovation_row_id: 0,
            window_style: 0,
            window_stain: 0,
            door_style: 0,
            door_stain: 0,
            ground_walls: DEFAULT_MINIMAL_WALL,
            ground_floor: DEFAULT_MINIMAL_FLOOR,
            ground_chandelier: DEFAULT_MINIMAL_LIGHT,
            top_walls: DEFAULT_MINIMAL_WALL,
            top_floor: DEFAULT_MINIMAL_FLOOR,
            top_chandelier: DEFAULT_MINIMAL_LIGHT,
            cellar_walls: DEFAULT_MINIMAL_WALL,
            cellar_floor: DEFAULT_MINIMAL_FLOOR,
            cellar_chandelier: DEFAULT_MINIMAL_LIGHT,
            grand_chandelier: 0,
        }
    }
}

impl StoredHouseInterior {
    pub fn from_json(raw: &str) -> Self {
        serde_json::from_str(raw).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn default_for_location(
        catalog: &HousingCatalog,
        territory_type_id: u16,
        plot_size: PlotSize,
        is_apartment: bool,
    ) -> Self {
        let mut out = if !is_apartment
            && let Some(preset) = catalog.default_preset_for_district(territory_type_id, plot_size)
        {
            preset.interior
        } else {
            Self::default()
        };

        if out.renovation_row_id == 0 && !is_apartment {
            out.renovation_row_id =
                super::gamedata_ext::default_renovation_row_for_district_and_size(
                    territory_type_id,
                    plot_size,
                );
        }
        out
    }

    /// Converts stored interior state into the Patch `7.58` 56-byte `HousingInteriorDetails` packet.
    /// Zeroes upper/basement/grand chandelier fields for apartments to avoid destabilizing the client.
    pub fn to_packet(
        &self,
        light_level: u8,
        enable_ssao: bool,
        is_apartment: bool,
    ) -> HousingInteriorDetails {
        let mut details = HousingInteriorDetails {
            window_style: self.window_style,
            exterior_window_dye: self.window_stain as u16,
            door_style: self.door_style,
            door_stain: self.door_stain,
            light_level: light_level.min(5),
            enable_ssao,
            ground_walls: self.ground_walls,
            ground_floor: self.ground_floor,
            ground_chandelier: self.ground_chandelier,
            top_walls: self.top_walls,
            top_floor: self.top_floor,
            top_chandelier: self.top_chandelier,
            cellar_walls: self.cellar_walls,
            cellar_floor: self.cellar_floor,
            cellar_chandelier: self.cellar_chandelier,
            grand_chandelier: self.grand_chandelier,
        };

        if is_apartment {
            details.window_style = 0;
            details.set_window_stain(0);
            details.door_style = 0;
            details.door_stain = 0;
            details.top_walls = 0;
            details.top_floor = 0;
            details.top_chandelier = 0;
            details.cellar_walls = 0;
            details.cellar_floor = 0;
            details.cellar_chandelier = 0;
            details.grand_chandelier = 0;
        }
        details
    }

    /// Returns all interior fixture `Item.AdditionalData` values mapped to staging container `25002` slots `0..9`.
    pub fn appearance_values(&self) -> [u32; 10] {
        [
            self.ground_walls,
            self.ground_floor,
            self.ground_chandelier,
            self.top_walls,
            self.top_floor,
            self.top_chandelier,
            self.cellar_walls,
            self.cellar_floor,
            self.cellar_chandelier,
            self.grand_chandelier,
        ]
    }

    pub fn set_slot_by_index(&mut self, idx: usize, additional_data: u32) -> bool {
        match idx {
            0 => self.ground_walls = additional_data,
            1 => self.ground_floor = additional_data,
            2 => self.ground_chandelier = additional_data,
            3 => self.top_walls = additional_data,
            4 => self.top_floor = additional_data,
            5 => self.top_chandelier = additional_data,
            6 => self.cellar_walls = additional_data,
            7 => self.cellar_floor = additional_data,
            8 => self.cellar_chandelier = additional_data,
            9 => self.grand_chandelier = additional_data,
            _ => return false,
        }
        true
    }
}

/// Resolves interior remodel packet `InteriorRemodelItemOperation` (`C>S 164`) against a resolver closure `(container_id, slot) -> Option<(ItemId, AdditionalData)>`.
pub fn apply_interior_remodel_op<F>(
    interior: &mut StoredHouseInterior,
    op: &InteriorRemodelItemOperation,
    mut resolve_item: F,
) -> bool
where
    F: FnMut(u16, u16) -> Option<u32>,
{
    let mut changed = false;
    for idx in 0..10 {
        let src_c = op.source_containers[idx];
        let src_s = op.source_slots[idx];
        if src_c == 9999 || src_s == 0xFFFF {
            continue;
        }
        if let Some(additional_data) = resolve_item(src_c, src_s)
            && additional_data != 0
            && interior.set_slot_by_index(idx, additional_data)
        {
            changed = true;
        }
    }
    changed
}

/// Resolves exterior remodel packet `ExteriorRemodelItemOperation` (`C>S 349`).
pub fn apply_exterior_remodel_op<F>(
    exterior: &mut StoredHouseExterior,
    catalog: &HousingCatalog,
    op: &ExteriorRemodelItemOperation,
    mut resolve_item_id: F,
) -> bool
where
    F: FnMut(u16, u16) -> Option<u32>,
{
    let mut changed = false;
    for idx in 0..9 {
        let src_c = op.source_containers[idx];
        let src_s = op.source_slots[idx];
        if src_c == 9999 || src_s == 0xFFFF {
            continue;
        }
        let Some(item_id) = resolve_item_id(src_c, src_s) else {
            continue;
        };
        if idx == 8 || catalog.united_parts_for_item(item_id).is_some() {
            if let Some(parts) = catalog.united_parts_for_item(item_id) {
                exterior.apply_united_parts(parts);
                changed = true;
                break;
            }
        } else if let Some(ext_part_id) = catalog.exterior_part_id_for_item(item_id)
            && exterior.set_part_id(idx, ext_part_id)
        {
            changed = true;
        }
    }
    changed
}

/// Builds a `House` entry for `HouseList` from an estate DB row.
pub fn build_house_list_entry(row: &HousingEstateRow) -> House {
    let stored_ext = StoredHouseExterior::from_json(&row.exterior_json);
    let packet_ext = stored_ext.to_packet_exterior();
    let status = plot_state_for_row(row, &packet_ext);

    let mut flags = HousingFlag::empty();
    if (row.flags & (HousingFlag::OPEN.bits() as i32)) != 0 {
        flags |= HousingFlag::OPEN;
    }
    if (row.flags & (HousingFlag::OWNED_BY_FC.bits() as i32)) != 0 {
        flags |= HousingFlag::OWNED_BY_FC;
    }

    House {
        plot_size: PlotSize::from_repr(row.plot_size.clamp(0, 2) as u8).unwrap_or_default(),
        status,
        flags,
        fc_id: row.fc_id.max(0) as u32,
        fc_crest_id: 0,
        fc_crest_id1: 0,
        exterior: packet_ext,
    }
}

/// Builds the full `OwnedHousing` tuple `(owned_array, apartment)` from character-owned estate rows.
pub fn build_owned_housing_data(
    estates: &[HousingEstateRow],
) -> ([kawari::common::LandData; 5], kawari::common::LandData) {
    let mut owned = [kawari::common::LandData::default(); 5];
    let mut apartment = kawari::common::LandData::default();

    for estate in estates {
        let ld = kawari::common::LandData {
            id: HouseId::from(estate.house_id as u64),
            flags: estate.flags as u32,
            unk1: 0,
        };
        if estate.is_apartment {
            apartment = ld;
        } else if estate.flags & (HousingFlag::OWNED_BY_FC.bits() as i32) != 0 {
            owned[0] = ld;
        } else if owned[1].id == HouseId::default() {
            owned[1] = ld;
        }
    }
    (owned, apartment)
}

/// Builds `HousingFlags` for `HousingWardSummaryItem`.
pub fn ward_summary_flags_for_estate(row: &HousingEstateRow) -> HousingFlags {
    let stored_ext = StoredHouseExterior::from_json(&row.exterior_json);
    let packet_ext = stored_ext.to_packet_exterior();
    let mut f = HousingFlags::PLOT_OWNED;
    if row.flags & (HousingFlag::OPEN.bits() as i32) != 0 {
        f |= HousingFlags::VISITORS_ALLOWED;
    }
    if !row.greeting.is_empty() {
        f |= HousingFlags::HAS_SEARCH_COMMENT;
    }
    if !packet_ext.is_under_construction() {
        f |= HousingFlags::HOUSE_BUILT;
    }
    if row.flags & (HousingFlag::OWNED_BY_FC.bits() as i32) != 0 {
        f |= HousingFlags::OWNED_BY_FC;
    }
    f
}

/// Builds `[HousingAppealTag; 3]` from stored tag integers.
pub fn appeal_tags_from_row(row: &HousingEstateRow) -> [HousingAppealTag; 3] {
    [
        HousingAppealTag::from_repr(row.appeal_tag_0.clamp(0, 255) as usize).unwrap_or_default(),
        HousingAppealTag::from_repr(row.appeal_tag_1.clamp(0, 255) as usize).unwrap_or_default(),
        HousingAppealTag::from_repr(row.appeal_tag_2.clamp(0, 255) as usize).unwrap_or_default(),
    ]
}

/// Builds `ApartmentListEntry` rows for a requested `starting_index` (`1`-based UI offset).
pub fn build_apartment_entries(
    apartments: &[HousingEstateRow],
    starting_index: u32,
) -> Vec<ApartmentListEntry> {
    let skip = starting_index.saturating_sub(1) as usize;
    let mut sorted = apartments
        .iter()
        .filter(|e| e.is_apartment && e.room_number > 0)
        .collect::<Vec<_>>();
    sorted.sort_by_key(|e| (e.room_number, e.land_ident));

    sorted
        .into_iter()
        .skip(skip)
        .take(ApartmentListEntry::COUNT)
        .map(|estate| ApartmentListEntry {
            resident_zone_id: estate.territory_type_id.clamp(0, u16::MAX as i32) as u16,
            visitors_permitted: estate.flags & (HousingFlag::OPEN.bits() as i32) != 0,
            housing_appeal: appeal_tags_from_row(estate),
            resident_name: if estate.owner_name.is_empty() {
                "Resident".to_string()
            } else {
                estate.owner_name.clone()
            },
            apartment_description: if estate.greeting.is_empty() {
                "Foundation Housing Simulator Apartment".to_string()
            } else {
                estate.greeting.clone()
            },
            ..Default::default()
        })
        .collect()
}

/// Swaps the low and high 32-bit halves of a `HouseId` as expected by `ActorControlCategory::HousingInteriorReady`.
pub fn housing_interior_ready_primary_id(house_id: HouseId) -> u64 {
    let raw = u64::from(house_id);
    raw.rotate_right(32)
}

/// Builds chunked dense `FurnitureList` packets for an indoor estate (preserving flat slots `0..capacity`).
pub fn build_indoor_furniture_lists(
    house_id: HouseId,
    plot_size: PlotSize,
    is_apartment: bool,
    rows: &[HousingFurnitureRow],
) -> Vec<FurnitureList> {
    let estate_type = if is_apartment {
        kawari::common::EstateType::ApartmentRoom
    } else if house_id.unit.apartment_flag {
        kawari::common::EstateType::PrivateChambers
    } else {
        kawari::common::EstateType::PersonalEstate
    };

    let capacity = indoor_slot_capacity(plot_size, is_apartment).min(MAX_INDOOR_PLACED_SLOTS);
    let mut max_slot_used = 0usize;
    let mut dense = vec![Furniture::default(); capacity];

    for row in rows {
        let Some(container) = super::containers::container_from_i32(row.container_type) else {
            continue;
        };
        let Some(flat_slot) = flat_slot_for_placed_container(container, row.slot as u16) else {
            continue;
        };
        let idx = flat_slot as usize;
        if idx >= dense.len() {
            continue;
        }
        max_slot_used = max_slot_used.max(idx + 1);
        dense[idx] = Furniture {
            id: row.catalog_id.max(0) as u16,
            id2: (row.material_item_key.max(0) as u16) as i16,
            stain: row.stain.clamp(0, u8::MAX as i32) as u8,
            rotation: row.rotation,
            position: kawari::common::Position(glam::Vec3A::new(row.pos_x, row.pos_y, row.pos_z)),
        };
    }

    let represented_slots = capacity.max(max_slot_used);
    let chunks: Vec<Vec<Furniture>> = dense
        .chunks(Furniture::COUNT)
        .map(|chunk| chunk.to_vec())
        .collect();
    let count = chunks.len().max(1) as u8;

    let _ = represented_slots;
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| FurnitureList {
            id: house_id,
            estate_type,
            index: index as u8,
            count,
            unk2: chunk.len() as u8,
            furniture: chunk,
        })
        .collect()
}

/// Builds 1 outdoor `FurnitureList` (`unk2 = 0`, matching retail/Kawari ward init) for an estate plot.
pub fn build_outdoor_furniture_list(
    estate: &HousingEstateRow,
    rows: &[HousingFurnitureRow],
) -> FurnitureList {
    let mut house_id = HouseId::from(estate.house_id as u64);
    house_id.room_number = 0;
    let mut furniture = vec![Furniture::default(); Furniture::COUNT];

    for row in rows {
        let slot = row.slot as usize;
        if slot >= 50
            || row.container_type
                != kawari::common::ContainerType::HousingExteriorPlacedItems as i32
        {
            continue;
        }
        furniture[slot] = Furniture {
            id: row.catalog_id.max(0) as u16,
            id2: (row.material_item_key.max(0) as u16) as i16,
            stain: row.stain.clamp(0, u8::MAX as i32) as u8,
            rotation: row.rotation,
            position: kawari::common::Position(glam::Vec3A::new(row.pos_x, row.pos_y, row.pos_z)),
        };
    }

    FurnitureList {
        id: house_id,
        estate_type: kawari::common::EstateType::Ward,
        index: 0,
        count: 1,
        unk2: 0,
        furniture,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_indoor_furniture_lists_preserves_slots_beyond_300() {
        let house_id = super::super::db::make_house_id(340, 79, 0, 0, 5, 0, false);
        let row = HousingFurnitureRow {
            land_ident: 1,
            container_type: kawari::common::ContainerType::HousingInteriorPlacedItems12 as i32,
            slot: 42, // flat slot 592
            item_id: 28901,
            catalog_id: 777,
            material_item_key: 1500,
            stain: 9,
            placed: true,
            pos_x: 1.0,
            pos_y: 2.0,
            pos_z: 3.0,
            rotation: 0.5,
            ..Default::default()
        };
        let lists = build_indoor_furniture_lists(house_id, PlotSize::Large, false, &[row]);
        assert_eq!(lists.len(), 6);
        assert_eq!(lists[5].count, 6);
        assert_eq!(lists[5].unk2, 100);
        assert_eq!(lists[5].furniture[92].id, 777);
        assert_eq!(lists[5].furniture[92].material_item_key(), 1500);
        assert_eq!(lists[5].furniture[92].stain, 9);
    }
}
