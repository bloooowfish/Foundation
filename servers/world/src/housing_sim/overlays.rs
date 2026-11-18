use kawari::common::{ObjectId, Position, STRIKING_DUMMY_NAME_ID};
use kawari::config::get_config;
use kawari::ipc::zone::{
    BattleNpcSubKind, CharacterDataFlag, CommonSpawn, ObjectKind, SpawnNpc, SpawnObject,
};

use crate::GameData;
use crate::common::FromServer;
use crate::server::actor::NetworkedActor;
use crate::server::instance::Instance;
use crate::server::network::{DestinationNetwork, NetworkState};

use super::gamedata_ext::HousingCatalog;

const HOUSING_ENTITY_PREFIX: u32 = 0x4000_0000;
const HOUSING_STRIKING_DUMMY_ENTITY_PREFIX: u32 = 0x6B00_0000;
const HOUSING_STRIKING_DUMMY_GIMMICK_PREFIX: u32 = 0x6C00_0000;
const HOUSING_OBJECT_TYPE_YARD_OBJECT: u32 = 2;
const HOUSING_OBJECT_TYPE_FURNITURE: u32 = 3;
const HOUSING_STRIKING_DUMMY_LEVEL: u8 = 1;
const HOUSING_STRIKING_DUMMY_LINK_RANGE: u8 = 20;
const HOUSING_STRIKING_DUMMY_HEALTH: u32 = 1_000_000_000;

/// Scope matching interactive furniture overlays inside an `Instance`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde_derive::Serialize, serde_derive::Deserialize)]
pub struct HousingOverlayScope {
    pub indoors: bool,
    pub ward_index: u8,
    pub plot_index: u8,
}

/// Unique key identifying one placed furnishing overlay (`0x4000_xxxx` or `0x6B00_xxxx`).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, serde_derive::Serialize, serde_derive::Deserialize,
)]
pub struct HousingOverlayKey {
    pub indoors: bool,
    pub ward_index: u8,
    pub plot_index: u8,
    pub slot: u16,
}

/// Serialized descriptor of a placed furnishing used to spawn interactive `HousingEventObject` or `BattleNpc` dummies.
#[derive(Clone, Copy, Debug, PartialEq, serde_derive::Serialize, serde_derive::Deserialize)]
pub struct HousingOverlayObject {
    pub key: HousingOverlayKey,
    pub catalog_id: u16,
    pub position: Position,
    pub rotation: f32,
}

pub fn tracking_id_for_key(key: HousingOverlayKey) -> u32 {
    if key.indoors {
        key.slot as u32
    } else {
        ((key.ward_index as u32) << 16)
            | ((key.plot_index as u32) << 8)
            | ((key.slot as u32) & 0x00FF)
    }
}

pub fn overlay_object_actor_id(key: HousingOverlayKey) -> ObjectId {
    ObjectId(HOUSING_ENTITY_PREFIX | tracking_id_for_key(key))
}

pub fn overlay_dummy_actor_id(key: HousingOverlayKey) -> ObjectId {
    ObjectId(HOUSING_STRIKING_DUMMY_ENTITY_PREFIX | tracking_id_for_key(key))
}

fn tracking_id_from_actor_id(actor_id: ObjectId) -> Option<u32> {
    let prefix = actor_id.0 & 0xFF00_0000;
    if prefix == HOUSING_ENTITY_PREFIX || prefix == HOUSING_STRIKING_DUMMY_ENTITY_PREFIX {
        Some(actor_id.0 & 0x00FF_FFFF)
    } else {
        None
    }
}

fn actor_matches_scope(actor_id: ObjectId, scope: HousingOverlayScope) -> bool {
    let Some(tracking_id) = tracking_id_from_actor_id(actor_id) else {
        return false;
    };
    if scope.indoors {
        true
    } else {
        ((tracking_id >> 16) as u8) == scope.ward_index
            && ((tracking_id >> 8) as u8) == scope.plot_index
    }
}

pub fn build_housing_event_object_spawn(
    obj: HousingOverlayObject,
    interactable: bool,
) -> Option<(ObjectId, SpawnObject)> {
    if !interactable || obj.catalog_id == 0 {
        return None;
    }
    let actor_id = overlay_object_actor_id(obj.key);
    let obj_type = if obj.key.indoors {
        HOUSING_OBJECT_TYPE_FURNITURE
    } else {
        HOUSING_OBJECT_TYPE_YARD_OBJECT
    };
    let base_id = (obj_type << 16) | (obj.catalog_id as u32);
    Some((
        actor_id,
        SpawnObject {
            kind: ObjectKind::HousingEventObject,
            not_targetable: false,
            base_id,
            entity_id: actor_id,
            args2: tracking_id_for_key(obj.key),
            radius: 1.0,
            rotation: obj.rotation,
            position: obj.position,
            ..Default::default()
        },
    ))
}

pub fn build_striking_dummy_spawn(
    obj: HousingOverlayObject,
    data: &super::gamedata_ext::HousingStrikingDummyNpcData,
) -> Option<(ObjectId, SpawnNpc)> {
    if obj.key.indoors {
        return None;
    }
    let actor_id = overlay_dummy_actor_id(obj.key);
    let gimmick_id = HOUSING_STRIKING_DUMMY_GIMMICK_PREFIX | tracking_id_for_key(obj.key);
    Some((
        actor_id,
        SpawnNpc {
            gimmick_id,
            character_data_flags: CharacterDataFlag::empty(),
            character_data_icon: data.rank,
            normal_scaling: false,
            max_links: 0,
            link_family: 0,
            link_range: HOUSING_STRIKING_DUMMY_LINK_RANGE,
            common: CommonSpawn {
                base_id: data.base_id,
                name_id: STRIKING_DUMMY_NAME_ID,
                max_health_points: HOUSING_STRIKING_DUMMY_HEALTH,
                health_points: HOUSING_STRIKING_DUMMY_HEALTH,
                model_chara: data.model_chara,
                object_kind: ObjectKind::BattleNpc(BattleNpcSubKind::Enemy),
                battalion: data.battalion,
                level: HOUSING_STRIKING_DUMMY_LEVEL,
                look: data.customize.clone(),
                position: obj.position,
                rotation: obj.rotation,
                ..data.equip.clone()
            },
            ..Default::default()
        },
    ))
}

