use std::collections::BTreeMap;
use std::sync::Arc;

use kawari::common::{
    ContainerType, HandlerType, HouseId, HousingFlag, LogMessageType, ObjectId, Position,
    TerritoryIntendedUse, WarpType,
};
use kawari::ipc::zone::{
    ActorControlCategory, BatchIndoorFurnitureTranslated, BatchTranslateIndoorFurniture,
    BatchTranslatedFurnitureEntry, BuildEstateDeed, ClientTriggerCommand, ClientZoneIpcData,
    DyeInformation, ExteriorRemodelItemOperation, FurnitureTranslatedForObserver,
    HousingExteriorAppearanceUpdate, HousingObjectDataValueSet, HousingStainUpdated,
    InteriorRemodelItemOperation, ItemInfo, PlotSize, ServerZoneIpcData, ServerZoneIpcSegment,
    UpdateAquarium, UpdateEstateGreeting, UpdateEstateName, UpdateEstateProfile,
    UpdateOrchestrionPlaylist,
};
use parking_lot::Mutex;

use crate::common::{FromServer, ToServer};
use crate::inventory::{DesiredHousingInventoryPages, Item};
use crate::lua::{KawariLua, LuaPlayer, LuaTask};
use crate::server::network::{DestinationNetwork, NetworkState};
use crate::server::zone::{begin_change_zone, do_change_zone, pick_point_in_pop_range};
use crate::server::{InstanceQuery, WorldServer};
use crate::{Event, EventHandler, GameData, ZoneConnection};

use super::commands::try_handle_housing_chat_command;
use super::containers::{
    EXTERIOR_STOREROOM_CONTAINERS, INTERIOR_STOREROOM_CONTAINERS, container_from_i32,
    container_from_u16, flat_slot_for_placed_container, is_appearance_container,
    is_housing_container, is_interior_placed_container, is_storeroom_container,
    placed_container_for_flat_slot, storeroom_slots_per_page,
};
use super::db::{
    AquariumObjectState, FurnitureObjectData, HousingEstateRow, HousingFurnitureRow,
    MAX_ESTATE_GREETING_BYTES, MAX_ESTATE_NAME_BYTES, apartments_in_ward, clamp_utf8,
    create_snapshot, delete_furniture_slot, estate_by_land_ident, estate_by_location,
    list_placed_furniture_in_scope, move_furniture_row, save_estate, update_furniture_object_data,
    update_furniture_position, update_furniture_stain, upsert_furniture,
};
use super::fixtures::{
    StoredHouseExterior, StoredHouseInterior, apply_exterior_remodel_op, apply_interior_remodel_op,
    build_housing_object_data_value_sets, raw_plot_index,
};
use super::flows::{
    build_selection_from_row, complete_deferred_indoor_tail_on_remodel, connection_catalog,
    ensure_local_apartment_estate, ensure_local_plot_estate, execute_housing_reload,
    execute_housing_reload_with_source, give_sample_kit_to_connection, mutate_synced_indoor_slot,
    patch_synced_indoor_meta, patch_synced_indoor_slot, publish_session_snapshot,
    push_indoor_live_refresh, rebuild_house_inventory_from_db, remove_synced_indoor_slot,
    send_apartment_list_from_db, send_housing_placard_info, send_housing_ward_info,
    send_owned_housing, sync_current_overlays_for_connection, target_indoor_territory_for_estate,
    unlock_online_connection, warp_into_estate_interior, warp_to_apartment_outdoor,
    warp_to_estate_outdoor,
};
use super::gamedata_ext::cached_housing_catalog;
use super::overlays::{
    HousingOverlayKey, HousingOverlayObject, remove_single_overlay, replace_scope_overlays,
    translate_single_overlay, upsert_single_overlay,
};
use super::plots::{
    city_fallback_aetheryte_for_territory, default_district_for_zone, indoor_entry_transform,
    is_apartment_lobby_territory, is_apartment_territory, is_house_interior_territory,
    nearest_plot_to_position, nudge_plot_entrance_exit, plot_spec, wrap_angle_radians,
};
use super::session::{
    HousingBridgeDelta, HousingBridgeMoveItem, HousingSimFromServer, HousingSimToServer,
    HousingSseEvent, ReloadMode,
};

/// Intercepts housing-related client zone IPC packets before upstream default match arms.
/// Returns `true` if handled completely; returns `false` to let upstream continue normal handling.
pub async fn try_handle_client_ipc(
    connection: &mut ZoneConnection,
    lua_player: &mut LuaPlayer,
    _events: &mut Vec<(Box<dyn EventHandler>, Event)>,
    ipc: &ClientZoneIpcData,
) -> bool {
    match ipc {
        ClientZoneIpcData::FinishLoading { .. } => {
            let zone_id = connection.player_data.volatile.zone_id as u16;
            let has_deferred_tail = connection
                .housing_sim
                .pending_indoor_furniture_list_tail
                .is_some();
            if has_deferred_tail
                || is_house_interior_territory(zone_id)
                || is_apartment_territory(zone_id)
            {
                tracing::info!(
                    zone_id,
                    has_deferred_tail,
                    pending_overlay_sync = connection.housing_sim.pending_indoor_overlay_sync,
                    "Client sent FinishLoading"
                );
            }
            if has_deferred_tail {
                connection.housing_sim.pending_indoor_finish_loading = true;
                connection.housing_sim.pending_indoor_finish_loading_since =
                    Some(tokio::time::Instant::now());
                return true;
            }
            if connection.housing_sim.pending_indoor_overlay_sync
                && (is_house_interior_territory(zone_id) || is_apartment_territory(zone_id))
            {
                sync_current_overlays_for_connection(
                    connection,
                    TerritoryIntendedUse::HousingIndoor,
                )
                .await;
            } else if kawari::common::internal_housing_row(zone_id).is_some() {
                sync_current_overlays_for_connection(
                    connection,
                    TerritoryIntendedUse::HousingOutdoor,
                )
                .await;
            }
            false
        }
        ClientZoneIpcData::SendChatMessage(chat) => {
            try_handle_housing_chat_command(connection, &chat.message.to_string()).await
        }
        ClientZoneIpcData::StartTalkEvent {
            actor_id,
            handler_id,
        } => {
            let zone_id = connection.player_data.volatile.zone_id as u16;
            if kawari::common::internal_housing_row(zone_id).is_some() {
                let plot_match = lua_player
                    .zone_data
                    .cached_housing_entrance_plots
                    .get(&actor_id.object_id.0)
                    .map(|&raw| (raw / 30, raw % 30))
                    .or_else(|| {
                        let plots = &lua_player.zone_data.cached_housing_plots;
                        if plots.is_empty() {
                            return None;
                        }
                        let player_pos =
                            glam::Vec3::from(connection.player_data.volatile.position.0);
                        plots
                            .iter()
                            .enumerate()
                            .min_by(|(_, (a, _)), (_, (b, _))| {
                                let da = player_pos.distance_squared(glam::Vec3::from(a.0));
                                let db = player_pos.distance_squared(glam::Vec3::from(b.0));
                                da.total_cmp(&db)
                            })
                            .map(|(raw, _)| ((raw / 30) as u8, (raw % 30) as u8))
                    })
                    .or_else(|| {
                        nearest_plot_to_position(zone_id, connection.player_data.volatile.position)
                    });

                if let Some((division, plot_index)) = plot_match {
                    connection.housing_sim.last_entrance_plot = Some((division, plot_index));
                    connection.housing_sim.pending_placard_location = Some((division, plot_index));
                    if handler_id.handler_type() == HandlerType::CustomTalk
                        && handler_id.event_id() == 135
                    {
                        let ward = connection
                            .housing_sim
                            .display_ward_context
                            .map(|c| c.ward_index)
                            .unwrap_or(0);
                        send_housing_placard_info(connection, zone_id, ward, division, plot_index)
                            .await;
                    }
                }
            }
            false
        }
        ClientZoneIpcData::ClientTrigger(trigger) => {
            handle_client_trigger(connection, &trigger.trigger).await
        }
        ClientZoneIpcData::PlaceFurniture {
            plot_index: _,
            unk1: _,
            container,
            slot,
            position,
            rotation,
            spawn_furniture: _,
            to_storeroom,
            unk2: _,
        } => {
            handle_place_furniture(
                connection,
                *container,
                *slot,
                *position,
                *rotation,
                *to_storeroom != 0,
            )
            .await
        }
        ClientZoneIpcData::TranslateFurniture {
            house_id,
            slot,
            unk2: _,
            position,
            rotation,
            unk3: _,
        } => handle_translate_furniture(connection, *house_id, *slot, *position, *rotation).await,
        ClientZoneIpcData::BatchTranslateIndoorFurniture(batch) => {
            handle_batch_translate(connection, batch).await
        }
        ClientZoneIpcData::DyeInformation(info) => handle_housing_dye(connection, info).await,
        ClientZoneIpcData::InteriorRemodelItemOperation(op) => {
            handle_interior_remodel(connection, op).await
        }
        ClientZoneIpcData::ExteriorRemodelItemOperation(op) => {
            handle_exterior_remodel(connection, op).await
        }
        ClientZoneIpcData::UpdateEstateName(pkt) => handle_update_name(connection, pkt).await,
        ClientZoneIpcData::UpdateEstateGreeting(pkt) => {
            handle_update_greeting(connection, pkt).await
        }
        ClientZoneIpcData::UpdateEstateProfile(pkt) => handle_update_profile(connection, pkt).await,
        ClientZoneIpcData::BuildEstateDeed(pkt) => handle_build_deed(connection, pkt).await,
        ClientZoneIpcData::UpdateAquarium(pkt) => handle_update_aquarium(connection, pkt).await,
        ClientZoneIpcData::UpdateOrchestrionPlaylist(pkt) => {
            handle_update_orchestrion(connection, pkt).await
        }
        _ => false,
    }
}

