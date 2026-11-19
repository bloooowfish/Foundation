use std::path::Path;

use kawari::common::{
    ContainerType, EstateType, HouseId, HousingFlag, PlotState, Position, TerritoryIntendedUse,
    WarpType, internal_housing_row,
};
use kawari::constants::COMPLETED_QUEST_BITMASK_SIZE;
use kawari::ipc::zone::{
    ActorControlCategory, ApartmentBuildingState, ApartmentList, AvailabilityType, FurnitureList,
    GameMasterRank, GrandCompany as IpcGrandCompany, House, HouseList, HousingEstateGreeting,
    HousingInteriorDetails, HousingOccupiedLandInfo, HousingVacantLandInfo, HousingWardInfo,
    HousingWardSummaryItem, IndoorFurnitureMaterialUpdated, PlotSize, PurchaseType,
    ServerZoneIpcData, ServerZoneIpcSegment, TenantType, UpdateMapMarkersForWardDivision,
    WardDivision, WardDivisionInfo, WardMapInfo, WardMapInfoFlags,
};

use crate::common::ToServer;
use crate::inventory::{CurrencyKind, Item, Storage};
use crate::{GameData, ItemInfoQuery, PlayerData, WorldDatabase, ZoneConnection};

use super::containers::{
    MAX_EXTERIOR_PLACED_SLOTS, MAX_INDOOR_PLACED_SLOTS, containers_for_request,
    indoor_slot_capacity, placed_container_for_flat_slot,
};
use super::db::{
    EnsureEstateSpec, HousingEstateRow, HousingFurnitureRow, MAX_ESTATE_GREETING_BYTES,
    MAX_ESTATE_NAME_BYTES, apartments_in_ward, clamp_utf8, delete_furniture_slot, ensure_estate,
    estate_by_house_id, estate_by_land_ident, estate_by_location, estates_in_ward,
    list_placed_furniture_in_scope, owned_estates_for_character, save_estate,
};
use super::fixtures::{
    StoredHouseExterior, StoredHouseInterior, appeal_tags_from_row, build_apartment_entries,
    build_house_list_entry, build_housing_object_data_value_sets, build_indoor_furniture_lists,
    build_outdoor_furniture_list, build_owned_housing_data, housing_interior_ready_primary_id,
    housing_interior_ready_secondary_id, is_fc_estate, ward_summary_flags_for_estate,
};
use super::gamedata_ext::{
    HousingCatalog, cached_housing_catalog, default_renovation_row_for_district_and_size,
    renovation_territory_for_row,
};
use super::overlays::{HousingOverlayKey, HousingOverlayObject, HousingOverlayScope};
use super::plots::{
    apartment_territory_for_district, city_fallback_aetheryte_for_territory,
    default_district_for_zone, is_apartment_lobby_territory, is_apartment_territory,
    is_house_interior_territory, nearest_plot_to_position, plot_spec,
};
use super::remake_place::PresetScope;
use super::session::{
    ActiveEstateSelection, HousingSimToServer, HousingSseEvent, LoginWardSpawn,
    OnlineSessionSnapshot, ReloadMode, WardContext,
};

const MAX_INITIAL_INDOOR_FURNITURE_LISTS: usize = 3;

pub fn build_selection_from_row(row: &HousingEstateRow, indoors: bool) -> ActiveEstateSelection {
    ActiveEstateSelection {
        land_ident: row.land_ident,
        house_id: row.house_id as u64,
        territory_type_id: row.territory_type_id as u16,
        ward_index: row.ward_index as u8,
        division: row.division as u8,
        plot_index: row.plot_index as u8,
        room_number: row.room_number as u16,
        is_apartment: row.is_apartment,
        plot_size: row.plot_size.clamp(0, 2) as u8,
        indoors,
    }
}

pub fn publish_session_snapshot(connection: &ZoneConnection, intended_use: TerritoryIntendedUse) {
    let Some(ref hub) = connection.housing_hub else {
        return;
    };
    let pos = connection.player_data.volatile.position;
    let snap = OnlineSessionSnapshot {
        client_id_debug: format!("{:?}", connection.id),
        client_id: Some(connection.id),
        content_id: connection.player_data.character.content_id as u64,
        actor_id: connection.player_data.character.actor_id.0,
        character_name: connection.player_data.character.name.clone(),
        zone_id: connection.player_data.volatile.zone_id as u16,
        intended_use,
        position: [pos.0.x, pos.0.y, pos.0.z],
        rotation: connection.player_data.volatile.rotation as f32,
        active_estate: connection.housing_sim.active_estate.clone(),
        display_ward_context: connection.housing_sim.display_ward_context,
        authoritative_ward_context: connection.housing_sim.authoritative_ward_context,
    };
    hub.update_session(connection.id, snap);
}

/// Ensures invalid/orphaned indoor login positions return cleanly to the estate front gate or apartment entrance,
/// and preserves outdoor ward spawns inside the right `Ward` instance.
pub fn normalize_indoor_login_location(connection: &mut ZoneConnection) {
    connection.housing_sim.login_ward_spawn = None;
    let zone_id = connection.player_data.volatile.zone_id as u16;
    let in_indoor_house = is_house_interior_territory(zone_id);
    let in_apartment_or_lobby =
        is_apartment_territory(zone_id) || is_apartment_lobby_territory(zone_id);
    let in_outdoor_ward = internal_housing_row(zone_id).is_some();

    if !in_indoor_house && !in_apartment_or_lobby && !in_outdoor_ward {
        return;
    }

    let content_id = connection.player_data.character.content_id as u64;
    let estates = {
        let mut db = connection.database.lock();
        owned_estates_for_character(db.connection_mut(), content_id).unwrap_or_default()
    };

    if in_outdoor_ward {
        let matched = estates
            .iter()
            .find(|e| e.territory_type_id as u16 == zone_id && !e.is_apartment)
            .or_else(|| {
                estates
                    .iter()
                    .find(|e| e.territory_type_id as u16 == zone_id)
            });
        let (ward_index, division) = matched
            .map(|e| (e.ward_index as u8, e.division.clamp(0, 1) as u8))
            .unwrap_or((0, 0));
        connection.entering_subdivision = division != 0;
        connection.housing_sim.login_ward_spawn = Some(LoginWardSpawn {
            territory_type_id: zone_id,
            ward_index,
            division,
            plot_index: None,
            is_apartment_exit: false,
        });
        return;
    }

    let expected_district = default_district_for_zone(zone_id);
    let target_estate = estates
        .iter()
        .find(|e| {
            e.is_apartment == in_apartment_or_lobby
                && e.territory_type_id as u16 == expected_district
        })
        .or_else(|| {
            estates
                .iter()
                .find(|e| e.is_apartment == in_apartment_or_lobby)
        })
        .cloned();

    if let Some(estate) = target_estate {
        let district = estate.territory_type_id as u16;
        let ward_index = estate.ward_index as u8;
        let division = estate.division.clamp(0, 1) as u8;
        let plot_index = estate.plot_index.clamp(0, 29) as u8;
        connection.player_data.volatile.zone_id = district as i32;
        connection.entering_subdivision = division != 0;
        connection.housing_sim.active_estate = Some(build_selection_from_row(&estate, false));
        connection.housing_sim.is_owner_authorized = true;

        if !estate.is_apartment
            && let Some(spec) = plot_spec(district, division, plot_index)
        {
            let (exit_pos, exit_rot) = super::plots::nudge_plot_entrance_exit(
                Position(glam::Vec3A::new(spec.x, spec.y, spec.z)),
                spec.rotation,
                8.0,
            );
            connection.player_data.volatile.position = exit_pos;
            connection.player_data.volatile.rotation = exit_rot as f64;
        }

        connection.housing_sim.login_ward_spawn = Some(LoginWardSpawn {
            territory_type_id: district,
            ward_index,
            division,
            plot_index: (!estate.is_apartment && !in_apartment_or_lobby).then_some(plot_index),
            is_apartment_exit: estate.is_apartment || in_apartment_or_lobby,
        });
        return;
    }

    // Fallback to matching outdoor district entrance when no persisted estate record exists yet.
    let _ = city_fallback_aetheryte_for_territory(zone_id);
    connection.player_data.volatile.zone_id = expected_district as i32;
    connection.entering_subdivision = false;
    if !in_apartment_or_lobby && let Some(spec) = plot_spec(expected_district, 0, 0) {
        let (exit_pos, exit_rot) = super::plots::nudge_plot_entrance_exit(
            Position(glam::Vec3A::new(spec.x, spec.y, spec.z)),
            spec.rotation,
            8.0,
        );
        connection.player_data.volatile.position = exit_pos;
        connection.player_data.volatile.rotation = exit_rot as f64;
    }
    connection.housing_sim.login_ward_spawn = Some(LoginWardSpawn {
        territory_type_id: expected_district,
        ward_index: 0,
        division: 0,
        plot_index: (!in_apartment_or_lobby).then_some(0),
        is_apartment_exit: in_apartment_or_lobby,
    });
}

