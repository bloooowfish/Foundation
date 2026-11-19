use std::collections::BTreeMap;
use std::f32::consts::{PI, TAU};

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

#[derive(Clone)]
enum DesiredOverlay {
    Object(SpawnObject),
    Npc(SpawnNpc),
}

fn minimal_angle_diff(a: f32, b: f32) -> f32 {
    ((a - b + PI).rem_euclid(TAU) - PI).abs()
}

fn overlay_matches_existing(existing: &NetworkedActor, desired: &DesiredOverlay) -> bool {
    const POS_EPSILON_SQ: f32 = 1e-6;
    const ROT_EPSILON: f32 = 1e-3;
    match (existing, desired) {
        (NetworkedActor::Object { object, .. }, DesiredOverlay::Object(target)) => {
            object.base_id == target.base_id
                && object.position.0.distance_squared(target.position.0) <= POS_EPSILON_SQ
                && minimal_angle_diff(object.rotation, target.rotation) <= ROT_EPSILON
        }
        (NetworkedActor::Npc { spawn, .. }, DesiredOverlay::Npc(target)) => {
            spawn.common.base_id == target.common.base_id
                && spawn
                    .common
                    .position
                    .0
                    .distance_squared(target.common.position.0)
                    <= POS_EPSILON_SQ
                && minimal_angle_diff(spawn.common.rotation, target.common.rotation) <= ROT_EPSILON
        }
        _ => false,
    }
}

/// Diff-replaces interactive housing overlays matching `scope` inside `instance` and syncs connected clients.
pub fn replace_scope_overlays(
    instance: &mut Instance,
    network: &mut NetworkState,
    game_data: &mut GameData,
    catalog: &HousingCatalog,
    scope: HousingOverlayScope,
    objects: &[HousingOverlayObject],
) {
    replace_scope_overlays_inner(instance, network, Some(game_data), catalog, scope, objects);
}

