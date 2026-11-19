use kawari::common::{ContainerType, TerritoryIntendedUse};
use kawari::ipc::zone::PlotSize;

pub const SLOTS_PER_HOUSING_PAGE: u16 = 50;
pub const SLOTS_PER_EXTERIOR_PAGE: u16 = 40;
pub const MAX_INDOOR_PLACED_PAGES: usize = 12;
pub const MAX_INDOOR_STOREROOM_PAGES: usize = 12;
pub const MAX_EXTERIOR_PLACED_PAGES: usize = 2;
pub const MAX_EXTERIOR_STOREROOM_PAGES: usize = 2;
pub const MAX_INDOOR_PLACED_SLOTS: usize =
    MAX_INDOOR_PLACED_PAGES * SLOTS_PER_HOUSING_PAGE as usize;
pub const MAX_EXTERIOR_PLACED_SLOTS: usize =
    MAX_EXTERIOR_PLACED_PAGES * SLOTS_PER_EXTERIOR_PAGE as usize;
pub const MAX_EXTERIOR_STOREROOM_SLOTS: usize =
    MAX_EXTERIOR_STOREROOM_PAGES * SLOTS_PER_EXTERIOR_PAGE as usize;

pub const EXTERIOR_PLACED_CONTAINERS: [ContainerType; MAX_EXTERIOR_PLACED_PAGES] = [
    ContainerType::HousingExteriorPlacedItems,
    ContainerType::HousingExteriorPlacedItems2,
];

pub const EXTERIOR_STOREROOM_CONTAINERS: [ContainerType; MAX_EXTERIOR_STOREROOM_PAGES] = [
    ContainerType::HousingExteriorStoreroom,
    ContainerType::HousingExteriorStoreroom2,
];

pub const INTERIOR_PLACED_CONTAINERS: [ContainerType; MAX_INDOOR_PLACED_PAGES] = [
    ContainerType::HousingInteriorPlacedItems1,
    ContainerType::HousingInteriorPlacedItems2,
    ContainerType::HousingInteriorPlacedItems3,
    ContainerType::HousingInteriorPlacedItems4,
    ContainerType::HousingInteriorPlacedItems5,
    ContainerType::HousingInteriorPlacedItems6,
    ContainerType::HousingInteriorPlacedItems7,
    ContainerType::HousingInteriorPlacedItems8,
    ContainerType::HousingInteriorPlacedItems9,
    ContainerType::HousingInteriorPlacedItems10,
    ContainerType::HousingInteriorPlacedItems11,
    ContainerType::HousingInteriorPlacedItems12,
];

pub const INTERIOR_STOREROOM_CONTAINERS: [ContainerType; MAX_INDOOR_STOREROOM_PAGES] = [
    ContainerType::HousingInteriorStoreroom1,
    ContainerType::HousingInteriorStoreroom2,
    ContainerType::HousingInteriorStoreroom3,
    ContainerType::HousingInteriorStoreroom4,
    ContainerType::HousingInteriorStoreroom5,
    ContainerType::HousingInteriorStoreroom6,
    ContainerType::HousingInteriorStoreroom7,
    ContainerType::HousingInteriorStoreroom8,
    ContainerType::HousingInteriorStoreroom9,
    ContainerType::HousingInteriorStoreroom10,
    ContainerType::HousingInteriorStoreroom11,
    ContainerType::HousingInteriorStoreroom12,
];

/// Returns the placed capacity in flat slots for the given plot size and apartment flag.
pub fn indoor_slot_capacity(plot_size: PlotSize, is_apartment: bool) -> usize {
    if is_apartment {
        return 100;
    }
    match plot_size {
        PlotSize::Small => 300,
        PlotSize::Medium => 450,
        PlotSize::Large => MAX_INDOOR_PLACED_SLOTS,
    }
}

/// Returns the outdoor placed capacity for the given plot size.
pub fn outdoor_slot_capacity(plot_size: PlotSize) -> usize {
    match plot_size {
        PlotSize::Small => 40,
        PlotSize::Medium => 60,
        PlotSize::Large => MAX_EXTERIOR_PLACED_SLOTS,
    }
}

/// Converts a flat indoor placed index (`0..600`) to `(ContainerType, page_slot 0..50)`.
pub fn indoor_container_for_flat_slot(flat_slot: u16) -> Option<(ContainerType, u16)> {
    let page = (flat_slot / SLOTS_PER_HOUSING_PAGE) as usize;
    let slot = flat_slot % SLOTS_PER_HOUSING_PAGE;
    INTERIOR_PLACED_CONTAINERS
        .get(page)
        .copied()
        .map(|container| (container, slot))
}