/// Sends `OwnedHousing` packet from persistent DB (`housing_estates`), dynamically projecting `active_estate`
/// so a single character can build/remodel unlimited test plots and apartments across districts and sizes.
pub async fn send_owned_housing(connection: &mut ZoneConnection) {
    let content_id = connection.player_data.character.content_id as u64;
    let active_ident = connection
        .housing_sim
        .active_estate
        .as_ref()
        .map(|s| s.land_ident);
    let is_authorized = connection.housing_sim.is_owner_authorized;

    let (owned_estates, preferred_active) = {
        let mut db = connection.database.lock();
        let conn = db.connection_mut();
        let estates = owned_estates_for_character(conn, content_id).unwrap_or_default();
        let active_row = active_ident
            .and_then(|ident| estate_by_land_ident(conn, ident).ok().flatten())
            .filter(|est| {
                is_authorized
                    || est.owner_content_id == 0
                    || est.owner_content_id == content_id as i64
            });
        (estates, active_row)
    };
    let (owned, apartment) = build_owned_housing_data(&owned_estates, preferred_active.as_ref());
    let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::OwnedHousing {
        unk1: owned[0],
        unk2: owned[1],
        unk3: owned[2],
        unk4: owned[3],
        unk5: owned[4],
        apartment,
    });
    connection.send_ipc_self(ipc).await;
}

/// Resolves active catalog via `ZoneConnection`'s mutex-guarded `GameData`.
pub fn connection_catalog(connection: &ZoneConnection) -> std::sync::Arc<HousingCatalog> {
    let mut gd = connection.game_data.lock();
    cached_housing_catalog(&mut gd)
}

/// Builds overlay objects from DB furniture rows for either indoor or outdoor scope.
pub fn build_overlay_objects_from_rows(
    rows: &[HousingFurnitureRow],
    indoors: bool,
    ward_index: u8,
    plot_index: u8,
) -> Vec<HousingOverlayObject> {
    let mut out = Vec::new();
    for row in rows {
        let Some(ctype) = super::containers::container_from_i32(row.container_type) else {
            continue;
        };
        let Some(flat_slot) =
            super::containers::flat_slot_for_placed_container(ctype, row.slot as u16)
        else {
            continue;
        };
        out.push(HousingOverlayObject {
            key: HousingOverlayKey {
                indoors,
                ward_index,
                plot_index,
                slot: flat_slot,
            },
            catalog_id: row.catalog_id.max(0) as u16,
            position: Position(glam::Vec3A::new(row.pos_x, row.pos_y, row.pos_z)),
            rotation: row.rotation,
        });
    }
    out
}

/// Ensures or creates a local personal/FC plot estate in SQLite and returns it.
pub fn ensure_local_plot_estate(
    connection: &mut ZoneConnection,
    territory_type_id: u16,
    ward_index: u8,
    division: u8,
    plot_index: u8,
    requested_size: Option<PlotSize>,
    is_fc: bool,
    name_override: Option<String>,
) -> Option<HousingEstateRow> {
    let catalog = connection_catalog(connection);
    let world_id = connection.config.world_id;
    let content_id = connection.player_data.character.content_id as u64;
    let owner_name = connection.player_data.character.name.clone();

    let resolved_size = requested_size.unwrap_or_else(|| {
        plot_spec(territory_type_id, division, plot_index)
            .map(|p| p.size)
            .unwrap_or(PlotSize::Small)
    });

    let exterior =
        StoredHouseExterior::default_for_location(&catalog, territory_type_id, resolved_size);
    let interior = StoredHouseInterior::default_for_location(
        &catalog,
        territory_type_id,
        resolved_size,
        false,
    );

    let spec = EnsureEstateSpec {
        territory_type_id,
        world_id,
        ward_index,
        division,
        plot_index,
        room_number: 0,
        is_apartment: false,
        plot_size: resolved_size,
        is_fc,
        owner_content_id: content_id,
        owner_name: owner_name.clone(),
        estate_name: name_override.unwrap_or_else(|| format!("{owner_name}'s Estate")),
        exterior_json: exterior.to_json(),
        interior_json: interior.to_json(),
    };

    let mut db = connection.database.lock();
    let conn = db.connection_mut();
    let existing = estate_by_location(
        conn,
        territory_type_id,
        world_id,
        ward_index,
        division,
        plot_index,
    )
    .ok()
    .flatten();
    if let Some(mut row) = existing {
        // Update size or ownership if explicitly recreated via !housing testhouse.
        if let Some(sz) = requested_size
            && row.plot_size != sz as i32
        {
            row.plot_size = sz as i32;
            let mut int = StoredHouseInterior::from_json(&row.interior_json);
            int.renovation_row_id =
                default_renovation_row_for_district_and_size(territory_type_id, sz);
            row.interior_json = int.to_json();
            let _ = save_estate(conn, &row);
        }
        Some(row)
    } else {
        ensure_estate(conn, &spec).ok()
    }
}

/// Ensures or creates a local apartment room in SQLite and returns it.
pub fn ensure_local_apartment_estate(
    connection: &mut ZoneConnection,
    territory_type_id: u16,
    ward_index: u8,
    division: u8,
    room_number: u16,
) -> Option<HousingEstateRow> {
    let district = default_district_for_zone(territory_type_id);
    let catalog = connection_catalog(connection);
    let interior =
        StoredHouseInterior::default_for_location(&catalog, district, PlotSize::Small, true);
    let owner_name = connection.player_data.character.name.clone();
    let spec = EnsureEstateSpec {
        territory_type_id: district,
        world_id: connection.config.world_id,
        ward_index,
        division,
        plot_index: 0,
        room_number: room_number.max(1),
        is_apartment: true,
        plot_size: PlotSize::Small,
        is_fc: false,
        owner_content_id: connection.player_data.character.content_id as u64,
        owner_name: owner_name.clone(),
        estate_name: format!("{owner_name}'s Apt #{room_number}"),
        exterior_json: "{}".to_string(),
        interior_json: interior.to_json(),
    };
    let mut db = connection.database.lock();
    ensure_estate(db.connection_mut(), &spec).ok()
}

/// Populates `player_data.house_inventory` from `housing_furniture` rows for an active estate.
pub fn rebuild_house_inventory_from_db(
    connection: &mut ZoneConnection,
    estate: &HousingEstateRow,
    indoors: bool,
) {
    connection.player_data.house_inventory = Default::default();
    let rows = {
        let mut db = connection.database.lock();
        super::db::list_furniture_for_estate(db.connection_mut(), estate.land_ident, None)
            .unwrap_or_default()
    };

    for row in rows {
        let Some(ctype) = super::containers::container_from_i32(row.container_type) else {
            continue;
        };
        let is_matching_scope = if indoors {
            super::containers::is_interior_placed_container(ctype)
                || super::containers::is_interior_storeroom_container(ctype)
        } else {
            ctype == ContainerType::HousingExteriorPlacedItems
                || ctype == ContainerType::HousingExteriorStoreroom
        };
        if !is_matching_scope {
            continue;
        }
        if let Some(slot_mut) = connection
            .player_data
            .house_inventory
            .get_item_mut(ctype, row.slot as u16)
        {
            *slot_mut = Item {
                item_id: row.item_id.max(0) as u32,
                quantity: 1,
                stains: [row.stain.clamp(0, u8::MAX as i32) as u8, 0],
                ..Default::default()
            };
        }
    }

    // Populate appearance container items so native exterior/interior remodel previews show equipped parts.
    let catalog = connection_catalog(connection);
    if indoors {
        let interior = StoredHouseInterior::from_json(&estate.interior_json);
        let vals = interior.appearance_values();
        for (idx, add_data) in vals.into_iter().enumerate() {
            if add_data == 0 {
                continue;
            }
            if let Some(meta) = catalog.item_for_additional_data(add_data, None)
                && let Some(slot_mut) = connection
                    .player_data
                    .house_inventory
                    .get_item_mut(ContainerType::HousingInteriorAppearance, idx as u16)
            {
                *slot_mut = Item {
                    item_id: meta.item_id,
                    quantity: 1,
                    ..Default::default()
                };
            }
        }
    } else {
        let exterior = StoredHouseExterior::from_json(&estate.exterior_json);
        let parts = exterior.part_ids();
        let stains = [
            exterior.roof_stain,
            exterior.walls_stain,
            exterior.windows_stain,
            exterior.door_stain,
            exterior.roof_fixture_stain,
            exterior.wall_fixture_stain,
            exterior.above_door_banner_stain,
            exterior.fence_stain,
        ];
        for (idx, part_id) in parts.into_iter().enumerate() {
            if part_id == 0 {
                continue;
            }
            if let Some(meta) =
                catalog.item_for_additional_data(part_id as u32, Some(65 + idx as u8))
                && let Some(slot_mut) = connection
                    .player_data
                    .house_inventory
                    .get_item_mut(ContainerType::HousingExteriorAppearance, idx as u16)
            {
                *slot_mut = Item {
                    item_id: meta.item_id,
                    quantity: 1,
                    stains: [stains[idx], 0],
                    ..Default::default()
                };
            }
        }
    }
}

