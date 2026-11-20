use binrw::binrw;
use serde_derive::{Deserialize, Serialize};

use crate::common::{ContainerType, HouseId, HousingFlag, PlotState, Position};

// -----------------------------------------------------------------------------
// Server -> Client (S>C) Housing Extension Packets (Patch 7.58 IDA-verified)
// -----------------------------------------------------------------------------

/// Per-furniture object data values (flowerpots, mannequins/display stands, aquariums).
#[binrw]
#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HousingObjectDataValueSet {
    pub furniture_index: u16,
    pub value_count: u8,
    pub reserved: u8,
    pub values: [u16; 8],
    pub param_a: [u8; 8],
    pub param_b: [u8; 8],
    pub param_c: [u8; 8],
    pub padding: [u8; 4],
}

/// Live stain update for indoor furniture, yard furniture, or exterior house parts (no reload).
#[binrw]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct HousingStainUpdated {
    pub stain: u8,
    pub pad0: u8,
    pub container_id: ContainerType,
    pub slot: u8,
    pub pad1: [u8; 3],
    pub plot_index: u8,
    pub pad2: u8,
    /// When `container_id == HousingExteriorAppearance` (25000) and non-zero, applies `stain` to all 8 exterior parts.
    pub apply_all_exterior_parts: u16,
    pub pad3: u32,
}

/// Live `VaseFlower` material update for paintings, blank partitions, canvases, and flower vases.
#[binrw]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct IndoorFurnitureMaterialUpdated {
    pub material_item_key: u16,
    pub container_id: ContainerType,
    pub slot: u8,
    pub pad0: [u8; 3],
}

/// Live single-plot 8-part exterior fixture and stain update without reloading the ward.
#[binrw]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct HousingExteriorAppearanceUpdate {
    pub plot_index: u8,
    pub pad0: [u8; 7],
    pub size_or_type: u8,
    pub status: PlotState,
    pub flags: HousingFlag,
    pub pad1: [u8; 5],
    pub fc_or_owner_data: u64,
    pub parts: [u16; 8],
    pub stains: [u8; 8],
}

/// Single entry inside [`BatchIndoorFurnitureTranslated`]. `slot == 0xFFFF` marks unused entries.
#[binrw]
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BatchTranslatedFurnitureEntry {
    pub slot: u16,
    pub pad0: u16,
    pub rotation: f32,
    pub position: Position,
}

impl Default for BatchTranslatedFurnitureEntry {
    fn default() -> Self {
        Self {
            slot: 0xFFFF,
            pad0: 0,
            rotation: 0.0,
            position: Position::default(),
        }
    }
}

/// Live batch move/rotate of up to 10 indoor furnishings with zero zone reload.
#[binrw]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct BatchIndoorFurnitureTranslated {
    pub house_id: HouseId,
    pub entries: [BatchTranslatedFurnitureEntry; 10],
}

/// Live batch slot removal/update of up to 10 indoor furnishings (`0xFFFF` = unused).
#[binrw]
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct BatchIndoorFurnitureSlotUpdate {
    pub house_id: HouseId,
    pub mode: u8,
    pub pad0: u8,
    pub slots: [u16; 10],
    pub pad1: u16,
}

impl Default for BatchIndoorFurnitureSlotUpdate {
    fn default() -> Self {
        Self {
            house_id: HouseId::default(),
            mode: 0,
            pad0: 0,
            slots: [0xFFFF; 10],
            pad1: 0,
        }
    }
}

// -----------------------------------------------------------------------------
// Client -> Server (C>S) Housing Extension Packets (Patch 7.58 IDA-verified)
// -----------------------------------------------------------------------------

/// Interior remodel commit from staging container `2012` (10 fixture slots).
#[binrw]
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct InteriorRemodelItemOperation {
    pub reserved: u64,
    pub source_containers: [u16; 10],
    pub source_slots: [u16; 10],
}

impl Default for InteriorRemodelItemOperation {
    fn default() -> Self {
        Self {
            reserved: 0,
            source_containers: [9999; 10],
            source_slots: [0xFFFF; 10],
        }
    }
}

/// Exterior remodel commit from staging container `2011` (9 fixture slots).
#[binrw]
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ExteriorRemodelItemOperation {
    pub house_id: HouseId,
    pub flag: u8,
    pub pad0: u8,
    pub source_containers: [u16; 9],
    pub source_slots: [u16; 9],
    pub pad1: u16,
}

impl Default for ExteriorRemodelItemOperation {
    fn default() -> Self {
        Self {
            house_id: HouseId::default(),
            flag: 0,
            pad0: 0,
            source_containers: [9999; 9],
            source_slots: [0xFFFF; 9],
            pad1: 0,
        }
    }
}