/// Converts a `(ContainerType, page_slot)` to a flat index (`0..600` indoors, `0..80` outdoors).
pub fn flat_slot_for_placed_container(container: ContainerType, slot: u16) -> Option<u16> {
    if let Some(page) = EXTERIOR_PLACED_CONTAINERS
        .iter()
        .position(|&c| c == container)
    {
        return (slot < SLOTS_PER_EXTERIOR_PAGE)
            .then_some((page as u16) * SLOTS_PER_EXTERIOR_PAGE + slot);
    }
    if slot >= SLOTS_PER_HOUSING_PAGE {
        return None;
    }
    INTERIOR_PLACED_CONTAINERS
        .iter()
        .position(|&c| c == container)
        .map(|page| (page as u16) * SLOTS_PER_HOUSING_PAGE + slot)
}

/// Resolves an indoor or outdoor placed `(ContainerType, page_slot)` from `indoors` + flat slot.
pub fn placed_container_for_flat_slot(
    indoors: bool,
    flat_slot: u16,
) -> Option<(ContainerType, u16)> {
    if indoors {
        indoor_container_for_flat_slot(flat_slot)
    } else {
        let page = (flat_slot / SLOTS_PER_EXTERIOR_PAGE) as usize;
        let slot = flat_slot % SLOTS_PER_EXTERIOR_PAGE;
        EXTERIOR_PLACED_CONTAINERS
            .get(page)
            .copied()
            .map(|container| (container, slot))
    }
}

/// Returns whether a container holds indoor placed furnishings.
pub fn is_interior_placed_container(container: ContainerType) -> bool {
    INTERIOR_PLACED_CONTAINERS.contains(&container)
}

/// Returns whether a container holds indoor storeroom items.
pub fn is_interior_storeroom_container(container: ContainerType) -> bool {
    INTERIOR_STOREROOM_CONTAINERS.contains(&container)
}

/// Returns whether a container holds outdoor placed furnishings.
pub fn is_exterior_placed_container(container: ContainerType) -> bool {
    EXTERIOR_PLACED_CONTAINERS.contains(&container)
}

/// Returns whether a container holds outdoor storeroom items.
pub fn is_exterior_storeroom_container(container: ContainerType) -> bool {
    EXTERIOR_STOREROOM_CONTAINERS.contains(&container)
}

/// Returns the number of slots per page for a housing storeroom container.
pub fn storeroom_slots_per_page(container: ContainerType) -> u16 {
    if is_exterior_storeroom_container(container) {
        SLOTS_PER_EXTERIOR_PAGE
    } else {
        SLOTS_PER_HOUSING_PAGE
    }
}

/// Returns whether a container belongs to housing placed furniture (indoor or outdoor).
pub fn is_placed_furniture_container(container: ContainerType) -> bool {
    is_exterior_placed_container(container) || is_interior_placed_container(container)
}

/// Returns whether a container belongs to housing storeroom storage (indoor or outdoor).
pub fn is_storeroom_container(container: ContainerType) -> bool {
    is_exterior_storeroom_container(container) || is_interior_storeroom_container(container)
}

/// Returns whether a container belongs to housing appearance/remodeling fixtures.
pub fn is_appearance_container(container: ContainerType) -> bool {
    matches!(
        container,
        ContainerType::HousingExteriorAppearance
            | ContainerType::HousingInteriorAppearance
            | ContainerType::HousingExteriorAppearanceEdit
            | ContainerType::HousingInteriorAppearanceEdit
    )
}

/// Returns whether a container is any housing container managed by the simulator.
pub fn is_housing_container(container: ContainerType) -> bool {
    is_placed_furniture_container(container)
        || is_storeroom_container(container)
        || is_appearance_container(container)
}

/// Returns all database container type IDs considered placed furniture for an indoor/outdoor context.
pub fn placed_container_ids_for_scope(indoors: bool) -> Vec<i32> {
    if indoors {
        INTERIOR_PLACED_CONTAINERS
            .iter()
            .map(|c| *c as i32)
            .collect()
    } else {
        EXTERIOR_PLACED_CONTAINERS
            .iter()
            .map(|c| *c as i32)
            .collect()
    }
}

/// Returns the container slice list for client `UpdateItem`/`ContainerInfo` synchronization.
pub fn containers_for_request(
    intended_use: TerritoryIntendedUse,
    storeroom: bool,
) -> Vec<ContainerType> {
    match (intended_use, storeroom) {
        (TerritoryIntendedUse::HousingOutdoor, false) => EXTERIOR_PLACED_CONTAINERS.to_vec(),
        (TerritoryIntendedUse::HousingOutdoor, true) => EXTERIOR_STOREROOM_CONTAINERS.to_vec(),
        (TerritoryIntendedUse::HousingIndoor, false) => INTERIOR_PLACED_CONTAINERS.to_vec(),
        (TerritoryIntendedUse::HousingIndoor, true) => INTERIOR_STOREROOM_CONTAINERS.to_vec(),
        _ => Vec::new(),
    }
}