/// Initializes an outdoor residential ward after `ZoneInit`.
pub async fn init_outdoor_ward(connection: &mut ZoneConnection, zone_id: u16, ward_index: u16) {
    let Some(land_id) = internal_housing_row(zone_id) else {
        return;
    };

    let ward = ward_index as u8;
    let division = if connection.entering_subdivision {
        1u8
    } else {
        0u8
    };
    connection.entering_subdivision = false;
    connection.housing_sim.pending_indoor_furniture_list_tail = None;
    connection.housing_sim.pending_indoor_finish_loading = false;
    connection.housing_sim.pending_indoor_overlay_sync = false;

    connection.housing_sim.display_ward_context = Some(WardContext {
        territory_type_id: zone_id,
        ward_index: ward,
        division,
    });

    let world_id = connection.config.world_id;
    let content_id = connection.player_data.character.content_id as u64;

    let (ward_estates, all_owned, main_houses, sub_houses, opp_markers, ward_furn_lists, apt_state) = {
        let mut db = connection.database.lock();
        let conn = db.connection_mut();
        let ward_estates = estates_in_ward(conn, zone_id, world_id, ward).unwrap_or_default();
        let all_owned = owned_estates_for_character(conn, content_id).unwrap_or_default();
        let has_apt_in_div = apartments_in_ward(conn, zone_id, world_id, ward, division)
            .unwrap_or_default()
            .iter()
            .any(|a| a.owner_content_id == content_id as i64);

        let mut main_houses = [House::default(); 30];
        let mut sub_houses = [House::default(); 30];
        let mut opp_markers = [WardMapInfo::default(); 30];
        let mut furn_lists = Vec::new();

        for estate in &ward_estates {
            let entry = build_house_list_entry(estate);
            let p_idx = estate.plot_index as usize;
            if estate.division == 0 {
                if p_idx < 30 {
                    main_houses[p_idx] = entry;
                }
            } else if p_idx < 30 {
                sub_houses[p_idx] = entry;
            }

            let placed =
                list_placed_furniture_in_scope(conn, estate.land_ident, false).unwrap_or_default();
            furn_lists.push(build_outdoor_furniture_list(estate, &placed));
        }

        let opp_slice = if division == 0 {
            &sub_houses
        } else {
            &main_houses
        };
        for (h, m) in opp_slice.iter().zip(opp_markers.iter_mut()) {
            let mut mflags = if h.status != PlotState::UnownedLand {
                WardMapInfoFlags::HAS_OWNER
            } else {
                WardMapInfoFlags::empty()
            };
            if h.flags.contains(HousingFlag::OWNED_BY_FC) {
                mflags |= WardMapInfoFlags::IS_FREE_COMPANY;
            }
            *m = WardMapInfo {
                state: h.status,
                size: h.plot_size,
                flags: mflags,
            };
        }

        let apt_state = if has_apt_in_div {
            ApartmentBuildingState::IsOwn
        } else {
            ApartmentBuildingState::None
        };
        (
            ward_estates,
            all_owned,
            main_houses,
            sub_houses,
            opp_markers,
            furn_lists,
            apt_state,
        )
    };

    // Preserve the already-selected plot in this ward if owned/authorized; otherwise select one in division/ward.
    let active_ident = connection
        .housing_sim
        .active_estate
        .as_ref()
        .map(|s| s.land_ident);
    let is_authorized_row =
        |e: &&HousingEstateRow| e.owner_content_id == 0 || e.owner_content_id == content_id as i64;
    let owned_in_ward = active_ident
        .and_then(|ident| {
            ward_estates
                .iter()
                .find(|e| e.land_ident == ident && is_authorized_row(e))
        })
        .or_else(|| {
            ward_estates
                .iter()
                .find(|e| e.division as u8 == division && is_authorized_row(e))
        })
        .or_else(|| ward_estates.iter().find(|e| is_authorized_row(e)))
        .or_else(|| {
            active_ident.and_then(|ident| {
                all_owned.iter().find(|e| {
                    !e.is_apartment
                        && e.land_ident == ident
                        && e.territory_type_id as u16 == zone_id
                        && e.ward_index as u8 == ward
                })
            })
        })
        .or_else(|| {
            all_owned.iter().find(|e| {
                !e.is_apartment
                    && e.territory_type_id as u16 == zone_id
                    && e.ward_index as u8 == ward
            })
        });

    if let Some(owned) = owned_in_ward {
        connection.housing_sim.authoritative_ward_context = Some(WardContext {
            territory_type_id: owned.territory_type_id as u16,
            ward_index: owned.ward_index as u8,
            division: owned.division as u8,
        });
        connection.housing_sim.active_estate = Some(build_selection_from_row(owned, false));
        connection.housing_sim.is_owner_authorized = true;
        rebuild_house_inventory_from_db(connection, owned, false);
    } else {
        connection.housing_sim.is_owner_authorized = false;
    }

    let houses = if division == 0 {
        main_houses
    } else {
        sub_houses
    };
    connection
        .send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::HouseList(
            HouseList {
                land_id: land_id as u16,
                ward: ward_index,
                territory_type_id: zone_id,
                world_id,
                division_info: WardDivisionInfo {
                    apartment_building_state: apt_state,
                    ward_division: if division == 0 {
                        WardDivision::MainDivision
                    } else {
                        WardDivision::SubDivision
                    },
                },
                houses,
            },
        )))
        .await;

    connection
        .send_ipc_self(ServerZoneIpcSegment::new(
            ServerZoneIpcData::UpdateMapMarkersForWardDivision(UpdateMapMarkersForWardDivision {
                ward_division_info: WardDivisionInfo {
                    apartment_building_state: ApartmentBuildingState::None,
                    ward_division: if division == 0 {
                        WardDivision::SubDivision
                    } else {
                        WardDivision::MainDivision
                    },
                },
                info: opp_markers,
            }),
        ))
        .await;

    // Send outdoor FurnitureList packets (empty placeholders if no estates exist yet).
    if ward_furn_lists.is_empty() {
        for index in 0..15 {
            connection
                .send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::FurnitureList(
                    FurnitureList {
                        estate_type: EstateType::Ward,
                        count: 15,
                        index,
                        furniture: vec![Default::default(); 100],
                        ..Default::default()
                    },
                )))
                .await;
        }
    } else {
        for list in ward_furn_lists {
            connection
                .send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::FurnitureList(
                    list,
                )))
                .await;
        }
    }

    send_owned_housing(connection).await;
    publish_session_snapshot(connection, TerritoryIntendedUse::HousingOutdoor);
}

