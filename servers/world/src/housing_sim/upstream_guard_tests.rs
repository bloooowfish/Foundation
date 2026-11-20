use glam::Vec3A;
use kawari::common::{ContainerType, ObjectId, TerritoryIntendedUse, WarpType};
use kawari::packet::PacketHeader;
use tokio::sync::mpsc::channel;

use crate::Bitmask;
use crate::common::{ClientId, FromServer};
use crate::housing_sim::hooks::{
    drain_complete_client_packets, should_load_navmesh_for_intended_use, should_retain_instance,
};
use crate::inventory::{HousingInventory, Inventory};
use crate::server::instance::{Instance, NavmeshGenerationStep};
use crate::server::network::NetworkState;
use crate::server::zone::{HousingPlot, Zone, do_change_zone};

fn packet_bytes(payload_len: usize, fill: u8) -> Vec<u8> {
    let header_size = std::mem::size_of::<PacketHeader>();
    let packet_size = header_size + payload_len;
    let mut bytes = vec![fill; packet_size];
    bytes[..header_size].fill(0);
    bytes[24..28].copy_from_slice(&(packet_size as u32).to_le_bytes());
    bytes
}

#[test]
fn drains_multiple_complete_packets_from_one_read_buffer() {
    let first = packet_bytes(2, 0x11);
    let second = packet_bytes(3, 0x22);
    let mut pending = [first.clone(), second.clone()].concat();

    let packets = drain_complete_client_packets(&mut pending);

    assert_eq!(packets, vec![first, second]);
    assert!(pending.is_empty());
}

#[test]
fn leaves_partial_packet_buffered_until_more_bytes_arrive() {
    let complete = packet_bytes(2, 0x11);
    let partial = packet_bytes(4, 0x22);
    let partial_prefix = partial[..partial.len() - 1].to_vec();
    let mut pending = [complete.clone(), partial_prefix.clone()].concat();

    let packets = drain_complete_client_packets(&mut pending);

    assert_eq!(packets, vec![complete]);
    assert_eq!(pending, partial_prefix);
}