async fn handle_client_trigger(
    connection: &mut ZoneConnection,
    cmd: &ClientTriggerCommand,
) -> bool {
    match cmd {
        ClientTriggerCommand::FinishZoning => {
            let zone_id = connection.player_data.volatile.zone_id as u16;
            let intended_use =
                if is_house_interior_territory(zone_id) || is_apartment_territory(zone_id) {
                    TerritoryIntendedUse::HousingIndoor
                } else if kawari::common::internal_housing_row(zone_id).is_some() {
                    TerritoryIntendedUse::HousingOutdoor
                } else {
                    TerritoryIntendedUse::Town
                };
            publish_session_snapshot(connection, intended_use);
            if intended_use == TerritoryIntendedUse::HousingOutdoor
                && let Some(reentry) = connection
                    .housing_sim
                    .pending_same_zone_indoor_reentry
                    .as_mut()
                && reentry.ready_at.is_none()
            {
                let current_ward = connection
                    .housing_sim
                    .display_ward_context
                    .map(|ctx| ctx.ward_index);
                if reentry.expected_transit_zone_id == zone_id
                    && current_ward == Some(reentry.expected_transit_ward_index)
                {
                    reentry.ready_at =
                        Some(tokio::time::Instant::now() + std::time::Duration::from_millis(250));
                    tracing::info!(
                        land_ident = reentry.land_ident,
                        transit_zone_id = zone_id,
                        ward_index = reentry.expected_transit_ward_index,
                        "Transit outdoor FinishZoning received; scheduling indoor re-entry"
                    );
                } else {
                    tracing::warn!(
                        land_ident = reentry.land_ident,
                        expected_zone = reentry.expected_transit_zone_id,
                        actual_zone = zone_id,
                        expected_ward = reentry.expected_transit_ward_index,
                        actual_ward = ?current_ward,
                        "Clearing mismatched pending_same_zone_indoor_reentry on FinishZoning"
                    );
                    connection.housing_sim.clear_pending_indoor_reentry();
                }
            }
            false
        }
        ClientTriggerCommand::RequestHousingInventory { storeroom } => {
            let zone_id = connection.player_data.volatile.zone_id as u16;
            let indoors = is_house_interior_territory(zone_id) || is_apartment_territory(zone_id);
            if let Some(ref active) = connection.housing_sim.active_estate.clone() {
                let est = {
                    let mut db = connection.database.lock();
                    estate_by_land_ident(db.connection_mut(), active.land_ident)
                        .ok()
                        .flatten()
                };
                if let Some(est) = est {
                    rebuild_house_inventory_from_db(connection, &est, indoors);
                }
            }
            let which = match (indoors, *storeroom) {
                (true, false) => DesiredHousingInventoryPages::Interior,
                (true, true) => DesiredHousingInventoryPages::InteriorStoreroom,
                (false, false) => DesiredHousingInventoryPages::Exterior,
                (false, true) => DesiredHousingInventoryPages::ExteriorStoreroom,
            };
            connection.send_housing_inventory(which).await;
            true
        }
        ClientTriggerCommand::RequestHousingWardInfo {
            zone_id,
            ward_index,
        } => {
            send_housing_ward_info(connection, *zone_id as u16, *ward_index as u8).await;
            true
        }
        ClientTriggerCommand::RequestHousingVacantPlotInfo {
            zone_id,
            plot_index,
            ward_index,
        }
        | ClientTriggerCommand::RequestHousingOccupiedPlotInfo {
            zone_id,
            plot_index,
            ward_index,
        } => {
            let (division, plot_in_div) = if *plot_index >= 30 {
                (1u8, plot_index - 30)
            } else {
                let div = connection
                    .housing_sim
                    .pending_placard_location
                    .map(|(d, _)| d)
                    .or_else(|| {
                        connection
                            .housing_sim
                            .display_ward_context
                            .map(|c| c.division)
                    })
                    .unwrap_or(0);
                (div, *plot_index)
            };
            send_housing_placard_info(
                connection,
                *zone_id as u16,
                *ward_index,
                division,
                plot_in_div,
            )
            .await;
            true
        }
        ClientTriggerCommand::HousingMoveToFrontDoor => {
            if let Some(est) = load_active_estate(connection) {
                warp_into_estate_interior(connection, &est, None, None).await;
            }
            true
        }
        ClientTriggerCommand::RequestApartmentList { starting_index } => {
            send_apartment_list_from_db(connection, *starting_index).await;
            true
        }
        ClientTriggerCommand::SetInteriorLightLevel { level, unk } => {
            let clamped = (*level as u8).min(5);
            let ssao = *unk != 0;
            if let Some(ref sel) = connection.housing_sim.active_estate.clone() {
                let updated = {
                    let mut db = connection.database.lock();
                    if let Some(mut est) = estate_by_land_ident(db.connection_mut(), sel.land_ident)
                        .ok()
                        .flatten()
                    {
                        est.light_level = clamped as i32;
                        est.enable_ssao = ssao;
                        let _ = save_estate(db.connection_mut(), &est);
                        Some(est)
                    } else {
                        None
                    }
                };
                if let Some(est) = updated {
                    connection
                        .actor_control_self(ActorControlCategory::InteriorLightLevel {
                            unk1: 0,
                            level: clamped as u32,
                            unk2: u32::from(ssao),
                        })
                        .await;
                    patch_synced_indoor_meta(connection, &est);
                    emit_game_delta(
                        connection,
                        "layout_changed",
                        est.land_ident,
                        format!("실내 조명 변경 ({clamped})"),
                        HousingBridgeDelta::LayoutChanged {
                            indoors: true,
                            reason: "light_level".to_string(),
                        },
                    );
                }
            }
            true
        }
        ClientTriggerCommand::RequestEstateInteriorRemodel {}
        | ClientTriggerCommand::InteriorRemodelMenuToggled {} => {
            connection.housing_sim.pending_remodel_container =
                Some(ContainerType::HousingInteriorAppearance);
            let appearance = connection.player_data.house_inventory.interior_appearance[0].clone();
            connection
                .send_container(&appearance, ContainerType::HousingInteriorAppearance)
                .await;
            connection
                .actor_control_self(ActorControlCategory::ShowEstateInternalAppearanceUI {})
                .await;
            true
        }
        ClientTriggerCommand::RequestEstateExteriorRemodel { plot_index } => {
            connection.housing_sim.pending_remodel_container =
                Some(ContainerType::HousingExteriorAppearance);
            let appearance = connection.player_data.house_inventory.exterior_appearance[0].clone();
            connection
                .send_container(&appearance, ContainerType::HousingExteriorAppearance)
                .await;
            connection
                .actor_control_self(ActorControlCategory::ShowEstateExternalAppearanceUI {
                    plot_index: *plot_index,
                })
                .await;
            true
        }
        ClientTriggerCommand::FinishEstateRemodel { mode } => {
            let zone_id = connection.player_data.volatile.zone_id as u16;
            tracing::info!(
                zone_id,
                mode,
                has_deferred_tail = connection
                    .housing_sim
                    .pending_indoor_furniture_list_tail
                    .is_some(),
                "Client sent FinishEstateRemodel"
            );
            if complete_deferred_indoor_tail_on_remodel(connection, *mode).await {
                return true;
            }
            connection.housing_sim.pending_remodel_container = None;
            connection
                .actor_control_self(ActorControlCategory::FinishEstateAppearanceRemodel {})
                .await;
            true
        }
        ClientTriggerCommand::RequestEstateInteriorPattern { mode } => {
            if let Some(est) = load_active_estate(connection) {
                let interior = StoredHouseInterior::from_json(&est.interior_json);
                connection
                    .actor_control_self(ActorControlCategory::ShowEstateInteriorPatternUI {
                        enabled: true,
                        current_renovation_row_id: interior.renovation_row_id as u32,
                        current_size: est.plot_size.clamp(0, 2) as u32,
                        remodel_mode: *mode,
                        allowed_size_mask: 0x7,
                    })
                    .await;
            }
            true
        }
        ClientTriggerCommand::RequestEstateInteriorPatternStatus { mode, unk1: _ } => {
            connection
                .actor_control_self(ActorControlCategory::EstateInteriorPatternStepAck {
                    mode: *mode,
                    result: 0,
                })
                .await;
            true
        }
        ClientTriggerCommand::ApplyEstateInteriorPattern {
            mode,
            renovation_row_id,
        } => {
            let row_id = *renovation_row_id as u16;
            if let Some(mut est) = load_active_estate(connection)
                && (1..=21).contains(&row_id)
            {
                {
                    let mut db = connection.database.lock();
                    let conn = db.connection_mut();
                    let _ = create_snapshot(
                        conn,
                        est.land_ident,
                        &format!("interior pattern {row_id}"),
                    );
                    let mut interior = StoredHouseInterior::from_json(&est.interior_json);
                    interior.renovation_row_id = row_id;
                    est.interior_json = interior.to_json();
                    let _ = save_estate(conn, &est);
                }
                connection
                    .actor_control_self(ActorControlCategory::EstateInteriorPatternStepAck {
                        mode: *mode,
                        result: 0,
                    })
                    .await;
                connection
                    .actor_control_self(ActorControlCategory::FinishEstateInteriorPattern {
                        enabled: false,
                    })
                    .await;
                execute_housing_reload(
                    connection,
                    Some(est.land_ident),
                    ReloadMode::ZoneReload,
                    false,
                    false,
                )
                .await;
                emit_game_delta(
                    connection,
                    "layout_changed",
                    est.land_ident,
                    format!("실내 양식 변경 (#{row_id})"),
                    HousingBridgeDelta::LayoutChanged {
                        indoors: true,
                        reason: "interior_pattern".to_string(),
                    },
                );
            }
            true
        }
        ClientTriggerCommand::FurnitureMenuToggled { closed: _ } => {
            connection.send_conditions().await;
            true
        }
        ClientTriggerCommand::PlaceFurnitureFromStoreroom {
            unk1: _,
            unk2: _,
            world_id: _,
            plot_index: _,
            ward_index: _,
            container_type,
            unk3: _,
            container_index,
            unk4: _,
        } => {
            handle_preview_furniture_from_storeroom(connection, *container_type, *container_index)
                .await
        }
        ClientTriggerCommand::MoveHousingItemToInventory {
            house_id: _,
            storage_id,
            unk1: _,
            slot,
            to_storeroom,
        } => handle_remove_or_store_furniture(connection, *storage_id, *slot, *to_storeroom).await,
        _ => false,
    }
}

fn load_active_estate(connection: &ZoneConnection) -> Option<HousingEstateRow> {
    let sel = connection.housing_sim.active_estate.as_ref()?;
    let mut db = connection.database.lock();
    estate_by_land_ident(db.connection_mut(), sel.land_ident)
        .ok()
        .flatten()
}

fn send_item_update_for(
    connection: &ZoneConnection,
    container: ContainerType,
    slot: u16,
    item: Item,
) -> ServerZoneIpcSegment {
    ServerZoneIpcSegment::new(ServerZoneIpcData::UpdateInventorySlot(ItemInfo {
        sequence: connection.player_data.item_sequence,
        slot,
        container,
        ..item.into()
    }))
}

fn decode_cstr(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn emit_game_delta(
    connection: &ZoneConnection,
    kind: &str,
    land_ident: i32,
    message: impl Into<String>,
    delta: HousingBridgeDelta,
) {
    if let Some(ref hub) = connection.housing_hub {
        hub.emit_event(
            HousingSseEvent::new(kind, Some(land_ident), message)
                .with_source(Some("game".to_string()))
                .with_delta(delta),
        );
    }
}

fn local_transform_for_estate(
    estate: &HousingEstateRow,
    indoors: bool,
    world_pos: Position,
    world_rot: f32,
) -> (Position, f32) {
    if indoors || estate.is_apartment {
        return (world_pos, world_rot);
    }
    if let Some(spec) = plot_spec(
        estate.territory_type_id as u16,
        estate.division as u8,
        estate.plot_index as u8,
    ) {
        spec.world_to_local(world_pos, world_rot)
    } else {
        (world_pos, world_rot)
    }
}

fn sanitize_transform(position: Position, rotation: f32) -> (Position, f32) {
    let x = if position.0.x.is_finite() {
        position.0.x
    } else {
        0.0
    };
    let y = if position.0.y.is_finite() {
        position.0.y
    } else {
        0.0
    };
    let z = if position.0.z.is_finite() {
        position.0.z
    } else {
        0.0
    };
    (
        Position(glam::Vec3A::new(x, y, z)),
        wrap_angle_radians(rotation),
    )
}

async fn handle_preview_furniture_from_storeroom(
    connection: &mut ZoneConnection,
    src_container: ContainerType,
    src_slot: u16,
) -> bool {
    let zone_id = connection.player_data.volatile.zone_id as u16;
    let indoors = is_house_interior_territory(zone_id) || is_apartment_territory(zone_id);
    let Some(estate) = load_active_estate(connection) else {
        return true;
    };
    let Some(src_item) = connection
        .player_data
        .house_inventory
        .get_item(src_container, src_slot)
    else {
        return true;
    };
    if src_item.item_id == 0 {
        return true;
    }

    let catalog = connection_catalog(connection);
    let Some(cat_id) = catalog.furniture_catalog_id(src_item.item_id, indoors) else {
        return true;
    };

    let material_item_key = if indoors {
        let mut db = connection.database.lock();
        super::db::list_furniture_for_estate(db.connection_mut(), estate.land_ident, None)
            .ok()
            .and_then(|rows| {
                rows.into_iter()
                    .find(|r| r.container_type == src_container as i32 && r.slot == src_slot as i32)
            })
            .map(|r| r.material_item_key.max(0) as u16)
            .unwrap_or(0)
    } else {
        0
    };

    let rotation = connection.player_data.volatile.rotation as f32;
    let position = connection.player_data.volatile.position;
    if indoors {
        connection
            .send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::InteriorFurniturePlaced {
                    storage_id: src_container,
                    slot: src_slot,
                    item_id: cat_id,
                    unk1: material_item_key,
                    stain: src_item.stains[0],
                    unk2: [0; 3],
                    rotation,
                    position,
                    unk3: [0; 4],
                },
            ))
            .await;
    } else {
        connection
            .send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::ExteriorFurniturePlaced {
                    plot_index: 0xFF,
                    slot: 0xFF,
                    unk1: [0; 2],
                    item_id: cat_id,
                    unk2: 0,
                    stain: src_item.stains[0],
                    unk3: [0; 3],
                    rotation,
                    position,
                    unk4: 0,
                },
            ))
            .await;
    }
    true
}