/// Resolves and binds the active indoor estate before `send_owned_housing` and `ZoneInit`.
pub fn prepare_active_indoor_estate_for_zone_init(
    connection: &mut ZoneConnection,
    zone_id: u16,
    house_id: HouseId,
) -> Option<HousingEstateRow> {
    let content_id = connection.player_data.character.content_id as u64;
    let estate = {
        let mut db = connection.database.lock();
        let conn = db.connection_mut();
        estate_by_house_id(conn, house_id)
            .ok()
            .flatten()
            .or_else(|| {
                connection
                    .housing_sim
                    .active_estate
                    .as_ref()
                    .and_then(|sel| estate_by_land_ident(conn, sel.land_ident).ok().flatten())
            })
            .or_else(|| {
                owned_estates_for_character(conn, content_id)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|e| e.is_apartment == is_apartment_territory(zone_id))
            })
    }?;

    connection.housing_sim.active_estate = Some(build_selection_from_row(&estate, true));
    connection.housing_sim.is_owner_authorized =
        estate.owner_content_id == 0 || estate.owner_content_id == content_id as i64;
    Some(estate)
}

/// Initializes a housing interior or apartment after `ZoneInit`, deferring `FurnitureList` chunks beyond index 2 until `FinishEstateRemodel { mode: 255 }`.
pub async fn init_indoor_estate(connection: &mut ZoneConnection, zone_id: u16, house_id: HouseId) {
    let Some(estate) = prepare_active_indoor_estate_for_zone_init(connection, zone_id, house_id)
    else {
        tracing::info!(
            zone_id,
            house_id = u64::from(house_id),
            "Indoor init fallback: no matching estate, finalizing empty interior"
        );
        // Safe blank fallback for unseeded apartment lobby or unknown interior.
        connection
            .send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::HousingInteriorDetails(HousingInteriorDetails {
                    enable_ssao: true,
                    ..Default::default()
                }),
            ))
            .await;
        let raw_hid = u64::from(house_id);
        connection
            .actor_control_self(ActorControlCategory::HousingInteriorFinalizeEmpty {
                house_id_low: (raw_hid & 0xFFFF_FFFF) as u32,
                house_id_high: (raw_hid >> 32) as u32,
                estate_type: if is_apartment_territory(zone_id) {
                    6
                } else {
                    2
                },
            })
            .await;
        return;
    };

    let resolved_house_id = HouseId::from(estate.house_id as u64);
    rebuild_house_inventory_from_db(connection, &estate, true);

    let plot_size = PlotSize::from_repr(estate.plot_size.clamp(0, 2) as u8).unwrap_or_default();
    let stored_interior = StoredHouseInterior::from_json(&estate.interior_json);
    let details = stored_interior.to_packet(
        estate.light_level.clamp(0, 5) as u8,
        estate.enable_ssao,
        plot_size,
        estate.is_apartment,
    );

    connection
        .send_ipc_self(ServerZoneIpcSegment::new(
            ServerZoneIpcData::HousingInteriorDetails(details),
        ))
        .await;

    let placed_rows = {
        let mut db = connection.database.lock();
        list_placed_furniture_in_scope(db.connection_mut(), estate.land_ident, true)
            .unwrap_or_default()
    };

    let lists = build_indoor_furniture_lists(
        resolved_house_id,
        plot_size,
        estate.is_apartment,
        &placed_rows,
    );

    let initial_count = lists.len().min(MAX_INITIAL_INDOOR_FURNITURE_LISTS);
    for list in &lists[..initial_count] {
        connection
            .send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::FurnitureList(
                list.clone(),
            )))
            .await;
    }

    connection.housing_sim.pending_indoor_overlay_sync = true;
    connection.housing_sim.pending_indoor_finish_loading = false;
    connection.housing_sim.pending_indoor_finish_loading_since = None;

    let has_deferred_tail = lists.len() > MAX_INITIAL_INDOOR_FURNITURE_LISTS;
    tracing::info!(
        land_ident = estate.land_ident,
        zone_id,
        house_id = u64::from(resolved_house_id),
        total_furniture = placed_rows.len(),
        total_lists = lists.len(),
        initial_lists = initial_count,
        deferred = has_deferred_tail,
        "Indoor init prepared furniture chunks"
    );

    if has_deferred_tail {
        connection.housing_sim.pending_indoor_furniture_list_tail =
            Some(lists[MAX_INITIAL_INDOOR_FURNITURE_LISTS..].to_vec());
    } else {
        connection.housing_sim.pending_indoor_furniture_list_tail = None;
        finish_indoor_ready_sequence(connection, &estate, resolved_house_id, &placed_rows).await;
    }

    if !is_fc_estate(&estate) && !estate.is_apartment {
        connection
            .actor_control_self(ActorControlCategory::HideAdditionalChambersDoor {})
            .await;
    }

    publish_session_snapshot(connection, TerritoryIntendedUse::HousingIndoor);
}

/// Sends `HousingObjectDataValueSet` (`S>C Opcode 971`) packets for placed indoor furniture rows.
async fn send_indoor_object_data_value_sets(
    connection: &mut ZoneConnection,
    placed_rows: &[HousingFurnitureRow],
) {
    let segments: Vec<ServerZoneIpcSegment> = build_housing_object_data_value_sets(placed_rows)
        .into_iter()
        .map(|value_set| {
            ServerZoneIpcSegment::new(ServerZoneIpcData::HousingObjectDataValueSet(value_set))
        })
        .collect();
    if !segments.is_empty() {
        connection.send_ipcs_self(&segments).await;
    }
}

/// Sends `HousingObjectDataValueSet` (`971`), `HousingInteriorReady` (`1010`), and silent initial lighting (`1035`).
async fn finish_indoor_ready_sequence(
    connection: &mut ZoneConnection,
    estate: &HousingEstateRow,
    house_id: HouseId,
    placed_rows: &[HousingFurnitureRow],
) {
    send_indoor_object_data_value_sets(connection, placed_rows).await;

    connection
        .actor_control_self(ActorControlCategory::HousingInteriorReady {
            primary_id: housing_interior_ready_primary_id(house_id),
            secondary_id: housing_interior_ready_secondary_id(estate.land_ident, house_id),
            unk1: 0,
        })
        .await;

    connection
        .actor_control_self(ActorControlCategory::InteriorLightLevel {
            unk1: 1, // silent init
            level: estate.light_level.clamp(0, 5) as u32,
            unk2: u32::from(estate.enable_ssao),
        })
        .await;
}

/// Delivers deferred `FurnitureList` tail chunks (`3..`), ready packets, and any deferred `FinishLoading` sequence.
async fn flush_deferred_indoor_tail(connection: &mut ZoneConnection, reason: &str) -> bool {
    connection.housing_sim.pending_indoor_finish_loading_since = None;
    let Some(tail) = connection
        .housing_sim
        .pending_indoor_furniture_list_tail
        .take()
    else {
        connection.housing_sim.pending_indoor_finish_loading = false;
        return false;
    };

    let active_land_ident = connection
        .housing_sim
        .active_estate
        .as_ref()
        .map(|e| e.land_ident);
    tracing::info!(
        land_ident = active_land_ident,
        tail_chunks = tail.len(),
        pending_finish_loading = connection.housing_sim.pending_indoor_finish_loading,
        reason,
        "Flushing deferred indoor furniture tail"
    );

    for list in tail {
        connection
            .send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::FurnitureList(
                list,
            )))
            .await;
    }

    if let Some(ref active) = connection.housing_sim.active_estate.clone() {
        let (estate, placed_rows) = {
            let mut db = connection.database.lock();
            let conn = db.connection_mut();
            let est = estate_by_land_ident(conn, active.land_ident).ok().flatten();
            let rows =
                list_placed_furniture_in_scope(conn, active.land_ident, true).unwrap_or_default();
            (est, rows)
        };
        if let Some(est) = estate {
            finish_indoor_ready_sequence(
                connection,
                &est,
                HouseId::from(active.house_id),
                &placed_rows,
            )
            .await;
        }
    }

    if connection.housing_sim.pending_indoor_finish_loading {
        connection.housing_sim.pending_indoor_finish_loading = false;
        let spawn = connection.respawn_player(true).await;
        connection
            .handle
            .send(ToServer::ZoneLoaded(
                connection.id,
                connection.player_data.character.actor_id,
                spawn,
            ))
            .await;

        if connection.is_in_party() {
            if !connection.rejoining_party {
                connection
                    .handle
                    .send(ToServer::PartyMemberChangedAreas(
                        connection.party_id,
                        connection.player_data.character.service_account_id as u64,
                        connection.player_data.character.content_id as u64,
                        connection.player_data.character.actor_id,
                        connection.player_data.character.name.clone(),
                        connection.player_data.volatile.zone_id,
                    ))
                    .await;
            } else {
                connection
                    .handle
                    .send(ToServer::PartyMemberReturned(
                        connection.player_data.character.actor_id,
                        connection.player_data.volatile.zone_id,
                        connection.party_id,
                    ))
                    .await;
                connection.rejoining_party = false;
            }
        }

        connection.send_stats().await;
        sync_current_overlays_for_connection(connection, TerritoryIntendedUse::HousingIndoor).await;
        tracing::info!(
            land_ident = active_land_ident,
            reason,
            "Completed deferred indoor FinishLoading sequence"
        );
    } else {
        connection.housing_sim.pending_indoor_overlay_sync = true;
    }

    true
}