#[test]
fn housing_plot_exit_transform_nudges_eight_yalms_with_radian_rotation() {
    let mut zone = Zone::default();
    zone.cached_housing_plots.push(HousingPlot {
        entrance_position: Vec3A::new(1.0, 2.0, 3.0),
        entrance_rotation: std::f32::consts::FRAC_PI_2,
    });

    let (pos, rot) = zone
        .housing_plot_exit_transform(0)
        .expect("cached plot must resolve exit transform");
    assert!((pos.0.x - 9.0).abs() < 1e-4);
    assert!((pos.0.y - 2.0).abs() < 1e-4);
    assert!((pos.0.z - 3.0).abs() < 1e-4);
    assert!((rot - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
}

#[test]
fn real_758_housing_ward_loads_sixty_plot_gates_with_valid_radian_rotations() {
    let Some(game_data) = crate::test_game_data() else {
        return;
    };
    let zone = Zone::load(game_data, 339);
    assert_eq!(zone.cached_housing_plots.len(), 60);
    for raw_idx in 0..60usize {
        let (pos, rot) = zone
            .housing_plot_exit_transform(raw_idx)
            .expect("plot exit transform must exist");
        assert!(
            (-std::f32::consts::PI..=std::f32::consts::PI).contains(&rot),
            "rotation {rot} out of radian quantization bounds"
        );
        assert!(pos.0.length() > 0.0);
    }
}

#[test]
fn do_change_zone_skips_missing_client_without_panicking() {
    let mut network = NetworkState::default();
    let mut target_instance = Instance::default();
    do_change_zone(
        &mut network,
        &mut target_instance,
        true,
        None,
        None,
        ClientId::default(),
        WarpType::Normal,
    );
    assert!(target_instance.queued_tasks.is_empty());
    assert!(network.to_remove.is_empty());
}

#[test]
fn cleanup_dead_instances_retains_player_and_active_navmesh_instances() {
    let mut empty = Instance::default();
    assert!(!should_retain_instance(&empty));

    empty.generate_navmesh = NavmeshGenerationStep::Needed("missing.nvm".to_string());
    assert!(!should_retain_instance(&empty));

    let mut with_player = Instance::default();
    with_player.insert_empty_actor(ObjectId(1));
    assert!(should_retain_instance(&with_player));

    with_player.generate_navmesh = NavmeshGenerationStep::Needed("pending.nvm".to_string());
    assert!(should_retain_instance(&with_player));
}

#[test]
fn skips_navmesh_for_housing_and_inn_territories() {
    assert!(!should_load_navmesh_for_intended_use(
        TerritoryIntendedUse::HousingIndoor
    ));
    assert!(!should_load_navmesh_for_intended_use(
        TerritoryIntendedUse::HousingOutdoor
    ));
    assert!(!should_load_navmesh_for_intended_use(
        TerritoryIntendedUse::Inn
    ));

    assert!(should_load_navmesh_for_intended_use(
        TerritoryIntendedUse::Overworld
    ));
    assert!(should_load_navmesh_for_intended_use(
        TerritoryIntendedUse::Dungeon
    ));
    assert!(should_load_navmesh_for_intended_use(
        TerritoryIntendedUse::Town
    ));
}

#[test]
fn client_handle_send_drops_on_full_without_disconnecting_and_errors_on_closed() {
    let (tx, rx) = channel(1);
    let mut handle = crate::ClientHandle {
        id: ClientId::default(),
        channel: tx,
        actor_id: ObjectId(100),
        content_id: 42,
        account_id: 1,
    };

    assert!(handle.send(FromServer::ChatDisconnected).is_ok());
    assert!(handle.send(FromServer::ChatDisconnected).is_ok());

    drop(rx);
    let err = handle.send(FromServer::ChatDisconnected).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
}

#[test]
fn legacy_short_bitmask_is_bounds_safe() {
    let mut bitmask: Bitmask<8> = Bitmask::from(vec![1u8, 0u8]);
    assert_eq!(bitmask.data.len(), 8);
    assert!(bitmask.contains(0));
    assert!(!bitmask.contains(5000));
    bitmask.clear(5000);
    bitmask.set(100);
    assert!(bitmask.contains(100));
    assert!(bitmask.toggle(120));
    assert!(bitmask.contains(120));
}

#[test]
fn out_of_range_slot_indices_return_none_without_panicking() {
    let mut inv = Inventory::default();
    assert!(inv.get_item(ContainerType::Inventory0, 35).is_none());
    assert!(inv.get_item_mut(ContainerType::Inventory0, 999).is_none());
    assert!(inv.get_item(ContainerType::Equipped, 50).is_none());
    assert!(inv.get_item_mut(ContainerType::Equipped, 50).is_none());
    assert!(inv.get_item(ContainerType::Crystals, 18).is_none());
    assert!(inv.get_item_mut(ContainerType::Currency, 99).is_none());

    let mut hinv = HousingInventory::default();
    assert!(
        hinv.get_item(ContainerType::HousingInteriorPlacedItems1, 50)
            .is_none()
    );
    assert!(
        hinv.get_item(ContainerType::HousingExteriorPlacedItems, 39)
            .is_some()
    );
    assert!(
        hinv.get_item(ContainerType::HousingExteriorPlacedItems, 40)
            .is_none()
    );
    assert!(
        hinv.get_item(ContainerType::HousingExteriorPlacedItems2, 39)
            .is_some()
    );
    assert!(
        hinv.get_item_mut(ContainerType::HousingExteriorPlacedItems2, 40)
            .is_none()
    );
    assert!(
        hinv.get_item_mut(ContainerType::HousingExteriorPlacedItems, u16::MAX)
            .is_none()
    );
}