async fn handle_place_furniture(
    connection: &mut ZoneConnection,
    src_container: ContainerType,
    src_slot: u16,
    position: Position,
    rotation: f32,
    to_storeroom: bool,
) -> bool {
    let (position, rotation) = sanitize_transform(position, rotation);
    let zone_id = connection.player_data.volatile.zone_id as u16;
    let indoors = is_house_interior_territory(zone_id) || is_apartment_territory(zone_id);

    let Some(estate) = load_active_estate(connection) else {
        connection
            .send_notice("[Housing] No active estate selected.")
            .await;
        return true;
    };

    let from_storeroom = is_storeroom_container(src_container);
    let src_item = if from_storeroom {
        connection
            .player_data
            .house_inventory
            .get_item(src_container, src_slot)
            .unwrap_or_default()
    } else {
        connection
            .player_data
            .inventory
            .get_item(src_container, src_slot)
            .unwrap_or_default()
    };

    if src_item.item_id == 0 {
        return true;
    }
    let mut placed_item = src_item;
    if placed_item.condition == 0 {
        placed_item.condition = 30000;
    }

    let catalog = connection_catalog(connection);
    let Some(cat_id) = catalog.furniture_catalog_id(placed_item.item_id, indoors) else {
        return true;
    };

    let actor_cid = connection.player_data.character.content_id;

    if to_storeroom {
        let storeroom_candidates: Vec<ContainerType> = if indoors {
            INTERIOR_STOREROOM_CONTAINERS.to_vec()
        } else {
            EXTERIOR_STOREROOM_CONTAINERS.to_vec()
        };
        let mut free_storeroom: Option<(ContainerType, u16)> = None;
        for c in storeroom_candidates {
            for s_idx in 0..storeroom_slots_per_page(c) {
                if let Some(existing) = connection.player_data.house_inventory.get_item(c, s_idx)
                    && existing.item_id == 0
                {
                    free_storeroom = Some((c, s_idx));
                    break;
                }
            }
            if free_storeroom.is_some() {
                break;
            }
        }
        let Some((dst_c, dst_s)) = free_storeroom else {
            connection.send_notice("[Housing] Storeroom is full.").await;
            return true;
        };
        let row = HousingFurnitureRow {
            land_ident: estate.land_ident,
            container_type: dst_c as i32,
            slot: dst_s as i32,
            item_id: placed_item.item_id as i32,
            catalog_id: cat_id as i32,
            material_item_key: 0,
            stain: placed_item.stains[0] as i32,
            placed: false,
            pos_x: 0.0,
            pos_y: 0.0,
            pos_z: 0.0,
            rotation: 0.0,
            object_data_json: "{}".to_string(),
            remake_meta_json: "{}".to_string(),
            created_by_content_id: actor_cid,
            updated_at: 0,
        };
        {
            let mut db = connection.database.lock();
            let _ = upsert_furniture(db.connection_mut(), &row);
        }
        if let Some(s) = connection
            .player_data
            .inventory
            .get_item_mut(src_container, src_slot)
        {
            *s = Item::default();
        }
        if let Some(s) = connection
            .player_data
            .house_inventory
            .get_item_mut(dst_c, dst_s)
        {
            *s = placed_item;
        }
        let p_src = send_item_update_for(connection, src_container, src_slot, Item::default());
        let p_dst = send_item_update_for(connection, dst_c, dst_s, placed_item);
        connection.send_ipc_self(p_src).await;
        connection.send_ipc_self(p_dst).await;
        connection
            .actor_control_self(ActorControlCategory::LogMessage {
                log_message: LogMessageType::FurnitureMovedToStoreroom as u32,
                id: placed_item.item_id,
            })
            .await;
        return true;
    }

    let capacity = if indoors {
        let sz = PlotSize::from_repr(estate.plot_size.clamp(0, 2) as u8).unwrap_or_default();
        super::containers::indoor_slot_capacity(sz, estate.is_apartment)
    } else {
        super::containers::outdoor_slot_capacity(
            PlotSize::from_repr(estate.plot_size.clamp(0, 2) as u8).unwrap_or_default(),
        )
    };

    let occupied_slots = {
        let mut db = connection.database.lock();
        list_placed_furniture_in_scope(db.connection_mut(), estate.land_ident, indoors)
            .unwrap_or_default()
    };
    let mut used = vec![false; capacity];
    for r in &occupied_slots {
        if let Some(ctype) = container_from_i32(r.container_type)
            && let Some(flat) = flat_slot_for_placed_container(ctype, r.slot as u16)
            && (flat as usize) < used.len()
        {
            used[flat as usize] = true;
        }
    }

    let Some(free_flat) = used.iter().position(|&u| !u).map(|i| i as u16) else {
        connection
            .send_notice("[Housing] Estate furnishing limit reached.")
            .await;
        return true;
    };
    let Some((dst_container, dst_slot)) = placed_container_for_flat_slot(indoors, free_flat) else {
        return true;
    };

    let placed_row = if from_storeroom {
        let moved_row = {
            let mut db = connection.database.lock();
            let conn = db.connection_mut();
            let _ = move_furniture_row(
                conn,
                estate.land_ident,
                src_container as i32,
                src_slot as i32,
                dst_container as i32,
                dst_slot as i32,
                true,
            );
            let _ = update_furniture_position(
                conn,
                estate.land_ident,
                dst_container as i32,
                dst_slot as i32,
                position.0.x,
                position.0.y,
                position.0.z,
                rotation,
            );
            super::db::list_furniture_for_estate(conn, estate.land_ident, Some(indoors))
                .ok()
                .and_then(|rows| {
                    rows.into_iter().find(|r| {
                        r.container_type == dst_container as i32 && r.slot == dst_slot as i32
                    })
                })
        };
        if let Some(s) = connection
            .player_data
            .house_inventory
            .get_item_mut(src_container, src_slot)
        {
            *s = Item::default();
        }
        let pkt = send_item_update_for(connection, src_container, src_slot, Item::default());
        connection.send_ipc_self(pkt).await;
        moved_row
    } else {
        let row = HousingFurnitureRow {
            land_ident: estate.land_ident,
            container_type: dst_container as i32,
            slot: dst_slot as i32,
            item_id: placed_item.item_id as i32,
            catalog_id: cat_id as i32,
            material_item_key: 0,
            stain: placed_item.stains[0] as i32,
            placed: true,
            pos_x: position.0.x,
            pos_y: position.0.y,
            pos_z: position.0.z,
            rotation,
            object_data_json: "{}".to_string(),
            remake_meta_json: "{}".to_string(),
            created_by_content_id: actor_cid,
            updated_at: 0,
        };
        {
            let mut db = connection.database.lock();
            let _ = upsert_furniture(db.connection_mut(), &row);
        }
        if let Some(s) = connection
            .player_data
            .inventory
            .get_item_mut(src_container, src_slot)
        {
            *s = Item::default();
        }
        let pkt = send_item_update_for(connection, src_container, src_slot, Item::default());
        connection.send_ipc_self(pkt).await;
        Some(row)
    };

    if let Some(s) = connection
        .player_data
        .house_inventory
        .get_item_mut(dst_container, dst_slot)
    {
        *s = placed_item;
    }
    let dst_pkt = send_item_update_for(connection, dst_container, dst_slot, placed_item);
    connection.send_ipc_self(dst_pkt).await;

    let material_item_key = placed_row
        .as_ref()
        .map(|r| r.material_item_key.max(0) as u16)
        .unwrap_or(0);
    if indoors {
        connection
            .send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::InteriorFurniturePlaced {
                    storage_id: dst_container,
                    slot: dst_slot,
                    item_id: cat_id,
                    unk1: material_item_key,
                    stain: placed_item.stains[0],
                    unk2: [0; 3],
                    rotation,
                    position,
                    unk3: [0; 4],
                },
            ))
            .await;
        let value_set = placed_row
            .as_ref()
            .and_then(|r| {
                build_housing_object_data_value_sets(std::slice::from_ref(r))
                    .into_iter()
                    .next()
            })
            .unwrap_or_else(|| HousingObjectDataValueSet {
                furniture_index: free_flat,
                ..Default::default()
            });
        connection
            .send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::HousingObjectDataValueSet(value_set),
            ))
            .await;
    } else {
        connection
            .send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::ExteriorFurniturePlaced {
                    plot_index: raw_plot_index(&estate),
                    slot: free_flat as u8,
                    unk1: [0; 2],
                    item_id: cat_id,
                    unk2: 0,
                    stain: placed_item.stains[0],
                    unk3: [0; 3],
                    rotation,
                    position,
                    unk4: 0,
                },
            ))
            .await;
    }

    connection
        .actor_control_self(ActorControlCategory::FurniturePlacedAck {
            unk1: 0,
            unk2: 0,
            unk3: 0,
            unk4: 0,
        })
        .await;

    let key = HousingOverlayKey {
        indoors,
        ward_index: estate.ward_index as u8,
        division: estate.division.clamp(0, 1) as u8,
        plot_index: estate.plot_index as u8,
        slot: free_flat,
    };
    connection
        .handle
        .send(ToServer::HousingSim(
            HousingSimToServer::UpsertSingleOverlay {
                actor_id: connection.player_data.character.actor_id,
                object: HousingOverlayObject {
                    key,
                    catalog_id: cat_id,
                    position,
                    rotation,
                },
            },
        ))
        .await;
    if indoors && let Some(ref row) = placed_row {
        patch_synced_indoor_slot(connection, estate.land_ident, row);
    }
    emit_game_delta(
        connection,
        "furniture_added",
        estate.land_ident,
        format!("가구 배치 (#{free_flat})"),
        HousingBridgeDelta::FurnitureAdded {
            indoors,
            flat_slot: free_flat,
            item_id: placed_item.item_id,
            catalog_id: cat_id,
        },
    );
    true
}