/// Delivers deferred `FurnitureList` chunks (`3..`) when `FinishEstateRemodel { mode: 255 }` arrives, then releases deferred `FinishLoading`.
pub async fn complete_deferred_indoor_tail_on_remodel(
    connection: &mut ZoneConnection,
    mode: u32,
) -> bool {
    if mode != 255 {
        return false;
    }
    flush_deferred_indoor_tail(connection, "finish_estate_remodel_255").await
}

/// Failsafe flush when a client holds deferred indoor furniture chunks past the timeout after `FinishLoading`.
pub async fn flush_deferred_indoor_tail_on_timeout(connection: &mut ZoneConnection) -> bool {
    flush_deferred_indoor_tail(connection, "finish_loading_timeout").await
}

/// Requests `server_main_loop` to replace interactive overlays (`HousingEventObject` & striking dummies) for the player's current scope.
pub async fn sync_current_overlays_for_connection(
    connection: &mut ZoneConnection,
    intended_use: TerritoryIntendedUse,
) {
    match intended_use {
        TerritoryIntendedUse::HousingIndoor => {
            connection.housing_sim.pending_indoor_overlay_sync = false;
            let Some(ref sel) = connection.housing_sim.active_estate else {
                return;
            };
            let rows = {
                let mut db = connection.database.lock();
                list_placed_furniture_in_scope(db.connection_mut(), sel.land_ident, true)
                    .unwrap_or_default()
            };
            let objects =
                build_overlay_objects_from_rows(&rows, true, sel.ward_index, sel.plot_index);
            connection
                .handle
                .send(ToServer::HousingSim(
                    HousingSimToServer::SyncScopeOverlays {
                        actor_id: connection.player_data.character.actor_id,
                        scope: HousingOverlayScope {
                            indoors: true,
                            ward_index: sel.ward_index,
                            plot_index: sel.plot_index,
                        },
                        objects,
                    },
                ))
                .await;
        }
        TerritoryIntendedUse::HousingOutdoor => {
            let Some(ctx) = connection.housing_sim.display_ward_context else {
                return;
            };
            let ward_estates = {
                let mut db = connection.database.lock();
                let conn = db.connection_mut();
                let estates = estates_in_ward(
                    conn,
                    ctx.territory_type_id,
                    connection.config.world_id,
                    ctx.ward_index,
                )
                .unwrap_or_default();
                estates
                    .into_iter()
                    .filter(|e| e.division as u8 == ctx.division)
                    .map(|e| {
                        let rows = list_placed_furniture_in_scope(conn, e.land_ident, false)
                            .unwrap_or_default();
                        (e, rows)
                    })
                    .collect::<Vec<_>>()
            };
            for (estate, rows) in ward_estates {
                let objects = build_overlay_objects_from_rows(
                    &rows,
                    false,
                    estate.ward_index as u8,
                    estate.plot_index as u8,
                );
                connection
                    .handle
                    .send(ToServer::HousingSim(
                        HousingSimToServer::SyncScopeOverlays {
                            actor_id: connection.player_data.character.actor_id,
                            scope: HousingOverlayScope {
                                indoors: false,
                                ward_index: estate.ward_index as u8,
                                plot_index: estate.plot_index as u8,
                            },
                            objects,
                        },
                    ))
                    .await;
            }
        }
        _ => {}
    }
}

/// Resolves the indoor `TerritoryType` ID for a house or apartment row.
pub fn target_indoor_territory_for_estate(estate: &HousingEstateRow) -> u16 {
    if estate.is_apartment {
        return apartment_territory_for_district(estate.territory_type_id as u16).unwrap_or(609);
    }
    let interior = StoredHouseInterior::from_json(&estate.interior_json);
    if interior.renovation_row_id != 0
        && let Some(zone_id) = renovation_territory_for_row(interior.renovation_row_id)
    {
        return zone_id;
    }
    let size = PlotSize::from_repr(estate.plot_size.clamp(0, 2) as u8).unwrap_or_default();
    let default_row =
        default_renovation_row_for_district_and_size(estate.territory_type_id as u16, size);
    renovation_territory_for_row(default_row).unwrap_or(344)
}

/// Warps the player into `estate`'s interior (preserving optional coordinates for seamless live reloads).
pub async fn warp_into_estate_interior(
    connection: &mut ZoneConnection,
    estate: &HousingEstateRow,
    preserve_pos: Option<Position>,
    preserve_rot: Option<f32>,
) {
    let target_zone = target_indoor_territory_for_estate(estate);
    let house_id = HouseId::from(estate.house_id as u64);
    connection.housing_sim.active_estate = Some(build_selection_from_row(estate, true));
    connection
        .handle
        .send(ToServer::HousingSim(
            HousingSimToServer::WarpToHouseInterior {
                client_id: connection.id,
                actor_id: connection.player_data.character.actor_id,
                house_id,
                territory_type_id: target_zone,
                preserve_pos,
                preserve_rot,
            },
        ))
        .await;
}

/// Warps the player out to the Apartment Building entrance in `(territory_type_id, ward_index, division)`.
pub async fn warp_to_apartment_outdoor(
    connection: &mut ZoneConnection,
    territory_type_id: u16,
    ward_index: u8,
    division: u8,
) {
    let div = division.clamp(0, 1);
    connection.entering_subdivision = div != 0;
    connection
        .handle
        .send(ToServer::HousingSim(
            HousingSimToServer::WarpToApartmentOutdoor {
                client_id: connection.id,
                actor_id: connection.player_data.character.actor_id,
                territory_type_id,
                ward_index,
                division: div,
            },
        ))
        .await;
}

/// Warps the player out to `estate`'s outdoor ward plot (or Apartment Building entrance when `is_apartment` is true).
pub async fn warp_to_estate_outdoor(
    connection: &mut ZoneConnection,
    estate: &HousingEstateRow,
    override_pos: Option<Position>,
    override_rot: Option<f32>,
) {
    let division = estate.division.clamp(0, 1) as u8;
    if estate.is_apartment && override_pos.is_none() {
        warp_to_apartment_outdoor(
            connection,
            estate.territory_type_id as u16,
            estate.ward_index as u8,
            division,
        )
        .await;
        return;
    }
    connection.entering_subdivision = division != 0;
    connection
        .handle
        .send(ToServer::HousingSim(
            HousingSimToServer::WarpToHousingPlot {
                client_id: connection.id,
                actor_id: connection.player_data.character.actor_id,
                territory_type_id: estate.territory_type_id as u16,
                ward_index: estate.ward_index as u8,
                division,
                plot_index: estate.plot_index as u8,
                override_pos,
                override_rot,
            },
        ))
        .await;
}

