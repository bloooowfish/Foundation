use binrw::binrw;

use crate::common::ContainerType;

#[binrw]
#[derive(Debug, Clone, Default)]
pub struct DyeInformation {
    #[brw(pad_size_to = 4)]
    pub target_container: ContainerType,
    pub target_slot: u16,
    pub target_slot_pad: u16,
    pub dye1_container_raw: u32,
    pub dye2_container_or_slot_raw: u32,
    pub dye1_slot: i16,
    pub dye2_slot: i16,
    pub dye1: u8,
    pub dye2: u8,
    pub unk2: u16,
}