async fn handle_remove_or_store_furniture(
    connection: &mut ZoneConnection,
    storage_id: ContainerType,
    slot: u16,
    to_storeroom: bool,
) -> bool {
    let zone_id = connection.player_data.volatile.zone_id as u16;
    let indoors = is_house_interior_territory(zone_id) || is_apartment_territory(zone_id);
    let Some(estate) = load_active_estate(connection) else {
        return true;
    };

    let current_item = connection
        .player_data
        .house_inventory
        .get_item(storage_id, slot)
        .unwrap_or_default();
    if current_item.item_id == 0 {
        return true;
    }
    let mut returned_item = current_item;
    if returned_item.condition == 0 {
        returned_item.condition = 30000;
    }

    let removed_flat_slot = flat_slot_for_placed_container(storage_id, slot);
    let division = estate.division.clamp(0, 1) as u8;
    let raw_plot = raw_plot_index(&estate);

    if to_storeroom {
        let storeroom_candidates: Vec<ContainerType> = if indoors {
            INTERIOR_STOREROOM_CONTAINERS.to_vec()
        } else {
            EXTERIOR_STOREROOM_CONTAINERS.to_vec()
        };
        let mut stored_target: Option<(ContainerType, u16)> = None;
        for c in storeroom_candidates {
            for s_idx in 0..storeroom_slots_per_page(c) {
                if let Some(existing) = connection.player_data.house_inventory.get_item(c, s_idx)
                    && existing.item_id == 0
                {
                    stored_target = Some((c, s_idx));
                    break;
                }
            }
            if stored_target.is_some() {
                break;
            }
        }
        let Some((dst_c, dst_s)) = stored_target else {
            connection.send_notice("[Housing] Storeroom is full.").await;
            return true;
        };

        if let Some(flat_slot) = removed_flat_slot {
            let key = HousingOverlayKey {
                indoors,
                ward_index: estate.ward_index as u8,
                division,
                plot_index: estate.plot_index as u8,
                slot: flat_slot,
            };
            connection
                .handle
                .send(ToServer::HousingSim(
                    HousingSimToServer::RemoveSingleOverlay {
                        actor_id: connection.player_data.character.actor_id,
                        key,
                    },
                ))
                .await;
        }

        {
            let mut db = connection.database.lock();
            let _ = move_furniture_row(
                db.connection_mut(),
                estate.land_ident,
                storage_id as i32,
                slot as i32,
                dst_c as i32,
                dst_s as i32,
                false,
            );
        }
        if let Some(s) = connection
            .player_data
            .house_inventory
            .get_item_mut(storage_id, slot)
        {
            *s = Item::default();
        }
        if let Some(s) = connection
            .player_data
            .house_inventory
            .get_item_mut(dst_c, dst_s)
        {
            *s = returned_item;
        }
        let p1 = send_item_update_for(connection, storage_id, slot, Item::default());
        let p2 = send_item_update_for(connection, dst_c, dst_s, returned_item);
        connection.send_ipc_self(p1).await;
        connection.send_ipc_self(p2).await;
        if let Some(flat_slot) = removed_flat_slot {
            let ack = if indoors {
                ActorControlCategory::FurnitureRemovedToInventoryAck {
                    unk1: flat_slot as u32,
                    unk2: 0,
                    unk3: 0,
                    unk4: 0,
                }
            } else {
                ActorControlCategory::OutdoorFurnitureRemovedAck {
                    plot_and_slot: ((raw_plot as u32) << 16) | ((flat_slot as u32) & 0xFF),
                    flag: 0,
                    unk3: 0,
                    unk4: 0,
                }
            };
            connection.actor_control_self(ack).await;
        }
        connection
            .actor_control_self(ActorControlCategory::LogMessage {
                log_message: LogMessageType::FurnitureMovedToStoreroom as u32,
                id: returned_item.item_id,
            })
            .await;
        if indoors {
            remove_synced_indoor_slot(connection, estate.land_ident, storage_id, slot);
        }
        if let Some(flat_slot) = removed_flat_slot {
            emit_game_delta(
                connection,
                "furniture_removed",
                estate.land_ident,
                format!("가구 창고 보관 (#{flat_slot})"),
                HousingBridgeDelta::FurnitureRemoved {
                    indoors,
                    flat_slot,
                    item_id: returned_item.item_id,
                },
            );
        }
        return true;
    }

    let mut free_inv_slot: Option<(ContainerType, u16)> = None;
    for ctype in [
        ContainerType::Inventory0,
        ContainerType::Inventory1,
        ContainerType::Inventory2,
        ContainerType::Inventory3,
    ] {
        for s_idx in 0..35u16 {
            if let Some(inv_item) = connection.player_data.inventory.get_item(ctype, s_idx)
                && inv_item.item_id == 0
            {
                free_inv_slot = Some((ctype, s_idx));
                break;
            }
        }
        if free_inv_slot.is_some() {
            break;
        }
    }
    let Some((inv_ctype, inv_slot)) = free_inv_slot else {
        connection.send_notice("[Housing] Inventory is full.").await;
        return true;
    };

    if let Some(flat_slot) = removed_flat_slot {
        let key = HousingOverlayKey {
            indoors,
            ward_index: estate.ward_index as u8,
            division,
            plot_index: estate.plot_index as u8,
            slot: flat_slot,
        };
        connection
            .handle
            .send(ToServer::HousingSim(
                HousingSimToServer::RemoveSingleOverlay {
                    actor_id: connection.player_data.character.actor_id,
                    key,
                },
            ))
            .await;
    }

    {
        let mut db = connection.database.lock();
        let _ = delete_furniture_slot(
            db.connection_mut(),
            estate.land_ident,
            storage_id as i32,
            slot as i32,
        );
    }
    if let Some(s) = connection
        .player_data
        .house_inventory
        .get_item_mut(storage_id, slot)
    {
        *s = Item::default();
    }
    let p_clear = send_item_update_for(connection, storage_id, slot, Item::default());
    connection.send_ipc_self(p_clear).await;

    connection
        .player_data
        .inventory
        .add_in_slot(returned_item, &inv_ctype, inv_slot);
    let pkt = send_item_update_for(connection, inv_ctype, inv_slot, returned_item);
    connection.send_ipc_self(pkt).await;

    if let Some(flat_slot) = removed_flat_slot {
        let ack = if indoors {
            ActorControlCategory::FurnitureRemovedToInventoryAck {
                unk1: flat_slot as u32,
                unk2: 0,
                unk3: 0,
                unk4: 0,
            }
        } else {
            ActorControlCategory::OutdoorFurnitureRemovedAck {
                plot_and_slot: ((raw_plot as u32) << 16) | ((flat_slot as u32) & 0xFF),
                flag: 0,
                unk3: 0,
                unk4: 0,
            }
        };
        connection.actor_control_self(ack).await;
    }
    connection
        .actor_control_self(ActorControlCategory::LogMessage {
            log_message: LogMessageType::FurnitureMovedToInventory as u32,
            id: returned_item.item_id,
        })
        .await;
    if indoors {
        remove_synced_indoor_slot(connection, estate.land_ident, storage_id, slot);
    }
    if let Some(flat_slot) = removed_flat_slot {
        emit_game_delta(
            connection,
            "furniture_removed",
            estate.land_ident,
            format!("가구 회수 (#{flat_slot})"),
            HousingBridgeDelta::FurnitureRemoved {
                indoors,
                flat_slot,
                item_id: returned_item.item_id,
            },
        );
    }
    true
}

async fn handle_translate_furniture(
    connection: &mut ZoneConnection,
    _house_id: HouseId,
    flat_slot: u16,
    position: Position,
    rotation: f32,
) -> bool {
    let (position, rotation) = sanitize_transform(position, rotation);
    let zone_id = connection.player_data.volatile.zone_id as u16;
    let indoors = is_house_interior_territory(zone_id) || is_apartment_territory(zone_id);
    let Some(estate) = load_active_estate(connection) else {
        return true;
    };

    let Some((container, slot_in_page)) = placed_container_for_flat_slot(indoors, flat_slot) else {
        return true;
    };

    {
        let mut db = connection.database.lock();
        let _ = update_furniture_position(
            db.connection_mut(),
            estate.land_ident,
            container as i32,
            slot_in_page as i32,
            position.0.x,
            position.0.y,
            position.0.z,
            rotation,
        );
    }

    let raw_plot = raw_plot_index(&estate);
    let plot_and_index = if indoors {
        flat_slot
    } else {
        ((raw_plot as u16) << 8) | (flat_slot & 0x00FF)
    };
    connection
        .send_ipc_self(ServerZoneIpcSegment::new(
            ServerZoneIpcData::FurnitureTranslatedForObserver(FurnitureTranslatedForObserver {
                rotation,
                plot_and_index,
                outdoor_index: (plot_and_index & 0xFF) as u8,
                plot_number: (plot_and_index >> 8) as u8,
                unk1: [0; 2],
                position,
                unk2: [0; 4],
            }),
        ))
        .await;

    connection
        .actor_control_self(ActorControlCategory::FurnitureTranslatedAck {
            storage_id: container,
            plot_number: if estate.is_apartment {
                0
            } else {
                raw_plot as u16
            },
            slot: slot_in_page as u32,
        })
        .await;

    connection
        .handle
        .send(ToServer::HousingSim(
            HousingSimToServer::TranslateSingleOverlay {
                actor_id: connection.player_data.character.actor_id,
                key: HousingOverlayKey {
                    indoors,
                    ward_index: estate.ward_index as u8,
                    division: estate.division.clamp(0, 1) as u8,
                    plot_index: estate.plot_index as u8,
                    slot: flat_slot,
                },
                position,
                rotation,
            },
        ))
        .await;
    if indoors {
        mutate_synced_indoor_slot(
            connection,
            estate.land_ident,
            container,
            slot_in_page,
            |s| {
                s.pos_x = position.0.x;
                s.pos_y = position.0.y;
                s.pos_z = position.0.z;
                s.rotation = rotation;
            },
        );
    }
    let catalog = connection_catalog(connection);
    let item_id = connection
        .player_data
        .house_inventory
        .get_item(container, slot_in_page)
        .map(|i| i.item_id)
        .unwrap_or(0);
    let catalog_id = if item_id != 0 {
        catalog.furniture_catalog_id(item_id, indoors).unwrap_or(0)
    } else {
        0
    };
    let (local_pos, local_rot) = local_transform_for_estate(&estate, indoors, position, rotation);
    emit_game_delta(
        connection,
        "furniture_moved",
        estate.land_ident,
        format!("가구 이동 (#{flat_slot})"),
        HousingBridgeDelta::FurnitureMoved {
            indoors,
            moves: vec![HousingBridgeMoveItem {
                flat_slot,
                item_id,
                catalog_id,
                pos_x: position.0.x,
                pos_y: position.0.y,
                pos_z: position.0.z,
                rotation,
                local_x: local_pos.0.x,
                local_y: local_pos.0.y,
                local_z: local_pos.0.z,
                local_rotation: local_rot,
            }],
        },
    );
    true
}