/// Executes live reload after DB mutations (HTTP `/preset/apply`, `/undo`, `!housing reload`, or pattern change).
pub async fn execute_housing_reload(
    connection: &mut ZoneConnection,
    land_ident: Option<i32>,
    mode: ReloadMode,
    preserve_player_pos: bool,
    force_enter: bool,
) {
    if mode == ReloadMode::None {
        return;
    }

    let current_zone_id = connection.player_data.volatile.zone_id as u16;
    let currently_indoors =
        is_house_interior_territory(current_zone_id) || is_apartment_territory(current_zone_id);

    let target_ident = land_ident.or_else(|| {
        connection
            .housing_sim
            .active_estate
            .as_ref()
            .map(|a| a.land_ident)
    });
    let Some(ident) = target_ident else {
        return;
    };

    let estate = {
        let mut db = connection.database.lock();
        estate_by_land_ident(db.connection_mut(), ident)
            .ok()
            .flatten()
    };
    let Some(estate) = estate else {
        return;
    };

    if currently_indoors || force_enter {
        let target_zone = target_indoor_territory_for_estate(&estate);
        let same_territory = currently_indoors && current_zone_id == target_zone;
        let same_estate = connection
            .housing_sim
            .active_estate
            .as_ref()
            .is_some_and(|a| a.land_ident == estate.land_ident);

        if !same_estate && !force_enter {
            tracing::debug!(
                land_ident = ident,
                zone_id = current_zone_id,
                "Skipping indoor housing reload for non-active estate"
            );
            return;
        }

        if mode == ReloadMode::LivePackets && same_territory && same_estate {
            push_indoor_live_refresh(connection, &estate).await;
            return;
        }

        let (pos, rot) = if preserve_player_pos && same_territory && same_estate {
            (
                Some(connection.player_data.volatile.position),
                Some(connection.player_data.volatile.rotation as f32),
            )
        } else {
            (None, None)
        };
        warp_into_estate_interior(connection, &estate, pos, rot).await;
        return;
    }

    // Player is standing outdoors in residential ward.
    let in_same_ward = current_zone_id == estate.territory_type_id as u16
        && connection
            .housing_sim
            .display_ward_context
            .is_some_and(|ctx| ctx.ward_index == estate.ward_index as u8);

    if !in_same_ward && !force_enter {
        tracing::debug!(
            land_ident = ident,
            zone_id = current_zone_id,
            "Skipping outdoor housing reload outside target ward"
        );
        return;
    }

    if in_same_ward && mode != ReloadMode::ZoneReload && !estate.is_apartment {
        // Push live outdoor appearance (476), FurnitureList, and interactive overlays with zero loading screen!
        let stored_ext = StoredHouseExterior::from_json(&estate.exterior_json);
        let ext_pkt = stored_ext.to_live_update_packet(&estate);
        connection
            .send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::HousingExteriorAppearanceUpdate(ext_pkt),
            ))
            .await;

        let placed = {
            let mut db = connection.database.lock();
            list_placed_furniture_in_scope(db.connection_mut(), estate.land_ident, false)
                .unwrap_or_default()
        };
        let flist = build_outdoor_furniture_list(&estate, &placed);
        connection
            .send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::FurnitureList(
                flist,
            )))
            .await;
        rebuild_house_inventory_from_db(connection, &estate, false);
        sync_current_overlays_for_connection(connection, TerritoryIntendedUse::HousingOutdoor)
            .await;
    } else if !estate.is_apartment {
        let same_ward_pos = if in_same_ward {
            Some(connection.player_data.volatile.position)
        } else {
            None
        };
        let same_ward_rot = if in_same_ward {
            Some(connection.player_data.volatile.rotation as f32)
        } else {
            None
        };
        warp_to_estate_outdoor(connection, &estate, same_ward_pos, same_ward_rot).await;
    }
}

/// Pushes zero-loading-screen live packets for fixture/lighting/material/stain/slot refreshes when inside the same indoor shell.
pub async fn push_indoor_live_refresh(connection: &mut ZoneConnection, estate: &HousingEstateRow) {
    let plot_size = PlotSize::from_repr(estate.plot_size.clamp(0, 2) as u8).unwrap_or_default();
    let interior = StoredHouseInterior::from_json(&estate.interior_json);
    let details = interior.to_packet(
        estate.light_level.clamp(0, 5) as u8,
        estate.enable_ssao,
        plot_size,
        estate.is_apartment,
    );
    connection
        .send_ipc_self(ServerZoneIpcSegment::new(
            ServerZoneIpcData::HousingInteriorDetails(details),
        ))
        .await;
    connection
        .actor_control_self(ActorControlCategory::InteriorLightLevel {
            unk1: 1,
            level: estate.light_level.clamp(0, 5) as u32,
            unk2: u32::from(estate.enable_ssao),
        })
        .await;

    let placed = {
        let mut db = connection.database.lock();
        list_placed_furniture_in_scope(db.connection_mut(), estate.land_ident, true)
            .unwrap_or_default()
    };

    for row in &placed {
        let Some(ctype) = super::containers::container_from_i32(row.container_type) else {
            continue;
        };
        if row.material_item_key > 0 {
            connection
                .send_ipc_self(ServerZoneIpcSegment::new(
                    ServerZoneIpcData::IndoorFurnitureMaterialUpdated(
                        IndoorFurnitureMaterialUpdated {
                            material_item_key: row.material_item_key as u16,
                            container_id: ctype,
                            slot: row.slot as u8,
                            pad0: [0; 3],
                        },
                    ),
                ))
                .await;
        }
    }

    send_indoor_object_data_value_sets(connection, &placed).await;
    rebuild_house_inventory_from_db(connection, estate, true);
    sync_current_overlays_for_connection(connection, TerritoryIntendedUse::HousingIndoor).await;
}

/// Applies a ReMakePlace preset file or JSON struct onto an estate and returns a human/HTTP summary.
pub fn apply_preset_file_to_estate(
    database: &parking_lot::Mutex<WorldDatabase>,
    game_data: &parking_lot::Mutex<GameData>,
    land_ident: i32,
    path: &Path,
    scope: PresetScope,
    actor_content_id: u64,
    snapshot_label: &str,
) -> Result<(super::remake_place::PreparedReMakePlaceApply, Option<i32>), String> {
    let layout = super::remake_place::read_layout_file(path)?;
    apply_preset_layout_to_estate(
        database,
        game_data,
        land_ident,
        &layout,
        scope,
        actor_content_id,
        snapshot_label,
    )
}

pub fn apply_preset_layout_to_estate(
    database: &parking_lot::Mutex<WorldDatabase>,
    game_data: &parking_lot::Mutex<GameData>,
    land_ident: i32,
    layout: &super::remake_place::ReMakePlaceLayout,
    scope: PresetScope,
    actor_content_id: u64,
    snapshot_label: &str,
) -> Result<(super::remake_place::PreparedReMakePlaceApply, Option<i32>), String> {
    let catalog = {
        let mut gd = game_data.lock();
        cached_housing_catalog(&mut gd)
    };
    apply_preset_layout_with_catalog(
        database,
        &catalog,
        land_ident,
        layout,
        scope,
        actor_content_id,
        snapshot_label,
    )
}

pub fn apply_preset_layout_with_catalog(
    database: &parking_lot::Mutex<WorldDatabase>,
    catalog: &HousingCatalog,
    land_ident: i32,
    layout: &super::remake_place::ReMakePlaceLayout,
    scope: PresetScope,
    actor_content_id: u64,
    snapshot_label: &str,
) -> Result<(super::remake_place::PreparedReMakePlaceApply, Option<i32>), String> {
    let mut db = database.lock();
    let conn = db.connection_mut();
    let estate = estate_by_land_ident(conn, land_ident)
        .map_err(|e| format!("Database error loading estate {land_ident}: {e}"))?
        .ok_or_else(|| format!("Housing estate land_ident={land_ident} not found"))?;

    let snap_id = super::db::create_snapshot(conn, land_ident, snapshot_label).ok();
    let prepared = super::remake_place::prepare_apply_layout(
        &estate,
        layout,
        catalog,
        scope,
        actor_content_id,
    );

    save_estate(conn, &prepared.updated_estate)
        .map_err(|e| format!("Failed to update housing_estates: {e}"))?;
    super::db::replace_placed_furniture_for_scope(
        conn,
        land_ident,
        prepared.replace_indoor,
        prepared.replace_outdoor,
        &prepared.furniture_rows,
    )
    .map_err(|e| format!("Failed to replace housing_furniture: {e}"))?;

    Ok((prepared, snap_id))
}

/// Sends `HousingWardInfo` (`624`) when player opens the ward residential directory.
pub async fn send_housing_ward_info(connection: &mut ZoneConnection, zone_id: u16, ward_index: u8) {
    let world_id = connection.config.world_id;
    let estates = {
        let mut db = connection.database.lock();
        estates_in_ward(db.connection_mut(), zone_id, world_id, ward_index).unwrap_or_default()
    };

    let land_sets = if let Some(row) = internal_housing_row(zone_id) {
        let mut gd = connection.game_data.lock();
        gd.get_land_sets(row)
    } else {
        None
    };

    let mut house_summaries = vec![HousingWardSummaryItem::default(); 60];
    for (i, item) in house_summaries.iter_mut().enumerate() {
        let price = land_sets
            .as_ref()
            .and_then(|ls| ls.get(i))
            .map(|e| e.InitialPrice)
            .unwrap_or(3_000_000);
        item.plot_price = price;
    }

    for estate in estates {
        let raw_idx = (estate.division as usize) * 30 + (estate.plot_index as usize);
        if let Some(summary) = house_summaries.get_mut(raw_idx) {
            summary.plot_price = estate.price.max(0) as u32;
            summary.flags = ward_summary_flags_for_estate(&estate);
            summary.tags = appeal_tags_from_row(&estate);
            summary.name = if estate.estate_name.is_empty() {
                estate.owner_name.clone()
            } else {
                estate.estate_name.clone()
            };
        }
    }

    connection
        .send_ipc_self(ServerZoneIpcSegment::new(
            ServerZoneIpcData::HousingWardInfo(HousingWardInfo {
                id: HouseId {
                    ward_index,
                    territory_type_id: zone_id,
                    world_id,
                    ..Default::default()
                },
                house_summaries,
                purchase_type: PurchaseType::FCFS,
                unk1: 0,
                tenant_type: TenantType::Any,
                terminator: 0,
            }),
        ))
        .await;
}

