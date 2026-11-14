use crate::{
    common::{HouseId, PlotState, read_string, write_string},
    ipc::zone::{HousingAppealTag, HousingFlags, PlotSize, PurchaseType, TenantType},
};
use binrw::binrw;
use bitflags::bitflags;

#[binrw]
#[derive(Clone, Debug, Default)]
pub struct HousingWardInfo {
    pub id: HouseId,

    #[br(count = 60)]
    #[brw(pad_size_to = 60 * HousingWardSummaryItem::SIZE)]
    pub house_summaries: Vec<HousingWardSummaryItem>,

    pub purchase_type: PurchaseType,
    pub unk1: u8,
    pub tenant_type: TenantType,

    #[brw(pad_after = 1)]
    pub terminator: u32,
}

#[binrw]
#[derive(Clone, Debug, Default)]
pub struct HousingWardSummaryItem {
    pub plot_price: u32,
    pub flags: HousingFlags,
    pub tags: [HousingAppealTag; 3],
    #[brw(pad_size_to = 32)]
    #[br(count = 32)]
    #[br(map = read_string)]
    #[bw(map = write_string)]
    pub name: String,
}

impl HousingWardSummaryItem {
    pub const SIZE: usize = 40;
}

#[binrw]
#[derive(Clone, Copy, Debug, Default)]
pub struct WardMapInfoFlags(u8);

bitflags! {
    impl WardMapInfoFlags: u8 {
        /// This plot is owned (won't display the auction icon).
        const HAS_OWNER = 1;
        /// This plot belongs to a free company.
        const IS_FREE_COMPANY = 2;
    }
}

/// The actual marker/icon data an individual plot.
#[binrw]
#[derive(Clone, Copy, Debug, Default)]
pub struct WardMapInfo {
    pub state: PlotState,
    pub size: PlotSize,
    pub flags: WardMapInfoFlags,
}

/// The current state of the apartment building.
/// See <https://github.com/aers/FFXIVClientStructs/blob/main/FFXIVClientStructs/FFXIV/Client/Game/Housing/OutdoorTerritory.cs>
#[binrw]
#[brw(repr = u8)]
#[derive(Clone, Copy, Debug, Default)]
pub enum ApartmentBuildingState {
    #[default]
    None = 0,
    /// The apartment building has no more units for purchase.
    IsFull = 1,
    /// The apartment building has the player's apartment.
    IsOwn = 2,
}

/// Which division the markers are updating.
#[binrw]
#[brw(repr = u8)]
#[derive(Clone, Copy, Debug, Default)]
pub enum WardDivision {
    #[default]
    MainDivision = 1,
    SubDivision = 2,
}

#[binrw]
#[derive(Clone, Copy, Debug, Default)]
pub struct WardDivisionInfo {
    pub apartment_building_state: ApartmentBuildingState,
    pub ward_division: WardDivision,
}

#[binrw]
#[derive(Clone, Copy, Debug, Default)]
pub struct UpdateMapMarkersForWardDivision {
    #[brw(pad_after = 1)]
    pub ward_division_info: WardDivisionInfo,
    #[brw(pad_after = 3)]
    pub info: [WardMapInfo; 30],
}

#[cfg(test)]
mod tests {
    use crate::common::ensure_size;

    use super::*;

    #[test]
    fn housing_ward_size() {
        ensure_size::<HousingWardSummaryItem, { HousingWardSummaryItem::SIZE }>();
    }
}