async fn handle_batch_translate(
    connection: &mut ZoneConnection,
    batch: &BatchTranslateIndoorFurniture,
) -> bool {
    let Some(estate) = load_active_estate(connection) else {
        return true;
    };
    let mut ack_entries = [BatchTranslatedFurnitureEntry::default(); 10];
    let mut overlays = Vec::new();
    let mut synced_updates = Vec::new();

    {
        let mut db = connection.database.lock();
        let conn = db.connection_mut();
        for (i, ack) in ack_entries.iter_mut().enumerate() {
            let flat_slot = batch.slots[i];
            if flat_slot == 0xFFFF {
                continue;
            }
            if let Some((container, slot_in_page)) = placed_container_for_flat_slot(true, flat_slot)
            {
                let (position, rotation) = sanitize_transform(
                    Position(glam::Vec3A::new(
                        batch.pos_x[i],
                        batch.pos_y[i],
                        batch.pos_z[i],
                    )),
                    batch.rotation[i],
                );
                let _ = update_furniture_position(
                    conn,
                    estate.land_ident,
                    container as i32,
                    slot_in_page as i32,
                    position.0.x,
                    position.0.y,
                    position.0.z,
                    rotation,
                );
                *ack = BatchTranslatedFurnitureEntry {
                    slot: flat_slot,
                    pad0: 0,
                    rotation,
                    position,
                };
                overlays.push((flat_slot, position, rotation));
                synced_updates.push((container, slot_in_page, position, rotation));
            }
        }
    }

    for (flat_slot, position, rotation) in overlays {
        connection
            .handle
            .send(ToServer::HousingSim(
                HousingSimToServer::TranslateSingleOverlay {
                    actor_id: connection.player_data.character.actor_id,
                    key: HousingOverlayKey {
                        indoors: true,
                        ward_index: estate.ward_index as u8,
                        division: estate.division.clamp(0, 1) as u8,
                        plot_index: estate.plot_index as u8,
                        slot: flat_slot,
                    },
                    position,
                    rotation,
                },
            ))
            .await;
    }

    connection
        .send_ipc_self(ServerZoneIpcSegment::new(
            ServerZoneIpcData::BatchIndoorFurnitureTranslated(BatchIndoorFurnitureTranslated {
                house_id: HouseId::from(estate.house_id as u64),
                entries: ack_entries,
            }),
        ))
        .await;
    let catalog = connection_catalog(connection);
    let mut delta_moves = Vec::with_capacity(synced_updates.len());
    for (container, slot_in_page, position, rotation) in synced_updates {
        mutate_synced_indoor_slot(
            connection,
            estate.land_ident,
            container,
            slot_in_page,
            |s| {
                s.pos_x = position.0.x;
                s.pos_y = position.0.y;
                s.pos_z = position.0.z;
                s.rotation = rotation;
            },
        );
        if let Some(flat_slot) = flat_slot_for_placed_container(container, slot_in_page) {
            let item_id = connection
                .player_data
                .house_inventory
                .get_item(container, slot_in_page)
                .map(|i| i.item_id)
                .unwrap_or(0);
            let catalog_id = if item_id != 0 {
                catalog.furniture_catalog_id(item_id, true).unwrap_or(0)
            } else {
                0
            };
            delta_moves.push(HousingBridgeMoveItem {
                flat_slot,
                item_id,
                catalog_id,
                pos_x: position.0.x,
                pos_y: position.0.y,
                pos_z: position.0.z,
                rotation,
                local_x: position.0.x,
                local_y: position.0.y,
                local_z: position.0.z,
                local_rotation: rotation,
            });
        }
    }
    if !delta_moves.is_empty() {
        emit_game_delta(
            connection,
            "furniture_moved",
            estate.land_ident,
            format!("가구 일괄 이동 ({}개)", delta_moves.len()),
            HousingBridgeDelta::FurnitureMoved {
                indoors: true,
                moves: delta_moves,
            },
        );
    }
    true
}

async fn handle_housing_dye(connection: &mut ZoneConnection, info: &DyeInformation) -> bool {
    if !info.is_housing_dye_mode() && !is_housing_container(info.target_container) {
        return false;
    }
    let Some(estate) = load_active_estate(connection) else {
        return true;
    };
    let stain_id = info.housing_stain();

    if is_appearance_container(info.target_container) {
        if info.target_container == ContainerType::HousingExteriorAppearance {
            let mut updated_est_opt: Option<HousingEstateRow> = None;
            {
                let mut db = connection.database.lock();
                if let Some(mut est) = estate_by_land_ident(db.connection_mut(), estate.land_ident)
                    .ok()
                    .flatten()
                {
                    let mut ext = StoredHouseExterior::from_json(&est.exterior_json);
                    let mut interior = StoredHouseInterior::from_json(&est.interior_json);
                    if info.target_slot == 0 {
                        ext.set_all_stains(stain_id);
                        interior.window_stain = stain_id;
                        interior.door_stain = stain_id;
                    } else if (1..=8).contains(&info.target_slot) {
                        let part_idx = (info.target_slot - 1) as usize;
                        ext.set_stain(part_idx, stain_id);
                        if part_idx == 2 {
                            interior.window_stain = stain_id;
                        } else if part_idx == 3 {
                            interior.door_stain = stain_id;
                        }
                    }
                    est.exterior_json = ext.to_json();
                    est.interior_json = interior.to_json();
                    let _ = save_estate(db.connection_mut(), &est);
                    updated_est_opt = Some(est);
                }
            }
            if let Some(est) = updated_est_opt {
                let ext = StoredHouseExterior::from_json(&est.exterior_json);
                let pkt = ext.to_live_update_packet(&est);
                if info.target_slot == 0 {
                    for s_idx in 0..=8u16 {
                        let updated_item = connection
                            .player_data
                            .house_inventory
                            .get_item_mut(ContainerType::HousingExteriorAppearance, s_idx)
                            .filter(|item| item.item_id != 0)
                            .map(|item| {
                                item.stains = [stain_id, 0];
                                *item
                            });
                        if let Some(updated_item) = updated_item {
                            let inv_pkt = send_item_update_for(
                                connection,
                                ContainerType::HousingExteriorAppearance,
                                s_idx,
                                updated_item,
                            );
                            connection.send_ipc_self(inv_pkt).await;
                        }
                    }
                    connection
                        .send_ipc_self(ServerZoneIpcSegment::new(
                            ServerZoneIpcData::HousingStainUpdated(HousingStainUpdated {
                                stain: stain_id,
                                pad0: 0,
                                container_id: ContainerType::HousingExteriorAppearance,
                                slot: 0,
                                pad1: [0; 3],
                                plot_index: raw_plot_index(&est),
                                pad2: 0,
                                apply_all_exterior_parts: 1,
                                pad3: 0,
                            }),
                        ))
                        .await;
                } else {
                    let updated_item = connection
                        .player_data
                        .house_inventory
                        .get_item_mut(ContainerType::HousingExteriorAppearance, info.target_slot)
                        .filter(|item| item.item_id != 0)
                        .map(|item| {
                            item.stains = [stain_id, 0];
                            *item
                        });
                    if let Some(updated_item) = updated_item {
                        let inv_pkt = send_item_update_for(
                            connection,
                            ContainerType::HousingExteriorAppearance,
                            info.target_slot,
                            updated_item,
                        );
                        connection.send_ipc_self(inv_pkt).await;
                    }
                }
                connection
                    .send_ipc_self(ServerZoneIpcSegment::new(
                        ServerZoneIpcData::HousingExteriorAppearanceUpdate(pkt),
                    ))
                    .await;
                emit_game_delta(
                    connection,
                    "layout_changed",
                    estate.land_ident,
                    "외장 염색 변경",
                    HousingBridgeDelta::LayoutChanged {
                        indoors: false,
                        reason: "exterior_dyed".to_string(),
                    },
                );
            }
        } else if info.target_container == ContainerType::HousingInteriorAppearance {
            let updated_item = connection
                .player_data
                .house_inventory
                .get_item_mut(ContainerType::HousingInteriorAppearance, info.target_slot)
                .filter(|item| item.item_id != 0)
                .map(|item| {
                    item.stains = [stain_id, 0];
                    *item
                });
            if let Some(updated_item) = updated_item {
                let inv_pkt = send_item_update_for(
                    connection,
                    ContainerType::HousingInteriorAppearance,
                    info.target_slot,
                    updated_item,
                );
                connection.send_ipc_self(inv_pkt).await;
            }
            let est = load_active_estate(connection).unwrap_or(estate);
            let plot_size =
                PlotSize::from_repr(est.plot_size.clamp(0, 2) as u8).unwrap_or_default();
            let details = StoredHouseInterior::from_json(&est.interior_json).to_packet(
                est.light_level.clamp(0, 5) as u8,
                est.enable_ssao,
                plot_size,
                est.is_apartment,
            );
            connection
                .send_ipc_self(ServerZoneIpcSegment::new(
                    ServerZoneIpcData::HousingInteriorDetails(details),
                ))
                .await;
            patch_synced_indoor_meta(connection, &est);
            emit_game_delta(
                connection,
                "layout_changed",
                est.land_ident,
                "내장 염색 변경",
                HousingBridgeDelta::LayoutChanged {
                    indoors: true,
                    reason: "interior_dyed".to_string(),
                },
            );
        }
        return true;
    }

    {
        let mut db = connection.database.lock();
        let _ = update_furniture_stain(
            db.connection_mut(),
            estate.land_ident,
            info.target_container as i32,
            info.target_slot as i32,
            stain_id,
        );
    }

    let updated_item = connection
        .player_data
        .house_inventory
        .get_item_mut(info.target_container, info.target_slot)
        .filter(|item| item.item_id != 0)
        .map(|item| {
            item.stains = [stain_id, 0];
            *item
        });
    if let Some(updated_item) = updated_item {
        let inv_pkt = send_item_update_for(
            connection,
            info.target_container,
            info.target_slot,
            updated_item,
        );
        connection.send_ipc_self(inv_pkt).await;
    }

    connection
        .send_ipc_self(ServerZoneIpcSegment::new(
            ServerZoneIpcData::HousingStainUpdated(HousingStainUpdated {
                stain: stain_id,
                pad0: 0,
                container_id: info.target_container,
                slot: info.target_slot as u8,
                pad1: [0; 3],
                plot_index: raw_plot_index(&estate),
                pad2: 0,
                apply_all_exterior_parts: 0,
                pad3: 0,
            }),
        ))
        .await;
    let zone_id = connection.player_data.volatile.zone_id as u16;
    let indoors = is_house_interior_territory(zone_id)
        || is_apartment_territory(zone_id)
        || is_interior_placed_container(info.target_container);
    if indoors {
        mutate_synced_indoor_slot(
            connection,
            estate.land_ident,
            info.target_container,
            info.target_slot,
            |s| {
                s.stain = stain_id;
            },
        );
    }
    if let Some(flat_slot) = flat_slot_for_placed_container(info.target_container, info.target_slot)
    {
        let catalog = connection_catalog(connection);
        let item_id = updated_item.map(|i| i.item_id).unwrap_or(0);
        let color_hex = if stain_id > 0 {
            catalog.stain_rgba_hex(stain_id)
        } else {
            None
        };
        emit_game_delta(
            connection,
            "furniture_dyed",
            estate.land_ident,
            format!("가구 염색 (#{flat_slot})"),
            HousingBridgeDelta::FurnitureDyed {
                indoors,
                flat_slot,
                item_id,
                stain: stain_id,
                color_hex,
            },
        );
    }
    true
}

async fn handle_interior_remodel(
    connection: &mut ZoneConnection,
    op: &InteriorRemodelItemOperation,
) -> bool {
    let Some(estate) = load_active_estate(connection) else {
        return true;
    };
    let catalog = connection_catalog(connection);
    let mut interior = StoredHouseInterior::from_json(&estate.interior_json);
    let changed = apply_interior_remodel_op(&mut interior, op, |src_c, src_s| {
        let ctype = container_from_u16(src_c)?;
        let item = connection
            .player_data
            .inventory
            .get_item(ctype, src_s)
            .or_else(|| {
                connection
                    .player_data
                    .house_inventory
                    .get_item(ctype, src_s)
            })?;
        catalog
            .items_by_id
            .get(&item.item_id)
            .map(|m| m.additional_data)
    });

    if changed {
        let mut updated = estate.clone();
        updated.interior_json = interior.to_json();
        {
            let mut db = connection.database.lock();
            let _ = create_snapshot(db.connection_mut(), updated.land_ident, "interior remodel");
            let _ = save_estate(db.connection_mut(), &updated);
        }
        push_indoor_live_refresh(connection, &updated).await;
        emit_game_delta(
            connection,
            "layout_changed",
            updated.land_ident,
            "실내 내장재 변경",
            HousingBridgeDelta::LayoutChanged {
                indoors: true,
                reason: "interior_remodel".to_string(),
            },
        );
    }

    connection
        .actor_control_self(ActorControlCategory::FinishEstateAppearanceItemOperation {})
        .await;
    true
}

