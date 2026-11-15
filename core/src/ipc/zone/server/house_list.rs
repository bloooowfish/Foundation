use crate::{
    common::{HousingFlag, PlotState},
    ipc::zone::{PlotSize, WardDivisionInfo},
};
use binrw::binrw;

#[cfg(feature = "server")]
use serde_derive::{Deserialize, Serialize};

#[binrw]
#[derive(Clone, Copy, Debug, Default)]
pub struct House {
    pub plot_size: PlotSize,
    pub status: PlotState,
    #[brw(pad_after = 1)]
    pub flags: HousingFlag,
    pub fc_id: u32,
    pub fc_crest_id: u32,
    pub fc_crest_id1: u32,
    pub exterior: HouseExterior,
}

/// Represents a House's "pattern ids", or in other words, what models make up the house's exterior. All values are an index into the HousingExterior Excel sheet.
#[binrw]
#[cfg_attr(feature = "server", derive(Deserialize, Serialize))]
#[derive(Clone, Copy, Debug, Default)]
pub struct HouseExterior {
    /// The roof's style.
    pub roof_id: u16,
    /// The walls' style.
    pub walls_id: u16,
    /// The windows' style.
    pub windows_id: u16,
    // The front door's style.
    pub door_id: u16,
    /// The roof's fixture, like a chimney.
    pub roof_fixture_id: u16,
    /// Exterior wall fixture, like an awning.
    pub wall_fixture_id: u16,
    /// The flag/banner/crest that resides above the front door.
    pub above_door_banner_id: u16,
    /// The perimeter fence's style.
    pub fence_id: u16,
    /// All of the above's dyes/stains info.
    pub colors: HouseExteriorColors,
}

impl HouseExterior {
    // TODO: There might be a better way to do this?
    pub fn is_under_construction(&self) -> bool {
        self.roof_id == 0 || self.walls_id == 0 || self.windows_id == 0 || self.door_id == 0
    }
}

/// Represents a HouseExterior's dyes/stains.
#[binrw]
#[cfg_attr(feature = "server", derive(Deserialize, Serialize))]
#[derive(Clone, Copy, Debug, Default)]
pub struct HouseExteriorColors {
    /// The roof.
    pub roof: u8,
    /// The walls.
    pub walls: u8,
    /// The windows.
    pub windows: u8,
    // The front door.
    pub door: u8,
    /// The roof's fixture.
    pub roof_fixture: u8,
    /// Exterior wall fixture.
    pub wall_fixture: u8,
    /// The flag/banner/crest that resides above the front door.
    pub above_door_banner: u8,
    /// The perimeter fence.
    pub fence: u8,
}

#[binrw]
#[derive(Clone, Debug, Default)]
pub struct HouseList {
    pub land_id: u16,
    pub ward: u16,
    pub territory_type_id: u16,
    pub world_id: u16,

    #[brw(pad_after = 6)] // Empty
    pub division_info: WardDivisionInfo,

    pub houses: [House; 30],
}
