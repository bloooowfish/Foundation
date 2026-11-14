use crate::{
    common::{EstateType, HouseId, Position},
    ipc::zone::server::{read_bool_from, write_bool_as},
};
use binrw::binrw;
use serde::{Deserialize, Serialize};

#[binrw]
#[derive(Clone, Debug, Default)]
pub struct FurnitureList {
    /// The LandId this list is for.
    pub id: HouseId,
    /// The type of estate this furniture is for. This mainly seems to control visibility of the additional chambers door. Wards set it to 255.
    pub unk1: EstateType,
    /// The current `index` out of `count` packets to be sent.
    pub index: u8,
    /// The number of these lists that will be sent.
    pub count: u8,
    /// Seems to be some sort of outdoor vs indoor flag. If it's 0, it means this is for outdoor furniture, and 100 means it's for an interior.
    pub unk2: u8,
    /// The actual furnishings.
    #[br(count = Furniture::COUNT)]
    #[brw(pad_size_to = Furniture::COUNT * Furniture::SIZE)]
    #[brw(pad_after = 4)] // Seems to be empty/zeroes
    pub furniture: Vec<Furniture>,
}

// TODO: Figure out if `exterior_window_dye` needs to be serialized
#[binrw]
#[derive(Copy, Clone, Debug, Default, Deserialize, Serialize)]
pub struct HousingInteriorDetails {
    /// This interior's window style.
    pub window_style: u16,
    /// Seems to only be affected by all-in-one exterior sets. In interiors, it has no noticeable visual effect. Perhaps a deprecated or abandoned field.
    #[serde(default, skip_serializing)]
    pub exterior_window_dye: u16, // From worstaquaplayer: v10.ExteriorStains[1] = a2->field_2;
    /// This interior's door style.
    pub door_style: u16,
    /// This interior door's dye colour. Index into the Stain Excel sheet.
    pub door_stain: u8,
    /// The light level in the interior. Note that this is actually described in terms of a level of *darkness*. When the client sets the UI light level to 5, the client will send value 0 in the ClientTrigger. Other examples: light level 1 will send 4, light level 2 will send 3, and so on.
    pub light_level: u8,
    /// Enables SSAO for furnishings. 1 = enabled (unchecked), 0 = disabled (checked).
    #[br(map = read_bool_from::<u8>)]
    #[bw(map = write_bool_as::<u8>)]
    #[brw(pad_after = 3)] // Likely padding/empty, observed only as zeroes so far
    pub enable_ssao: bool,
    /// The ground floor's wall style. In an apartment, this along with ground_floor and ground_chandelier dictate what will decorate the apartment, leaving doors, windows, top floor and cellar all zeroes/blank..
    // NOTE: Be careful when experimenting with these values, as invalid combinations of u16s can crash the client, particularly if the interior is an apartment and top floor or cellar values are changed!
    pub ground_walls: u32,
    /// The ground floor's style/texture.
    pub ground_floor: u32,
    /// THe ground floor's chandelier. Unknown if this is a model id + toggle, or an item id + toggle.
    pub ground_chandelier: u32,
    /// The top floor's wall style/texture.
    pub top_walls: u32,
    /// The top floor's style/texture.
    pub top_floor: u32,
    /// The top floor's chandelier.
    pub top_chandelier: u32,
    /// The cellar's wall style/texture.
    pub cellar_walls: u32,
    /// The cellar's floor syle/texture.
    pub cellar_floor: u32,
    /// The cellar's chandelier.
    pub cellar_chandelier: u32,
    /// The mansion's grand chandelier ("options - ceiling light") in the foyer.
    #[brw(pad_after = 4)] // Empty, client doesn't read it
    pub grand_chandelier: u32,
}

#[binrw]
#[derive(Copy, Clone, Debug, Default, Deserialize, Serialize)]
pub struct Furniture {
    /// Index into the FurnitureCatalogItemList sheet. If 0, no item is present in this entry. Therefore, this index needs to subtract 1 when indexing into the sheet!
    pub id: u16,
    /// Unknown.
    pub id2: i16,
    /// Index into the Stain sheet. Sets the dye for this item.
    #[brw(pad_after = 3)] // Empty, not read by the client.
    pub stain: u8,
    /// This item's rotation.
    pub rotation: f32,
    /// This item's 3d coordinates in the housing interior.
    pub position: Position,
}

/// Data sent to a client that observes another client moving or rotating furniture.
#[binrw]
#[derive(Clone, Copy, Debug, Default)]
pub struct FurnitureTranslatedForObserver {
    /// This furniture's new rotation.
    pub rotation: f32,
    /// NOTE: The purpose of `plot_and_index` changes depending on whether this operation is happening indoors or outdoors. When indoors, this value stays as a u16 so that furniture can be addressed beyond an index of 255. When outdoors, it's treated as two separate values: `outdoor_index`, and `plot_number`. See their comments below for more info.
    pub plot_and_index: u16,
    /// When outdoors, this byte represents the affected index into the outdoor furniture. This exists mainly for PacketAnalyzer display.
    #[br(calc = (plot_and_index & 0xFF) as u8)]
    #[bw(ignore)]
    pub outdoor_index: u8,
    /// When outdoors, this byte represents which plot the furniture was moved on. This exists mainly for PacketAnalyzer display.
    #[br(calc = (plot_and_index >> 8) as u8)]
    #[bw(ignore)]
    pub plot_number: u8,
    pub unk1: [u8; 2], // Likely just padding, observed as zeroes
    /// This furniture's new position in the world.
    pub position: Position,
    pub unk2: [u8; 4], // Likely just padding, observed as zeroes
}

impl Furniture {
    pub const SIZE: usize = 24;
    pub const COUNT: usize = 100;
}