async fn handle_exterior_remodel(
    connection: &mut ZoneConnection,
    op: &ExteriorRemodelItemOperation,
) -> bool {
    let Some(estate) = load_active_estate(connection) else {
        return true;
    };
    let catalog = connection_catalog(connection);
    let mut exterior = StoredHouseExterior::from_json(&estate.exterior_json);
    let changed = apply_exterior_remodel_op(&mut exterior, &catalog, op, |src_c, src_s| {
        let ctype = container_from_u16(src_c)?;
        let item = connection
            .player_data
            .inventory
            .get_item(ctype, src_s)
            .or_else(|| {
                connection
                    .player_data
                    .house_inventory
                    .get_item(ctype, src_s)
            })?;
        Some(item.item_id)
    });

    if changed {
        let mut updated = estate.clone();
        updated.exterior_json = exterior.to_json();
        {
            let mut db = connection.database.lock();
            let _ = create_snapshot(db.connection_mut(), updated.land_ident, "exterior remodel");
            let _ = save_estate(db.connection_mut(), &updated);
        }
        let pkt: HousingExteriorAppearanceUpdate = exterior.to_live_update_packet(&updated);
        connection
            .send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::HousingExteriorAppearanceUpdate(pkt),
            ))
            .await;
        rebuild_house_inventory_from_db(connection, &updated, false);
        emit_game_delta(
            connection,
            "layout_changed",
            updated.land_ident,
            "실외 외장재 변경",
            HousingBridgeDelta::LayoutChanged {
                indoors: false,
                reason: "exterior_remodel".to_string(),
            },
        );
    }

    connection
        .actor_control_self(ActorControlCategory::FinishEstateAppearanceItemOperation {})
        .await;
    true
}

async fn handle_update_name(connection: &mut ZoneConnection, pkt: &UpdateEstateName) -> bool {
    let Some(mut est) = load_active_estate(connection) else {
        return true;
    };
    let decoded = decode_cstr(&pkt.name_bytes);
    est.estate_name = clamp_utf8(&decoded, MAX_ESTATE_NAME_BYTES);
    {
        let mut db = connection.database.lock();
        let _ = save_estate(db.connection_mut(), &est);
    }
    connection
        .send_notice(&format!(
            "[Housing] Estate name saved: '{}'.",
            est.estate_name
        ))
        .await;
    true
}

async fn handle_update_greeting(
    connection: &mut ZoneConnection,
    pkt: &UpdateEstateGreeting,
) -> bool {
    let Some(mut est) = load_active_estate(connection) else {
        return true;
    };
    let decoded = decode_cstr(&pkt.greeting_bytes);
    est.greeting = clamp_utf8(&decoded, MAX_ESTATE_GREETING_BYTES);
    {
        let mut db = connection.database.lock();
        let _ = save_estate(db.connection_mut(), &est);
    }
    connection
        .send_notice("[Housing] Estate greeting saved.")
        .await;
    true
}

async fn handle_update_profile(connection: &mut ZoneConnection, pkt: &UpdateEstateProfile) -> bool {
    let Some(mut est) = load_active_estate(connection) else {
        return true;
    };
    let open_bit = HousingFlag::OPEN.bits() as i32;
    if (pkt.guest_access_flags & 1) != 0 {
        est.flags |= open_bit;
    } else {
        est.flags &= !open_bit;
    }
    est.appeal_tag_0 = pkt.profile_bytes[0] as i32;
    est.appeal_tag_1 = pkt.profile_bytes[1] as i32;
    est.appeal_tag_2 = pkt.profile_bytes[2] as i32;
    {
        let mut db = connection.database.lock();
        let _ = save_estate(db.connection_mut(), &est);
    }
    connection
        .send_notice("[Housing] Estate appeal profile saved.")
        .await;
    true
}

async fn handle_build_deed(connection: &mut ZoneConnection, pkt: &BuildEstateDeed) -> bool {
    let zone_id = connection.player_data.volatile.zone_id as u16;
    let district = default_district_for_zone(zone_id);
    let ward = connection
        .housing_sim
        .display_ward_context
        .map(|c| c.ward_index)
        .unwrap_or(0);
    let (division, plot_idx) = if pkt.plot_index >= 30 {
        (1u8, pkt.plot_index - 30)
    } else {
        (
            connection
                .housing_sim
                .display_ward_context
                .map(|c| c.division)
                .unwrap_or(0),
            pkt.plot_index,
        )
    };
    let deed_name = decode_cstr(&pkt.deed_name_bytes);
    let name_override = if deed_name.trim().is_empty() {
        None
    } else {
        Some(deed_name)
    };
    if let Some(est) = ensure_local_plot_estate(
        connection,
        district,
        ward,
        division,
        plot_idx,
        None,
        false,
        name_override,
    ) {
        send_owned_housing(connection).await;
        execute_housing_reload(
            connection,
            Some(est.land_ident),
            ReloadMode::Auto,
            true,
            false,
        )
        .await;
    }
    true
}

async fn handle_update_aquarium(connection: &mut ZoneConnection, pkt: &UpdateAquarium) -> bool {
    let Some(estate) = load_active_estate(connection) else {
        return true;
    };
    let catalog = connection_catalog(connection);
    let trim_item_id = if pkt.trimming_container != 9999 {
        u16::try_from(pkt.trimming_container)
            .ok()
            .and_then(container_from_u16)
            .and_then(|c| {
                connection
                    .player_data
                    .inventory
                    .get_item(c, pkt.trimming_slot)
            })
            .filter(|i| i.item_id != 0)
            .map(|i| i.item_id)
            .unwrap_or(0)
    } else {
        0
    };
    let mut fish_item_ids = [0u32; 4];
    let mut fish_params = [0u16; 4];
    for i in 0..4 {
        if pkt.fish_containers[i] != 9999
            && let Some(c) = u16::try_from(pkt.fish_containers[i])
                .ok()
                .and_then(container_from_u16)
            && let Some(item) = connection
                .player_data
                .inventory
                .get_item(c, pkt.fish_slots[i])
            && item.item_id != 0
        {
            fish_item_ids[i] = item.item_id;
            fish_params[i] = catalog.aquarium_fish_param_for_item(item.item_id);
        }
    }

    let water_param = pkt.water_type as u16;
    let mut count = 1usize;
    for i in 0..4 {
        if fish_params[i] != 0 || fish_item_ids[i] != 0 {
            count = i + 2;
        }
    }
    let value_count = count.min(8) as u8;

    if let Some((container, slot_in_page)) =
        placed_container_for_flat_slot(true, pkt.furniture_slot)
    {
        let obj_data = FurnitureObjectData {
            aquarium: Some(AquariumObjectState {
                trim_item_id,
                water_param,
                fish_item_ids,
                fish_params,
            }),
            values: vec![
                water_param,
                fish_params[0],
                fish_params[1],
                fish_params[2],
                fish_params[3],
            ],
            ..Default::default()
        };
        {
            let mut db = connection.database.lock();
            let _ = update_furniture_object_data(
                db.connection_mut(),
                estate.land_ident,
                container as i32,
                slot_in_page as i32,
                Some(trim_item_id as u16),
                &obj_data,
            );
        }
        connection
            .send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::HousingObjectDataValueSet(HousingObjectDataValueSet {
                    furniture_index: pkt.furniture_slot,
                    value_count,
                    reserved: 0,
                    values: [
                        water_param,
                        fish_params[0],
                        fish_params[1],
                        fish_params[2],
                        fish_params[3],
                        0,
                        0,
                        0,
                    ],
                    param_a: [trim_item_id as u8, 0, 0, 0, 0, 0, 0, 0],
                    param_b: [0; 8],
                    param_c: [0; 8],
                    padding: [0; 4],
                }),
            ))
            .await;
        let obj_json = serde_json::to_string(&obj_data).unwrap_or_else(|_| "{}".to_string());
        mutate_synced_indoor_slot(
            connection,
            estate.land_ident,
            container,
            slot_in_page,
            |s| {
                s.material_item_key = trim_item_id as u16;
                s.object_data_json = obj_json;
            },
        );
        emit_game_delta(
            connection,
            "layout_changed",
            estate.land_ident,
            "수조 설정 변경",
            HousingBridgeDelta::LayoutChanged {
                indoors: true,
                reason: "aquarium_updated".to_string(),
            },
        );
    }
    true
}

async fn handle_update_orchestrion(
    connection: &mut ZoneConnection,
    pkt: &UpdateOrchestrionPlaylist,
) -> bool {
    let Some(mut estate) = load_active_estate(connection) else {
        return true;
    };
    let mut meta: BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&estate.remake_meta_json).unwrap_or_default();
    meta.insert(
        "orchestrion_raw".to_string(),
        serde_json::Value::from(pkt.raw.to_vec()),
    );
    estate.remake_meta_json = serde_json::to_string(&meta).unwrap_or_else(|_| "{}".to_string());
    {
        let mut db = connection.database.lock();
        let _ = save_estate(db.connection_mut(), &estate);
    }
    true
}

