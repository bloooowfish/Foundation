use binrw::binrw;

use crate::common::ContainerType;

#[binrw]
#[derive(Debug, Clone, Default)]
pub struct DyeInformation {
    #[brw(pad_size_to = 4)]
    pub target_container: ContainerType,
    pub target_slot: u32,
    pub dye1_container_raw: u32,
    pub dye2_container_or_slot_raw: u32,
    pub unk1: u32,
    pub dye1: u8,
    pub dye2: u8,
    pub unk2: u16,
}

impl DyeInformation {
    /// Extracts the two signed `stain_ids` at `+0x10` used by Patch `7.58` housing dye packets.
    pub fn housing_stain_pair(&self) -> [i16; 2] {
        let bytes = self.unk1.to_le_bytes();
        [
            i16::from_le_bytes([bytes[0], bytes[1]]),
            i16::from_le_bytes([bytes[2], bytes[3]]),
        ]
    }

    /// Returns the primary stain ID for housing dye operations.
    pub fn housing_stain(&self) -> u8 {
        let [s0, _] = self.housing_stain_pair();
        if s0 > 0 && s0 <= u8::MAX as i16 {
            s0 as u8
        } else {
            self.dye1
        }
    }

    /// Returns true when the packet originates from housing dye mode (`slot == 9999` or negative secondary stain).
    pub fn is_housing_dye_mode(&self) -> bool {
        self.dye2_container_or_slot_raw == 9999 || self.housing_stain_pair()[1] < 0
    }
}