/// Decodes a `ContainerType` from its persisted or wire `u16` value.
pub fn container_from_u16(raw: u16) -> Option<ContainerType> {
    match raw {
        0 => Some(ContainerType::Inventory0),
        1 => Some(ContainerType::Inventory1),
        2 => Some(ContainerType::Inventory2),
        3 => Some(ContainerType::Inventory3),
        1000 => Some(ContainerType::Equipped),
        2011 => Some(ContainerType::HousingExteriorAppearanceEdit),
        2012 => Some(ContainerType::HousingInteriorAppearanceEdit),
        25000 => Some(ContainerType::HousingExteriorAppearance),
        25001 => Some(ContainerType::HousingExteriorPlacedItems),
        25002 => Some(ContainerType::HousingInteriorAppearance),
        25003 => Some(ContainerType::HousingInteriorPlacedItems1),
        25004 => Some(ContainerType::HousingInteriorPlacedItems2),
        25005 => Some(ContainerType::HousingInteriorPlacedItems3),
        25006 => Some(ContainerType::HousingInteriorPlacedItems4),
        25007 => Some(ContainerType::HousingInteriorPlacedItems5),
        25008 => Some(ContainerType::HousingInteriorPlacedItems6),
        25009 => Some(ContainerType::HousingInteriorPlacedItems7),
        25010 => Some(ContainerType::HousingInteriorPlacedItems8),
        25011 => Some(ContainerType::HousingInteriorPlacedItems9),
        25012 => Some(ContainerType::HousingInteriorPlacedItems10),
        25013 => Some(ContainerType::HousingInteriorPlacedItems11),
        25014 => Some(ContainerType::HousingInteriorPlacedItems12),
        25200 => Some(ContainerType::HousingExteriorPlacedItems2),
        27000 => Some(ContainerType::HousingExteriorStoreroom),
        27001 => Some(ContainerType::HousingInteriorStoreroom1),
        27002 => Some(ContainerType::HousingInteriorStoreroom2),
        27003 => Some(ContainerType::HousingInteriorStoreroom3),
        27004 => Some(ContainerType::HousingInteriorStoreroom4),
        27005 => Some(ContainerType::HousingInteriorStoreroom5),
        27006 => Some(ContainerType::HousingInteriorStoreroom6),
        27007 => Some(ContainerType::HousingInteriorStoreroom7),
        27008 => Some(ContainerType::HousingInteriorStoreroom8),
        27009 => Some(ContainerType::HousingInteriorStoreroom9),
        27010 => Some(ContainerType::HousingInteriorStoreroom10),
        27011 => Some(ContainerType::HousingInteriorStoreroom11),
        27012 => Some(ContainerType::HousingInteriorStoreroom12),
        27200 => Some(ContainerType::HousingExteriorStoreroom2),
        _ => None,
    }
}

/// Decodes a `ContainerType` from its persisted integer value.
pub fn container_from_i32(raw: i32) -> Option<ContainerType> {
    u16::try_from(raw).ok().and_then(container_from_u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_slot_preserves_full_600_range_without_u8_truncation() {
        for slot in [0u16, 49, 50, 255, 256, 399, 400, 599] {
            let (container, page_slot) =
                placed_container_for_flat_slot(true, slot).expect("slot 0..599 must map");
            assert!(page_slot < 50);
            let roundtrip =
                flat_slot_for_placed_container(container, page_slot).expect("must roundtrip");
            assert_eq!(roundtrip, slot);
        }
        assert!(placed_container_for_flat_slot(true, 600).is_none());
    }

    #[test]
    fn outdoor_containers_map_two_40_slot_pages_and_40_60_80_capacities() {
        assert_eq!(outdoor_slot_capacity(PlotSize::Small), 40);
        assert_eq!(outdoor_slot_capacity(PlotSize::Medium), 60);
        assert_eq!(outdoor_slot_capacity(PlotSize::Large), 80);

        for slot in [0u16, 39, 40, 59, 60, 79] {
            let (container, page_slot) =
                placed_container_for_flat_slot(false, slot).expect("slot 0..80 must map");
            assert!(page_slot < 40);
            let expected_container = if slot < 40 {
                ContainerType::HousingExteriorPlacedItems
            } else {
                ContainerType::HousingExteriorPlacedItems2
            };
            assert_eq!(container, expected_container);
            assert_eq!(
                flat_slot_for_placed_container(container, page_slot),
                Some(slot)
            );
        }
        assert!(placed_container_for_flat_slot(false, 80).is_none());
        assert!(
            flat_slot_for_placed_container(ContainerType::HousingExteriorPlacedItems, 40).is_none()
        );
        assert!(
            flat_slot_for_placed_container(ContainerType::HousingExteriorPlacedItems2, 40)
                .is_none()
        );
        assert_eq!(
            container_from_u16(25200),
            Some(ContainerType::HousingExteriorPlacedItems2)
        );
        assert_eq!(
            container_from_u16(27200),
            Some(ContainerType::HousingExteriorStoreroom2)
        );
    }
}