/// Sends vacant/occupied placard details (`HousingVacantLandInfo` / `HousingOccupiedLandInfo`) and promotes active selection if owner.
pub async fn send_housing_placard_info(
    connection: &mut ZoneConnection,
    zone_id: u16,
    ward_index: u8,
    division: u8,
    plot_index: u8,
) {
    let world_id = connection.config.world_id;
    let estate = {
        let mut db = connection.database.lock();
        estate_by_location(
            db.connection_mut(),
            zone_id,
            world_id,
            ward_index,
            division,
            plot_index,
        )
        .ok()
        .flatten()
    };

    if let Some(ref est) = estate {
        let content_id = connection.player_data.character.content_id as u64;
        if est.owner_content_id == 0 || est.owner_content_id == content_id as i64 {
            connection.housing_sim.authoritative_ward_context = Some(WardContext {
                territory_type_id: zone_id,
                ward_index,
                division,
            });
            connection.housing_sim.active_estate = Some(build_selection_from_row(est, false));
            connection.housing_sim.is_owner_authorized = true;
            rebuild_house_inventory_from_db(connection, est, false);
            send_owned_housing(connection).await;
        }

        let ext = StoredHouseExterior::from_json(&est.exterior_json).to_packet_exterior();
        let house_icon = if ext.is_under_construction() {
            2
        } else if (est.flags & (HousingFlag::OPEN.bits() as i32)) != 0 {
            1
        } else {
            0
        };

        connection
            .send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::HousingVacantLandInfo(HousingVacantLandInfo {
                    purchase_type: PurchaseType::Unavailable,
                    tenant_type: TenantType::Any,
                    availability_type: AvailabilityType::Unavailable,
                    ..Default::default()
                }),
            ))
            .await;

        connection
            .send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::HousingOccupiedLandInfo(HousingOccupiedLandInfo {
                    id: HouseId::from(est.house_id as u64),
                    owner_id: est.owner_content_id.max(0) as u64,
                    unk1: 0,
                    house_icon,
                    house_size: PlotSize::from_repr(est.plot_size.clamp(0, 2) as u8)
                        .unwrap_or_default(),
                    unk2: 0,
                    estate_name: clamp_utf8(&est.estate_name, MAX_ESTATE_NAME_BYTES),
                    unk3: 0,
                    estate_greeting: clamp_utf8(&est.greeting, MAX_ESTATE_GREETING_BYTES),
                    owner_name: est.owner_name.clone(),
                    fc_tag: String::new(),
                    unk4: 0,
                    tags: appeal_tags_from_row(est),
                }),
            ))
            .await;
    } else {
        connection
            .send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::HousingVacantLandInfo(HousingVacantLandInfo {
                    purchase_type: PurchaseType::FCFS,
                    tenant_type: TenantType::Any,
                    availability_type: AvailabilityType::Available,
                    ..Default::default()
                }),
            ))
            .await;
    }

    let _ = (
        &delete_furniture_slot,
        MAX_EXTERIOR_PLACED_SLOTS,
        MAX_INDOOR_PLACED_SLOTS,
    );
    let _ = containers_for_request;
    let _ = HousingEstateGreeting::default();
    let _ = WarpType::Normal;
    let _ = placed_container_for_flat_slot;
    let _ = indoor_slot_capacity;
    let _ = nearest_plot_to_position;
    let _ = HousingSseEvent {
        kind: String::new(),
        land_ident: None,
        message: String::new(),
        session: None,
    };
}

/// Sends real apartment residents list from `housing_estates`.
pub async fn send_apartment_list_from_db(connection: &mut ZoneConnection, starting_index: u32) {
    let ctx = connection
        .housing_sim
        .display_ward_context
        .unwrap_or(WardContext {
            territory_type_id: default_district_for_zone(
                connection.player_data.volatile.zone_id as u16,
            ),
            ward_index: 0,
            division: 0,
        });

    let apts = {
        let mut db = connection.database.lock();
        apartments_in_ward(
            db.connection_mut(),
            ctx.territory_type_id,
            connection.config.world_id,
            ctx.ward_index,
            ctx.division,
        )
        .unwrap_or_default()
    };

    let mut entries = build_apartment_entries(&apts, starting_index);
    entries.resize_with(15, Default::default);

    connection
        .send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::ApartmentList(
            ApartmentList {
                content_id: connection.player_data.character.content_id as u64,
                flags: 0x80 | if ctx.division != 0 { 1 } else { 0 },
                ward_id: ctx.ward_index as u16,
                zone_id: ctx.territory_type_id,
                world_id: connection.config.world_id,
                list_index: starting_index.max(1),
                apartments: entries,
            },
        )))
        .await;
}

/// Applies full simulator unlock permissions and unrestricted access onto `PlayerData`.
/// Returns `true` if the character was in an intro zone (`181..=183`) or uninitialized origin and was moved to Limsa Lominsa Upper Decks (`128`).
pub fn apply_simulator_unlock_to_player_data(
    player_data: &mut PlayerData,
    exp_index: Option<i8>,
) -> bool {
    let had_cutscene_2 = player_data.unlock.cutscene_seen.contains(2);

    player_data.character.gm_rank = GameMasterRank::Debug;

    player_data.unlock.unlocks.set_all();
    player_data.unlock.seen_active_help.set_all();
    player_data.unlock.minions.set_all();
    player_data.unlock.mounts.set_all();
    player_data.unlock.orchestrion_rolls.set_all();
    player_data.unlock.cutscene_seen.set_all();
    player_data.unlock.ornaments.set_all();
    player_data.unlock.caught_fish.set_all();
    player_data.unlock.caught_spearfish.set_all();
    player_data.unlock.adventures.set_all();
    player_data.unlock.triple_triad_cards.set_all();
    player_data.unlock.glasses_styles.set_all();
    player_data.unlock.chocobo_taxi_stands.set_all();
    player_data.unlock.titles.set_all();
    player_data.companion.unlocked_equip.set_all();

    for id in 1..239 {
        player_data.aetheryte.unlocked.set(id);
    }
    player_data.aether_current.unlocked.set_all();
    player_data.aether_current.comp_flg_set.set_all();

    player_data.content.unlocked_special_content.set_all();
    player_data.content.unlocked_raids.set_all();
    player_data.content.unlocked_dungeons.set_all();
    player_data.content.unlocked_guildhests.set_all();
    player_data.content.unlocked_trials.set_all();
    player_data.content.unlocked_crystalline_conflicts.set_all();
    player_data.content.unlocked_frontlines.set_all();
    player_data.content.cleared_raids.set_all();
    player_data.content.cleared_dungeons.set_all();
    player_data.content.cleared_guildhests.set_all();
    player_data.content.cleared_trials.set_all();
    player_data.content.cleared_crystalline_conflicts.set_all();
    player_data.content.cleared_frontlines.set_all();
    player_data.content.cleared_masked_carnivale.set_all();
    player_data.content.unlocked_misc_content.set_all();
    player_data.content.cleared_misc_content.set_all();

    player_data.quest.completed.data = vec![0xFF; COMPLETED_QUEST_BITMASK_SIZE];
    player_data.quest.completed_legacy.set_all();
    player_data.quest.unlocked_map_markers.set_all();
    player_data.quest.completed_levequests.set_all();

    player_data.mentor.is_novice = 0;

    if player_data.grand_company.active_company == IpcGrandCompany::None {
        player_data.grand_company.active_company = IpcGrandCompany::Maelstrom;
    }
    if player_data.grand_company.company_ranks.0.len() < 3 {
        player_data.grand_company.company_ranks.0.resize(3, 11);
    }
    for rank in &mut player_data.grand_company.company_ranks.0 {
        *rank = (*rank).max(11);
    }

    if let Some(idx) = exp_index
        && idx >= 0
        && let Some(slot) = player_data.classjob.levels.0.get_mut(idx as usize)
    {
        *slot = (*slot).max(100);
    }
    for level in &mut player_data.classjob.levels.0 {
        if *level > 0 {
            *level = (*level).max(100);
        }
    }

    let gil = player_data.inventory.currency.get_slot_mut(0);
    gil.item_id = CurrencyKind::Gil as u32;
    gil.quantity = gil.quantity.max(99_999_999);

    let in_intro = matches!(player_data.volatile.zone_id, 181..=183)
        || (!had_cutscene_2 && player_data.volatile.position.0 == glam::Vec3A::ZERO);
    if in_intro {
        player_data.volatile.zone_id = 128;
        player_data.volatile.position = Position(glam::Vec3A::new(0.0, 40.0, 0.0));
        player_data.volatile.rotation = 0.0;
    }
    in_intro
}