/// Processes `ToServer::HousingSim(...)` inside `server_main_loop`.
pub(crate) fn handle_server_housing_message(
    data: &Arc<Mutex<WorldServer>>,
    network: &Arc<Mutex<NetworkState>>,
    game_data: &Arc<Mutex<GameData>>,
    lua: &Arc<Mutex<KawariLua>>,
    msg: &ToServer,
) -> bool {
    let ToServer::HousingSim(sim_msg) = msg else {
        return false;
    };

    match sim_msg {
        HousingSimToServer::SyncScopeOverlays {
            actor_id,
            scope,
            objects,
        } => {
            let mut data = data.lock();
            let mut network = network.lock();
            let mut game_data = game_data.lock();
            let catalog = cached_housing_catalog(&mut game_data);
            if let Some(instance) = data.find_actor_instance_mut(*actor_id) {
                replace_scope_overlays(
                    instance,
                    &mut network,
                    &mut game_data,
                    &catalog,
                    *scope,
                    objects,
                );
            }
        }
        HousingSimToServer::UpsertSingleOverlay { actor_id, object } => {
            let mut data = data.lock();
            let mut network = network.lock();
            let mut game_data = game_data.lock();
            let catalog = cached_housing_catalog(&mut game_data);
            if let Some(instance) = data.find_actor_instance_mut(*actor_id) {
                upsert_single_overlay(instance, &mut network, &mut game_data, &catalog, *object);
            }
        }
        HousingSimToServer::TranslateSingleOverlay {
            actor_id,
            key,
            position,
            rotation,
        } => {
            let mut data = data.lock();
            let mut network = network.lock();
            if let Some(instance) = data.find_actor_instance_mut(*actor_id) {
                translate_single_overlay(instance, &mut network, *key, *position, *rotation);
            }
        }
        HousingSimToServer::RemoveSingleOverlay { actor_id, key } => {
            let mut data = data.lock();
            let mut network = network.lock();
            if let Some(instance) = data.find_actor_instance_mut(*actor_id) {
                remove_single_overlay(instance, &mut network, *key);
            }
        }
        HousingSimToServer::WarpToHouseInterior {
            client_id,
            actor_id,
            house_id,
            territory_type_id,
            preserve_pos,
            preserve_rot,
        } => {
            let mut data = data.lock();
            let mut network = network.lock();
            if !network.clients.contains_key(client_id) {
                tracing::warn!(
                    client_id = ?client_id,
                    "Skipping WarpToHouseInterior because client is no longer registered"
                );
                return true;
            }
            let mut game_data = game_data.lock();
            let lua = lua.lock();

            let (target_instance, needs_init_zone) = begin_change_zone(
                &mut data,
                &mut network,
                &mut game_data,
                &lua,
                *territory_type_id,
                *actor_id,
                WarpType::Normal,
                0,
                Some(InstanceQuery::House {
                    house_id: *house_id,
                }),
            );

            let (pos, rot) = if let Some(p) = preserve_pos {
                (Some(*p), *preserve_rot)
            } else {
                let (entry_pos, entry_rot) = indoor_entry_transform(*territory_type_id);
                (Some(entry_pos), Some(entry_rot))
            };

            do_change_zone(
                &mut network,
                target_instance,
                needs_init_zone,
                pos,
                rot,
                *client_id,
                WarpType::Normal,
            );
        }
        HousingSimToServer::WarpToHousingPlot {
            client_id,
            actor_id,
            territory_type_id,
            ward_index,
            division,
            plot_index,
            override_pos,
            override_rot,
        } => {
            let mut data = data.lock();
            let mut network = network.lock();
            if !network.clients.contains_key(client_id) {
                tracing::warn!(
                    client_id = ?client_id,
                    "Skipping WarpToHousingPlot because client is no longer registered"
                );
                return true;
            }
            let mut game_data = game_data.lock();
            let lua = lua.lock();

            let (target_instance, needs_init_zone) = begin_change_zone(
                &mut data,
                &mut network,
                &mut game_data,
                &lua,
                *territory_type_id,
                *actor_id,
                WarpType::Normal,
                0,
                Some(InstanceQuery::Ward {
                    ward: *ward_index as u16,
                }),
            );

            let raw_plot_index = (*division as usize) * 30 + (*plot_index as usize);
            let (exit_pos, exit_rot) = if let Some(p) = override_pos {
                (Some(*p), *override_rot)
            } else if let Some((p, r)) = target_instance
                .zone
                .housing_plot_exit_transform(raw_plot_index)
            {
                (Some(p), Some(r))
            } else if let Some(spec) = plot_spec(*territory_type_id, *division, *plot_index) {
                let (p, r) = nudge_plot_entrance_exit(
                    Position(glam::Vec3A::new(spec.x, spec.y, spec.z)),
                    spec.rotation,
                    8.0,
                );
                (Some(p), Some(r))
            } else {
                (None, None)
            };

            do_change_zone(
                &mut network,
                target_instance,
                needs_init_zone,
                exit_pos,
                exit_rot,
                *client_id,
                WarpType::Normal,
            );
        }
        HousingSimToServer::WarpToApartmentOutdoor {
            client_id,
            actor_id,
            territory_type_id,
            ward_index,
            division,
        } => {
            let mut data = data.lock();
            let mut network = network.lock();
            if !network.clients.contains_key(client_id) {
                tracing::warn!(
                    client_id = ?client_id,
                    "Skipping WarpToApartmentOutdoor because client is no longer registered"
                );
                return true;
            }
            let mut game_data = game_data.lock();
            let lua = lua.lock();

            let pop_id = super::gamedata_ext::apartment_aethernet_pop_range(
                &mut game_data,
                *territory_type_id,
                *division,
            );

            let (target_instance, needs_init_zone) = begin_change_zone(
                &mut data,
                &mut network,
                &mut game_data,
                &lua,
                *territory_type_id,
                *actor_id,
                WarpType::Normal,
                0,
                Some(InstanceQuery::Ward {
                    ward: *ward_index as u16,
                }),
            );

            let (pos, rot) = pop_id
                .and_then(|id| target_instance.zone.cached_pop_ranges.get(&id).copied())
                .or_else(|| {
                    target_instance
                        .zone
                        .find_random_pop_range()
                        .map(|obj| (pick_point_in_pop_range(obj), 0.0))
                })
                .map(|(p, r)| (Some(p), Some(r)))
                .unwrap_or((None, None));

            do_change_zone(
                &mut network,
                target_instance,
                needs_init_zone,
                pos,
                rot,
                *client_id,
                WarpType::Normal,
            );
        }
        HousingSimToServer::SpawnLoginInWard {
            client_id,
            actor_id,
            territory_type_id,
            ward_index,
            division,
            plot_index,
            is_apartment_exit,
            fallback_pos,
            fallback_rot,
        } => {
            let mut data = data.lock();
            let mut network = network.lock();
            let mut game_data = game_data.lock();
            let lua = lua.lock();

            let apt_pop_id = if *is_apartment_exit {
                super::gamedata_ext::apartment_aethernet_pop_range(
                    &mut game_data,
                    *territory_type_id,
                    *division,
                )
            } else {
                None
            };

            let instance = data.ensure_exists_query(
                &mut game_data,
                &lua,
                *territory_type_id,
                InstanceQuery::Ward {
                    ward: *ward_index as u16,
                },
            );
            instance.insert_empty_actor(*actor_id);

            let (exit_position, exit_rotation) = if *is_apartment_exit {
                apt_pop_id
                    .and_then(|id| instance.zone.cached_pop_ranges.get(&id).copied())
                    .or_else(|| {
                        instance
                            .zone
                            .find_random_pop_range()
                            .map(|obj| (pick_point_in_pop_range(obj), 0.0))
                    })
                    .unwrap_or((*fallback_pos, *fallback_rot))
            } else if let Some(plot_idx) = plot_index {
                let raw_plot = (*division as usize) * 30 + (*plot_idx as usize);
                instance
                    .zone
                    .housing_plot_exit_transform(raw_plot)
                    .unwrap_or((*fallback_pos, *fallback_rot))
            } else {
                (*fallback_pos, *fallback_rot)
            };

            let msg = FromServer::ChangeZone(
                exit_position,
                exit_rotation,
                instance
                    .zone
                    .to_lua_zone(instance.weather_id, instance.query),
                instance.content_handler_id(),
            );
            network.send_to(*client_id, msg, DestinationNetwork::ZoneClients);
        }
        HousingSimToServer::SwitchClientEstate {
            content_id,
            land_ident,
            enter_interior,
        } => {
            let mut network = network.lock();
            for (handle, _state) in network.clients.values_mut() {
                if content_id.is_none_or(|cid| cid == handle.content_id) {
                    let _ = handle.send(FromServer::HousingSim(
                        HousingSimFromServer::SwitchActiveEstate {
                            land_ident: *land_ident,
                            enter_interior: *enter_interior,
                        },
                    ));
                }
            }
        }
        HousingSimToServer::BroadcastEstateLivePackets {
            land_ident: _,
            indoors_only,
            packets,
        } => {
            let data = data.lock();
            let mut network = network.lock();
            for (handle, _state) in network.clients.values_mut() {
                if let Some(inst) = data.find_actor_instance(handle.actor_id) {
                    let is_indoor = inst.zone.intended_use == TerritoryIntendedUse::HousingIndoor;
                    if *indoors_only && !is_indoor {
                        continue;
                    }
                    for pkt in packets {
                        let _ = handle
                            .send(FromServer::PacketSegment(pkt.clone(), ObjectId::default()));
                    }
                }
            }
        }
        HousingSimToServer::BroadcastEstateLiveMoves {
            land_ident,
            indoors,
            moves,
        } => {
            let mut network = network.lock();
            for (handle, _state) in network.clients.values_mut() {
                let _ = handle.send(FromServer::HousingSim(
                    HousingSimFromServer::ApplyLiveMoves {
                        land_ident: *land_ident,
                        indoors: *indoors,
                        moves: moves.clone(),
                    },
                ));
            }
        }
        HousingSimToServer::NotifyClientsEstateReload {
            land_ident,
            mode,
            preserve_player_pos,
            enter_if_elsewhere,
            source,
        } => {
            let mut network = network.lock();
            for (handle, _state) in network.clients.values_mut() {
                let _ = handle.send(FromServer::HousingSim(
                    HousingSimFromServer::TriggerReload {
                        land_ident: *land_ident,
                        mode: *mode,
                        preserve_player_pos: *preserve_player_pos,
                        force_enter: *enter_if_elsewhere,
                        source: source.clone(),
                    },
                ));
            }
        }
        HousingSimToServer::UnlockClientSession { content_id } => {
            let mut network = network.lock();
            for (handle, _state) in network.clients.values_mut() {
                if content_id.is_none_or(|cid| cid == handle.content_id) {
                    let _ = handle.send(FromServer::HousingSim(
                        HousingSimFromServer::UnlockCharacter,
                    ));
                }
            }
        }
        HousingSimToServer::GiveClientSampleKit { content_id, kind } => {
            let mut network = network.lock();
            for (handle, _state) in network.clients.values_mut() {
                if content_id.is_none_or(|cid| cid == handle.content_id) {
                    let _ = handle.send(FromServer::HousingSim(
                        HousingSimFromServer::GiveSampleKit { kind: kind.clone() },
                    ));
                }
            }
        }
    }

    true
}