/// Simultaneous move/rotate request for up to 10 indoor furnishings.
#[binrw]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct BatchTranslateIndoorFurniture {
    pub house_id: HouseId,
    pub pos_x: [f32; 10],
    pub pos_y: [f32; 10],
    pub pos_z: [f32; 10],
    pub rotation: [f32; 10],
    pub slots: [u16; 10],
    pub pad0: u32,
}

/// Client updates the active estate's name (`23` UTF-8 bytes + padding).
#[binrw]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct UpdateEstateName {
    pub house_id: HouseId,
    pub name_bytes: [u8; 23],
    pub pad0: u8,
}

/// Client updates the active estate's greeting (`193` UTF-8 bytes + `7` pad).
#[binrw]
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateEstateGreeting {
    pub house_id: HouseId,
    pub greeting_bytes: [u8; 193],
    pub pad0: [u8; 7],
}

impl Default for UpdateEstateGreeting {
    fn default() -> Self {
        Self {
            house_id: HouseId::default(),
            greeting_bytes: [0u8; 193],
            pad0: [0u8; 7],
        }
    }
}

/// Client updates estate appeal tags & guest access flags.
#[binrw]
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateEstateProfile {
    pub house_id: HouseId,
    pub guest_access_flags: u8,
    pub pad0: [u8; 7],
    pub profile_bytes: [u8; 40],
}

impl Default for UpdateEstateProfile {
    fn default() -> Self {
        Self {
            house_id: HouseId::default(),
            guest_access_flags: 0,
            pad0: [0u8; 7],
            profile_bytes: [0u8; 40],
        }
    }
}

/// Client builds a house from an estate construction permit item.
#[binrw]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct BuildEstateDeed {
    pub item_id: u32,
    pub plot_index: u8,
    pub deed_name_bytes: [u8; 23],
    pub pad0: u32,
}

/// Client updates an aquarium's trim, water, and fish slots.
#[binrw]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct UpdateAquarium {
    pub furniture_slot: u16,
    pub water_type: u8,
    pub pad0: u8,
    pub trimming_container: u32,
    pub trimming_slot: u16,
    pub pad1: u16,
    pub fish_containers: [u32; 4],
    pub fish_slots: [u16; 4],
    pub pad2: u32,
}

/// Client updates orchestrion playback or playlist.
#[binrw]
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct UpdateOrchestrionPlaylist {
    pub raw: [u8; 16],
}

impl super::DyeInformation {
    /// Returns the primary stain ID for housing dye operations.
    pub fn housing_stain(&self) -> u8 {
        self.dye1
    }

    /// Returns true when the packet originates from housing dye mode (`9999` or negative secondary slot).
    pub fn is_housing_dye_mode(&self) -> bool {
        self.dye2_container_or_slot_raw == 9999 || self.dye2_slot < 0
    }
}

impl super::HousingInteriorDetails {
    pub const fn window_stain(&self) -> u8 {
        (self.exterior_window_dye & 0xFF) as u8
    }

    pub fn set_window_stain(&mut self, stain: u8) {
        self.exterior_window_dye = stain as u16;
    }
}

impl super::Furniture {
    pub const fn material_item_key(&self) -> u16 {
        self.id2 as u16
    }

    pub fn set_material_item_key(&mut self, key: u16) {
        self.id2 = key as i16;
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use binrw::{BinRead, BinWrite};

    use super::*;
    use crate::ipc::zone::DyeInformation;

    #[test]
    fn dye_information_758_layout_roundtrips_24_bytes() {
        let info = DyeInformation {
            target_container: ContainerType::HousingInteriorPlacedItems1,
            target_slot: 12,
            target_slot_pad: 0,
            dye1_container_raw: 0,
            dye2_container_or_slot_raw: 9999,
            dye1_slot: 5,
            dye2_slot: -1,
            dye1: 42,
            dye2: 0,
            unk2: 0,
        };
        let mut buf = Cursor::new(Vec::new());
        info.write_le(&mut buf)
            .expect("DyeInformation should serialize");
        let bytes = buf.into_inner();
        assert_eq!(bytes.len(), 24);

        let decoded =
            DyeInformation::read_le(&mut Cursor::new(&bytes)).expect("DyeInformation should parse");
        assert_eq!(
            decoded.target_container,
            ContainerType::HousingInteriorPlacedItems1
        );
        assert_eq!(decoded.target_slot, 12);
        assert_eq!(decoded.dye1_slot, 5);
        assert_eq!(decoded.dye2_slot, -1);
        assert_eq!(decoded.housing_stain(), 42);
        assert!(decoded.is_housing_dye_mode());
    }
}
