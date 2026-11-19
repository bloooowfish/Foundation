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

impl DyeInformation {
    /// Returns the primary stain ID for housing dye operations.
    pub fn housing_stain(&self) -> u8 {
        self.dye1
    }

    /// Returns true when the packet originates from housing dye mode (`9999` or negative secondary slot).
    pub fn is_housing_dye_mode(&self) -> bool {
        self.dye2_container_or_slot_raw == 9999 || self.dye2_slot < 0
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use binrw::{BinRead, BinWrite};

    use super::*;

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