/// Processes `FromServer::HousingSim(...)` inside a `ZoneConnection` event loop.
pub async fn handle_from_server(connection: &mut ZoneConnection, sim_msg: &HousingSimFromServer) {
    match sim_msg {
        HousingSimFromServer::TriggerReload {
            land_ident,
            mode,
            preserve_player_pos,
            force_enter,
            source,
        } => {
            let estate = {
                let mut db = connection.database.lock();
                estate_by_land_ident(db.connection_mut(), *land_ident)
                    .ok()
                    .flatten()
            };
            let Some(est) = estate else {
                return;
            };

            let current_zone_id = connection.player_data.volatile.zone_id as u16;
            let matches_active = connection
                .housing_sim
                .active_estate
                .as_ref()
                .is_some_and(|a| a.land_ident == *land_ident);
            let matches_ward = connection
                .housing_sim
                .display_ward_context
                .is_some_and(|w| {
                    w.territory_type_id == est.territory_type_id as u16
                        && w.ward_index == est.ward_index as u8
                        && w.division == est.division.clamp(0, 1) as u8
                });
            let matches_owner = est.owner_content_id == connection.player_data.character.content_id;

            if matches_active || matches_ward || (*force_enter && matches_owner) {
                execute_housing_reload_with_source(
                    connection,
                    Some(*land_ident),
                    *mode,
                    *preserve_player_pos,
                    *force_enter && matches_owner,
                    source.as_deref(),
                )
                .await;
                send_owned_housing(connection).await;
                let intended = if is_house_interior_territory(current_zone_id)
                    || is_apartment_territory(current_zone_id)
                {
                    TerritoryIntendedUse::HousingIndoor
                } else if kawari::common::internal_housing_row(current_zone_id).is_some() {
                    TerritoryIntendedUse::HousingOutdoor
                } else {
                    TerritoryIntendedUse::Town
                };
                publish_session_snapshot(connection, intended);
            }
        }
        HousingSimFromServer::ApplyLiveMoves {
            land_ident,
            indoors,
            moves,
        } => {
            if moves.is_empty() {
                return;
            }
            let current_zone_id = connection.player_data.volatile.zone_id as u16;
            let currently_indoors = is_house_interior_territory(current_zone_id)
                || is_apartment_territory(current_zone_id);
            if *indoors != currently_indoors {
                return;
            }

            if *indoors {
                let Some(active) = connection.housing_sim.active_estate.clone() else {
                    return;
                };
                if active.land_ident != *land_ident {
                    return;
                }
                let catalog = connection_catalog(connection);
                let mut segments = Vec::with_capacity(moves.len());
                let mut overlay_moves = Vec::new();
                for m in moves {
                    let position = Position(glam::Vec3A::new(m.pos_x, m.pos_y, m.pos_z));
                    segments.push(ServerZoneIpcSegment::new(
                        ServerZoneIpcData::FurnitureTranslatedForObserver(
                            FurnitureTranslatedForObserver {
                                rotation: m.rotation,
                                plot_and_index: m.flat_slot,
                                outdoor_index: (m.flat_slot & 0xFF) as u8,
                                plot_number: (m.flat_slot >> 8) as u8,
                                unk1: [0; 2],
                                position,
                                unk2: [0; 4],
                            },
                        ),
                    ));
                    if let Some((ctype, slot_in_page)) =
                        placed_container_for_flat_slot(true, m.flat_slot)
                    {
                        mutate_synced_indoor_slot(
                            connection,
                            *land_ident,
                            ctype,
                            slot_in_page,
                            |s| {
                                s.pos_x = m.pos_x;
                                s.pos_y = m.pos_y;
                                s.pos_z = m.pos_z;
                                s.rotation = m.rotation;
                            },
                        );
                        if let Some(cat_id) = connection
                            .player_data
                            .house_inventory
                            .get_item(ctype, slot_in_page)
                            .filter(|i| i.item_id != 0)
                            .and_then(|i| catalog.furniture_catalog_id(i.item_id, true))
                            && catalog.is_interactable(cat_id, true)
                        {
                            overlay_moves.push((m.flat_slot, position, m.rotation));
                        }
                    }
                }
                if !segments.is_empty() {
                    connection.send_ipcs_self(&segments).await;
                }
                for (slot, position, rotation) in overlay_moves {
                    let _ = connection
                        .handle
                        .send(ToServer::HousingSim(
                            HousingSimToServer::TranslateSingleOverlay {
                                actor_id: connection.player_data.character.actor_id,
                                key: HousingOverlayKey {
                                    indoors: true,
                                    ward_index: active.ward_index,
                                    division: active.division,
                                    plot_index: active.plot_index,
                                    slot,
                                },
                                position,
                                rotation,
                            },
                        ))
                        .await;
                }
            } else {
                let estate = {
                    let mut db = connection.database.lock();
                    estate_by_land_ident(db.connection_mut(), *land_ident)
                        .ok()
                        .flatten()
                };
                let Some(est) = estate else {
                    return;
                };
                if est.is_apartment {
                    return;
                }
                let in_target_ward = connection
                    .housing_sim
                    .display_ward_context
                    .is_some_and(|w| {
                        w.territory_type_id == est.territory_type_id as u16
                            && w.ward_index == est.ward_index as u8
                            && w.division == est.division.clamp(0, 1) as u8
                    });
                if !in_target_ward {
                    return;
                }
                let raw_plot = raw_plot_index(&est);
                let segments: Vec<ServerZoneIpcSegment> = moves
                    .iter()
                    .map(|m| {
                        let plot_and_index = ((raw_plot as u16) << 8) | (m.flat_slot & 0x00FF);
                        ServerZoneIpcSegment::new(
                            ServerZoneIpcData::FurnitureTranslatedForObserver(
                                FurnitureTranslatedForObserver {
                                    rotation: m.rotation,
                                    plot_and_index,
                                    outdoor_index: (plot_and_index & 0xFF) as u8,
                                    plot_number: (plot_and_index >> 8) as u8,
                                    unk1: [0; 2],
                                    position: Position(glam::Vec3A::new(m.pos_x, m.pos_y, m.pos_z)),
                                    unk2: [0; 4],
                                },
                            ),
                        )
                    })
                    .collect();
                if !segments.is_empty() {
                    connection.send_ipcs_self(&segments).await;
                }
            }
        }
        HousingSimFromServer::SwitchActiveEstate {
            land_ident,
            enter_interior,
        } => {
            let estate = {
                let mut db = connection.database.lock();
                estate_by_land_ident(db.connection_mut(), *land_ident)
                    .ok()
                    .flatten()
            };
            let Some(est) = estate else {
                return;
            };

            let current_zone_id = connection.player_data.volatile.zone_id as u16;
            let currently_indoors = is_house_interior_territory(current_zone_id)
                || is_apartment_territory(current_zone_id);
            let go_indoor = enter_interior.unwrap_or(est.is_apartment || currently_indoors);

            connection.housing_sim.authoritative_ward_context = Some(super::session::WardContext {
                territory_type_id: est.territory_type_id as u16,
                ward_index: est.ward_index as u8,
                division: est.division.clamp(0, 1) as u8,
            });
            connection.housing_sim.active_estate = Some(build_selection_from_row(&est, go_indoor));
            connection.housing_sim.is_owner_authorized = true;
            rebuild_house_inventory_from_db(connection, &est, go_indoor);
            send_owned_housing(connection).await;
            let current_intended = if currently_indoors {
                TerritoryIntendedUse::HousingIndoor
            } else if kawari::common::internal_housing_row(current_zone_id).is_some() {
                TerritoryIntendedUse::HousingOutdoor
            } else {
                TerritoryIntendedUse::Town
            };
            publish_session_snapshot(connection, current_intended);

            connection.housing_sim.clear_pending_indoor_reentry();
            if go_indoor {
                warp_into_estate_interior(connection, &est, None, None).await;
            } else {
                if !currently_indoors
                    && !est.is_apartment
                    && current_zone_id == est.territory_type_id as u16
                    && connection
                        .housing_sim
                        .display_ward_context
                        .is_some_and(|ctx| {
                            ctx.ward_index == est.ward_index as u8
                                && ctx.division != est.division.clamp(0, 1) as u8
                        })
                {
                    connection.entering_subdivision = est.division != 0;
                    super::flows::init_outdoor_ward(
                        connection,
                        current_zone_id,
                        est.ward_index as u16,
                    )
                    .await;
                    sync_current_overlays_for_connection(
                        connection,
                        TerritoryIntendedUse::HousingOutdoor,
                    )
                    .await;
                }
                warp_to_estate_outdoor(connection, &est, None, None).await;
            }
        }
        HousingSimFromServer::UnlockCharacter => {
            unlock_online_connection(connection).await;
        }
        HousingSimFromServer::GiveSampleKit { kind } => {
            let added = give_sample_kit_to_connection(connection, kind).await;
            connection
                .send_notice(&format!(
                    "[Housing] Added {added} sample '{kind}' furnishings to inventory."
                ))
                .await;
        }
        HousingSimFromServer::PushOverlaySync => {
            let zone_id = connection.player_data.volatile.zone_id as u16;
            let intended =
                if is_house_interior_territory(zone_id) || is_apartment_territory(zone_id) {
                    TerritoryIntendedUse::HousingIndoor
                } else if kawari::common::internal_housing_row(zone_id).is_some() {
                    TerritoryIntendedUse::HousingOutdoor
                } else {
                    return;
                };
            sync_current_overlays_for_connection(connection, intended).await;
        }
    }
}

/// Handles `LuaTask::WarpHouse { house_id }` triggered from plot entrance scripts (`WarpHousingEnterHouse.lua`).
pub async fn handle_lua_warp_house(connection: &mut ZoneConnection, raw_house_id: u32) {
    connection.housing_sim.clear_pending_indoor_reentry();
    let zone_id = connection.player_data.volatile.zone_id as u16;
    let estate = if raw_house_id != 0 {
        let mut db = connection.database.lock();
        super::db::estate_by_house_id(db.connection_mut(), HouseId::from(raw_house_id as u64))
            .ok()
            .flatten()
    } else if let Some((div, plot)) = connection
        .housing_sim
        .last_entrance_plot
        .or_else(|| nearest_plot_to_position(zone_id, connection.player_data.volatile.position))
    {
        let district = default_district_for_zone(zone_id);
        let ward = connection
            .housing_sim
            .display_ward_context
            .map(|c| c.ward_index)
            .unwrap_or(0);
        let found = {
            let mut db = connection.database.lock();
            estate_by_location(
                db.connection_mut(),
                district,
                connection.config.world_id,
                ward,
                div,
                plot,
            )
            .ok()
            .flatten()
        };
        if found.is_some() {
            found
        } else {
            ensure_local_plot_estate(connection, district, ward, div, plot, None, false, None)
        }
    } else {
        load_active_estate(connection)
    };

    if let Some(est) = estate {
        warp_into_estate_interior(connection, &est, None, None).await;
    } else {
        connection
            .send_notice(
                "[Housing] No plot estate found. Use !housing testhouse or stand at a plot entrance.",
            )
            .await;
    }
}

/// Handles `LuaTask::HousingEnterApartment` triggered from apartment building entrances (`HouFixMansionEntrance_00359.lua`).
pub async fn handle_lua_enter_apartment(connection: &mut ZoneConnection) {
    connection.housing_sim.clear_pending_indoor_reentry();
    let zone_id = connection.player_data.volatile.zone_id as u16;
    let content_id = connection.player_data.character.content_id as u64;
    let (district, ward_index, division) = connection
        .housing_sim
        .display_ward_context
        .map(|c| (c.territory_type_id, c.ward_index, c.division))
        .unwrap_or((default_district_for_zone(zone_id), 0, 0));

    let existing = {
        let mut db = connection.database.lock();
        apartments_in_ward(
            db.connection_mut(),
            district,
            connection.config.world_id,
            ward_index,
            division,
        )
        .unwrap_or_default()
        .into_iter()
        .find(|a| a.owner_content_id == content_id as i64)
    };

    let estate = existing
        .or_else(|| ensure_local_apartment_estate(connection, district, ward_index, division, 1));

    if let Some(est) = estate {
        connection.housing_sim.authoritative_ward_context = Some(super::session::WardContext {
            territory_type_id: est.territory_type_id as u16,
            ward_index: est.ward_index as u8,
            division: est.division.clamp(0, 1) as u8,
        });
        connection.housing_sim.active_estate = Some(build_selection_from_row(&est, true));
        connection.housing_sim.is_owner_authorized = true;
        send_owned_housing(connection).await;
        warp_into_estate_interior(connection, &est, None, None).await;
    } else {
        connection
            .send_notice("[Housing] Failed to enter apartment room.")
            .await;
    }
}

/// Handles `LuaTask::HousingExitHouse` triggered from `WarpHousingExitHouse.lua` and `HouFixMansionExit_00362.lua`.
pub async fn handle_lua_housing_exit(connection: &mut ZoneConnection) {
    connection.housing_sim.clear_pending_indoor_reentry();
    let zone_id = connection.player_data.volatile.zone_id as u16;
    if is_apartment_lobby_territory(zone_id) || is_apartment_territory(zone_id) {
        if let Some(est) = load_active_estate(connection)
            && est.is_apartment
        {
            warp_to_estate_outdoor(connection, &est, None, None).await;
            return;
        }
        let (district, ward_index, division) = connection
            .housing_sim
            .display_ward_context
            .filter(|c| c.territory_type_id == default_district_for_zone(zone_id))
            .map(|c| (c.territory_type_id, c.ward_index, c.division))
            .unwrap_or((default_district_for_zone(zone_id), 0, 0));
        warp_to_apartment_outdoor(connection, district, ward_index, division).await;
        return;
    }

    if let Some(est) = load_active_estate(connection) {
        warp_to_estate_outdoor(connection, &est, None, None).await;
        return;
    }

    let aetheryte_id = city_fallback_aetheryte_for_territory(zone_id);
    let target = {
        let mut gd = connection.game_data.lock();
        gd.get_aetheryte(aetheryte_id)
    };
    let (dst_pop_range, dst_zone) = target.unwrap_or((0, default_district_for_zone(zone_id)));
    connection
        .handle
        .send(ToServer::WarpPopRange(
            connection.id,
            connection.player_data.character.actor_id,
            dst_zone,
            dst_pop_range,
        ))
        .await;
    let _ = (
        HousingSseEvent::new("", None, ""),
        ensure_local_apartment_estate,
        target_indoor_territory_for_estate,
        LuaTask::FinishDyeing,
    );
}