fn replace_scope_overlays_inner(
    instance: &mut Instance,
    network: &mut NetworkState,
    mut game_data: Option<&mut GameData>,
    catalog: &HousingCatalog,
    scope: HousingOverlayScope,
    objects: &[HousingOverlayObject],
) {
    let mut desired: BTreeMap<u32, (ObjectId, DesiredOverlay)> = BTreeMap::new();
    for &obj in objects {
        if !obj.key.indoors
            && let Some(ref mut gd) = game_data
            && let Some(dummy_data) = catalog.striking_dummy_npc_data(gd, obj.catalog_id)
            && let Some((actor_id, npc_spawn)) = build_striking_dummy_spawn(obj, &dummy_data)
        {
            desired.insert(actor_id.0, (actor_id, DesiredOverlay::Npc(npc_spawn)));
        } else if catalog.is_interactable(obj.catalog_id, obj.key.indoors)
            && let Some((actor_id, obj_spawn)) = build_housing_event_object_spawn(obj, true)
        {
            desired.insert(actor_id.0, (actor_id, DesiredOverlay::Object(obj_spawn)));
        }
    }

    let stale_ids: Vec<ObjectId> = instance
        .actors
        .iter()
        .filter(|(id, actor)| {
            actor_matches_scope(**id, scope)
                && desired
                    .get(&id.0)
                    .is_none_or(|(_, target)| !overlay_matches_existing(actor, target))
        })
        .map(|(&id, _)| id)
        .collect();

    for stale in stale_ids {
        network.remove_actor(instance, stale);
    }

    let mut desired_ids = Vec::with_capacity(desired.len());
    for (_, (actor_id, desired_overlay)) in desired {
        if !instance.actors.contains_key(&actor_id) {
            match desired_overlay {
                DesiredOverlay::Object(obj_spawn) => {
                    instance.insert_object(actor_id, obj_spawn, String::new());
                }
                DesiredOverlay::Npc(npc_spawn) => {
                    if let Some(ref mut gd) = game_data {
                        let config = get_config();
                        instance.insert_npc(actor_id, npc_spawn, &config, gd);
                    }
                }
            }
        }
        desired_ids.push(actor_id);
    }

    push_new_spawns_to_instance_clients(instance, network, &desired_ids);
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

    let mut inserted = Vec::new();
    if !obj.key.indoors
        && let Some(dummy_data) = catalog.striking_dummy_npc_data(game_data, obj.catalog_id)
        && let Some((actor_id, npc_spawn)) = build_striking_dummy_spawn(obj, &dummy_data)
    {
        let config = get_config();
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
    enum TranslateAction {
        UpdateObject(SpawnObject),
        MoveNpc,
    }

    for actor_id in [overlay_object_actor_id(key), overlay_dummy_actor_id(key)] {
        let action = match instance.actors.get_mut(&actor_id) {
            Some(NetworkedActor::Object { object, .. }) => {
                let mut updated = *object;
                updated.position = position;
                updated.rotation = rotation;
                Some(TranslateAction::UpdateObject(updated))
            }
            Some(NetworkedActor::Npc { spawn, .. }) => {
                spawn.common.position = position;
                spawn.common.rotation = rotation;
                Some(TranslateAction::MoveNpc)
            }
            _ => None,
        };

        match action {
            Some(TranslateAction::UpdateObject(updated)) => {
                network.remove_actor(instance, actor_id);
                instance.insert_object(actor_id, updated, String::new());
                push_new_spawns_to_instance_clients(instance, network, &[actor_id]);
            }
            Some(TranslateAction::MoveNpc) => {
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
            None => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ClientState;
    use crate::{ClientHandle, ClientId};
    use tokio::sync::mpsc::channel;

    fn dummy_overlay(slot: u16, catalog_id: u16, x: f32) -> HousingOverlayObject {
        HousingOverlayObject {
            key: HousingOverlayKey {
                indoors: true,
                ward_index: 0,
                plot_index: 2,
                slot,
            },
            catalog_id,
            position: Position(glam::Vec3A::new(x, 0.0, 0.0)),
            rotation: 0.0,
        }
    }

    #[test]
    fn replace_scope_overlays_diffs_without_churning_unchanged_objects() {
        let mut catalog = HousingCatalog::mock_for_tests();
        let mut objects = Vec::new();
        for slot in 0..45u16 {
            let catalog_id = 1000 + slot;
            catalog
                .interactable_indoor_catalogs
                .insert(catalog_id, true);
            objects.push(dummy_overlay(slot, catalog_id, slot as f32));
        }

        let mut instance = Instance::default();
        let player_actor_id = ObjectId(1721199235);
        instance.insert_empty_actor(player_actor_id);

        let client_id = ClientId::default();
        let (tx, mut rx) = channel(64);
        let mut network = NetworkState::default();
        network.clients.insert(
            client_id,
            (
                ClientHandle {
                    id: client_id,
                    channel: tx,
                    actor_id: player_actor_id,
                    content_id: 1,
                    account_id: 1,
                },
                ClientState::default(),
            ),
        );

        let scope = HousingOverlayScope {
            indoors: true,
            ward_index: 0,
            plot_index: 2,
        };

        replace_scope_overlays_inner(&mut instance, &mut network, None, &catalog, scope, &objects);
        let mut initial_msgs = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            initial_msgs.push(msg);
        }
        assert_eq!(initial_msgs.len(), kawari::common::MAX_SPAWNED_OBJECTS);
        assert!(network.to_remove.is_empty());

        // Move one already-spawned object (slot 5) while leaving the other 44 unchanged.
        objects[5].position = Position(glam::Vec3A::new(99.0, 1.0, -4.0));
        replace_scope_overlays_inner(&mut instance, &mut network, None, &catalog, scope, &objects);

        let mut delta_msgs = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            delta_msgs.push(msg);
        }
        assert_eq!(delta_msgs.len(), 2);
        assert!(matches!(delta_msgs[0], FromServer::DeleteObject(_)));
        assert!(matches!(delta_msgs[1], FromServer::SpawnObject(_)));
        assert!(network.to_remove.is_empty());
        assert!(network.clients.contains_key(&client_id));
    }

    #[test]
    fn translate_single_overlay_updates_object_in_place_for_clients() {
        let mut catalog = HousingCatalog::mock_for_tests();
        catalog.interactable_indoor_catalogs.insert(1000, true);
        catalog.interactable_indoor_catalogs.insert(1001, true);
        let objects = vec![dummy_overlay(0, 1000, 1.0), dummy_overlay(1, 1001, 2.0)];

        let mut instance = Instance::default();
        let player_actor_id = ObjectId(42);
        instance.insert_empty_actor(player_actor_id);

        let client_id = ClientId::default();
        let (tx, mut rx) = channel(64);
        let mut network = NetworkState::default();
        network.clients.insert(
            client_id,
            (
                ClientHandle {
                    id: client_id,
                    channel: tx,
                    actor_id: player_actor_id,
                    content_id: 1,
                    account_id: 1,
                },
                ClientState::default(),
            ),
        );

        let scope = HousingOverlayScope {
            indoors: true,
            ward_index: 0,
            plot_index: 2,
        };
        replace_scope_overlays_inner(&mut instance, &mut network, None, &catalog, scope, &objects);
        while rx.try_recv().is_ok() {}

        let new_pos = Position(glam::Vec3A::new(12.5, 3.0, -7.5));
        let new_rot = 1.25;
        translate_single_overlay(
            &mut instance,
            &mut network,
            objects[1].key,
            new_pos,
            new_rot,
        );

        let mut sent = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            sent.push(msg);
        }
        assert_eq!(sent.len(), 2);
        assert!(matches!(sent[0], FromServer::DeleteObject(_)));
        assert!(matches!(
            sent[1],
            FromServer::SpawnObject(obj)
                if obj.position.0 == new_pos.0 && (obj.rotation - new_rot).abs() < 1e-6
        ));

        let actor = instance
            .actors
            .get(&overlay_object_actor_id(objects[1].key))
            .expect("translated overlay must remain in instance");
        assert_eq!(actor.position().0, new_pos.0);
        assert!((actor.rotation() - new_rot).abs() < 1e-6);
    }
}