/// Persists full simulator unlock across one or all characters in `world.db`.
pub fn unlock_characters_in_database(
    db: &mut WorldDatabase,
    game_data: Option<&parking_lot::Mutex<GameData>>,
    filter_content_id: Option<u64>,
) -> Result<Vec<String>, diesel::result::Error> {
    let characters = db.find_character_rows(filter_content_id)?;
    let mut unlocked_names = Vec::with_capacity(characters.len());

    for character in characters {
        let mut pdata = db.load_player_data_raw(character)?;
        let exp_index = game_data.and_then(|gd| {
            gd.lock()
                .get_exp_array_index(pdata.classjob.current_class as u16)
        });
        apply_simulator_unlock_to_player_data(&mut pdata, exp_index);
        let name = pdata.character.name.clone();
        db.commit_player_data(&pdata);
        unlocked_names.push(name);
    }

    Ok(unlocked_names)
}

/// Applies full simulator unlock to a live connected `ZoneConnection`, persists to DB, and sends immediate packets.
pub async fn unlock_online_connection(connection: &mut ZoneConnection) {
    let current_class = connection.player_data.classjob.current_class as u16;
    let current_zone_id = connection.player_data.volatile.zone_id;
    let exp_index = connection
        .game_data
        .lock()
        .get_exp_array_index(current_class);
    let was_in_intro =
        apply_simulator_unlock_to_player_data(&mut connection.player_data, exp_index);

    {
        let mut db = connection.database.lock();
        db.commit_player_data(&connection.player_data);
    }

    connection
        .actor_control_self(ActorControlCategory::ToggleCutsceneSeen {
            id: 2,
            unlocked: true,
        })
        .await;

    for id in 1..239 {
        connection
            .actor_control_self(ActorControlCategory::LearnTeleport { id, unlocked: true })
            .await;
    }

    connection.send_quest_information().await;
    connection.send_grand_company_info().await;
    connection
        .actor_control_self(ActorControlCategory::SetLevel {
            classjob_id: current_class as u32,
            level: 100,
        })
        .await;
    connection.update_class_info().await;
    connection.send_stats().await;
    connection.send_inventory().await;

    if was_in_intro || matches!(current_zone_id, 181..=183) {
        connection
            .change_zone(
                128,
                Some(Position(glam::Vec3A::new(0.0, 40.0, 0.0))),
                Some(0.0),
                WarpType::Normal,
                0,
            )
            .await;
    }

    connection
        .send_notice(
            "[Housing] Full character unlock applied (GM Debug, Lv100, GC Rank 11, 99,999,999 Gil, Teleports, Quests, Orchestrion). Collection masks refresh fully after relog if you were already logged in.",
        )
        .await;
}

/// Grants a sample indoor/outdoor/NPC furniture kit into the player's inventory and commits to SQLite.
pub async fn give_sample_kit_to_connection(connection: &mut ZoneConnection, kind: &str) -> usize {
    let catalog = connection_catalog(connection);
    let items = catalog.sample_kit_item_ids(kind);
    if items.is_empty() {
        return 0;
    }

    let mut added = 0usize;
    for item_id in items {
        let new_item = {
            let mut gd = connection.game_data.lock();
            gd.get_item_info(ItemInfoQuery::ById(item_id))
                .map(|info| Item::new(&info, 1))
                .unwrap_or(Item {
                    item_id,
                    quantity: 1,
                    ..Default::default()
                })
        };

        let mut inserted = false;
        for page_idx in 0..4 {
            for slot_idx in 0..35u16 {
                let ctype = match page_idx {
                    0 => ContainerType::Inventory0,
                    1 => ContainerType::Inventory1,
                    2 => ContainerType::Inventory2,
                    _ => ContainerType::Inventory3,
                };
                if let Some(slot) = connection.player_data.inventory.get_item(ctype, slot_idx)
                    && slot.item_id == 0
                {
                    connection
                        .player_data
                        .inventory
                        .add_in_slot(new_item, &ctype, slot_idx);
                    inserted = true;
                    added += 1;
                    break;
                }
            }
            if inserted {
                break;
            }
        }
    }

    if added > 0 {
        {
            let mut db = connection.database.lock();
            db.commit_classjob_and_inventory(&connection.player_data);
        }
        connection.send_inventory().await;
    }

    added
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CharaMake;
    use crate::inventory::Inventory as PlayerInventory;
    use physis::race::{Race, Tribe};
    use physis::savedata::chardat::CustomizeData;

    #[test]
    fn unlock_characters_in_database_sets_debug_bitmasks_gil_gc_level_and_rescues_intro_zone() {
        let Some(game_data) = crate::test_game_data() else {
            return;
        };
        let mut db = WorldDatabase::new_in_memory();
        let first_cid = db
            .create_player_data(
                1,
                "FirstHero",
                &CharaMake {
                    customize: CustomizeData {
                        race: Race::Hyur,
                        tribe: Tribe::Highlander,
                        ..Default::default()
                    },
                    classjob_id: 1,
                    ..Default::default()
                }
                .to_json(),
                1,
                128,
                PlayerInventory::default(),
                game_data,
            )
            .0;
        let second_cid = db
            .create_player_data(
                1,
                "SecondHero",
                &CharaMake {
                    customize: CustomizeData {
                        race: Race::Hyur,
                        tribe: Tribe::Highlander,
                        ..Default::default()
                    },
                    classjob_id: 1,
                    ..Default::default()
                }
                .to_json(),
                1,
                181,
                PlayerInventory::default(),
                game_data,
            )
            .0;

        let gd_mutex = parking_lot::Mutex::new(game_data.clone());
        let names = unlock_characters_in_database(&mut db, Some(&gd_mutex), None)
            .expect("unlock should succeed");
        assert_eq!(names.len(), 2);

        let second_row = db
            .find_character(Some(second_cid), None)
            .expect("second character must exist");
        let second_pdata = db
            .load_player_data_raw(second_row)
            .expect("raw player data must load");
        assert_eq!(second_pdata.character.gm_rank, GameMasterRank::Debug);
        assert!(second_pdata.unlock.cutscene_seen.contains(2));
        assert!(
            second_pdata
                .unlock
                .orchestrion_rolls
                .data
                .iter()
                .all(|b| *b == 0xFF)
        );
        assert!(second_pdata.unlock.unlocks.data.iter().all(|b| *b == 0xFF));
        assert!(second_pdata.aetheryte.unlocked.contains(8));
        assert_eq!(
            second_pdata.grand_company.active_company,
            IpcGrandCompany::Maelstrom
        );
        assert_eq!(second_pdata.grand_company.company_ranks.0, vec![11, 11, 11]);
        assert_eq!(second_pdata.inventory.currency.gil.quantity, 99_999_999);
        assert_eq!(second_pdata.volatile.zone_id, 128);
        assert_eq!(second_pdata.volatile.position.0.y, 40.0);
        let exp_idx = game_data
            .get_exp_array_index(second_pdata.classjob.current_class as u16)
            .unwrap();
        assert_eq!(second_pdata.classjob.levels.0[exp_idx as usize], 100);

        let first_row = db
            .find_character(Some(first_cid), None)
            .expect("first character must exist");
        let first_pdata = db
            .load_player_data_raw(first_row)
            .expect("first raw player data must load");
        assert_eq!(first_pdata.inventory.currency.gil.quantity, 99_999_999);
    }
}