/// Replaces all interactive housing overlays matching `scope` inside `instance` and syncs connected clients.
pub fn replace_scope_overlays(
    instance: &mut Instance,
    network: &mut NetworkState,
    game_data: &mut GameData,
    catalog: &HousingCatalog,
    scope: HousingOverlayScope,
    objects: &[HousingOverlayObject],
) {
    let stale_ids: Vec<ObjectId> = instance
        .actors
        .keys()
        .copied()
        .filter(|&id| actor_matches_scope(id, scope))
        .collect();

    for stale in stale_ids {
        network.remove_actor(instance, stale);
    }

    let config = get_config();
    let mut inserted_ids = Vec::new();
    for &obj in objects {
        if !obj.key.indoors
            && let Some(dummy_data) = catalog.striking_dummy_npc_data(game_data, obj.catalog_id)
            && let Some((actor_id, npc_spawn)) = build_striking_dummy_spawn(obj, &dummy_data)
        {
            instance.insert_npc(actor_id, npc_spawn, &config, game_data);
            inserted_ids.push(actor_id);
        } else if catalog.is_interactable(obj.catalog_id, obj.key.indoors)
            && let Some((actor_id, obj_spawn)) = build_housing_event_object_spawn(obj, true)
        {
            instance.insert_object(actor_id, obj_spawn, String::new());
            inserted_ids.push(actor_id);
        }
    }

    push_new_spawns_to_instance_clients(instance, network, &inserted_ids);
}

/// Spawns or updates a single furniture overlay after interactive placement.
pub fn upsert_single_overlay(
    instance: &mut Instance,
    network: &mut NetworkState,
    game_data: &mut GameData,
    catalog: &HousingCatalog,
    obj: HousingOverlayObject,
) {
    remove_single_overlay(instance, network, obj.key);

    let config = get_config();
    let mut inserted = Vec::new();
    if !obj.key.indoors
        && let Some(dummy_data) = catalog.striking_dummy_npc_data(game_data, obj.catalog_id)
        && let Some((actor_id, npc_spawn)) = build_striking_dummy_spawn(obj, &dummy_data)
    {
        instance.insert_npc(actor_id, npc_spawn, &config, game_data);
        inserted.push(actor_id);
    } else if catalog.is_interactable(obj.catalog_id, obj.key.indoors)
        && let Some((actor_id, obj_spawn)) = build_housing_event_object_spawn(obj, true)
    {
        instance.insert_object(actor_id, obj_spawn, String::new());
        inserted.push(actor_id);
    }
    push_new_spawns_to_instance_clients(instance, network, &inserted);
}

/// Moves/rotates an existing overlay (including striking dummies) in-place inside `instance`.
pub fn translate_single_overlay(
    instance: &mut Instance,
    network: &mut NetworkState,
    key: HousingOverlayKey,
    position: Position,
    rotation: f32,
) {
    for actor_id in [overlay_object_actor_id(key), overlay_dummy_actor_id(key)] {
        if let Some(actor) = instance.actors.get_mut(&actor_id) {
            match actor {
                NetworkedActor::Object { object, .. } => {
                    object.position = position;
                    object.rotation = rotation;
                }
                NetworkedActor::Npc { spawn, .. } => {
                    spawn.common.position = position;
                    spawn.common.rotation = rotation;
                    network.send_in_range_instance(
                        actor_id,
                        instance,
                        FromServer::ActorMove(
                            actor_id,
                            position,
                            rotation,
                            kawari::common::MoveAnimationType::default(),
                            kawari::common::MoveAnimationState::None,
                            kawari::common::JumpState::NoneOrFalling,
                        ),
                        DestinationNetwork::ZoneClients,
                    );
                }
                _ => {}
            }
        }
    }
}

/// Removes both event-object and striking-dummy overlays matching `key`.
pub fn remove_single_overlay(
    instance: &mut Instance,
    network: &mut NetworkState,
    key: HousingOverlayKey,
) {
    for actor_id in [overlay_object_actor_id(key), overlay_dummy_actor_id(key)] {
        if instance.actors.contains_key(&actor_id) {
            network.remove_actor(instance, actor_id);
        }
    }
}

fn push_new_spawns_to_instance_clients(
    instance: &Instance,
    network: &mut NetworkState,
    actor_ids: &[ObjectId],
) {
    if actor_ids.is_empty() {
        return;
    }
    for &target_id in actor_ids {
        let Some(actor) = instance.actors.get(&target_id) else {
            continue;
        };
        for (client_id, (handle, state)) in &mut network.clients {
            if !instance.actors.contains_key(&handle.actor_id) {
                continue;
            }
            if state.has_spawned(target_id) {
                continue;
            }
            if let Some(msg) = NetworkState::spawn_existing_actor_message(state, target_id, actor)
                && handle.send(msg).is_err()
            {
                network.to_remove.push(*client_id);
            }
        }
    }
}
