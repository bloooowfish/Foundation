use std::process::Child;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use kawari::common::{Position, RECEIVE_BUFFER_SIZE, TerritoryIntendedUse};
use kawari::ipc::chat::ClientChatIpcData;
use kawari::ipc::zone::ServerZoneIpcSegment;
use kawari::packet::{
    ConnectionType, PacketHeader, PacketSegment, SegmentData, SegmentType, parse_packet_header,
    send_packet,
};
use parking_lot::Mutex;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::time::Instant;

use crate::server::{
    actor::NetworkedActor,
    instance::{Instance, NavmeshGenerationStep},
    zone::Zone,
};
use crate::{
    ChatConnection, GameData, Navmesh, ServerHandle, ToServer, WorldDatabase, ZoneConnection,
};

use super::session::{HousingSimHub, HousingSimToServer};

pub fn drain_complete_client_packets(pending: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let header_size = std::mem::size_of::<PacketHeader>();
    let mut packets = Vec::new();

    while pending.len() >= header_size {
        let header = parse_packet_header(&pending[..header_size]);
        let packet_size = header.size as usize;

        if !(header_size..=RECEIVE_BUFFER_SIZE).contains(&packet_size) {
            tracing::warn!(
                pending_bytes = pending.len(),
                packet_size,
                "Discarding buffered client bytes with invalid packet size"
            );
            pending.clear();
            break;
        }

        if pending.len() < packet_size {
            break;
        }

        packets.push(pending[..packet_size].to_vec());
        pending.drain(..packet_size);
    }

    packets
}

pub async fn read_first_client_packet(
    socket: &mut TcpStream,
    pending: &mut Vec<u8>,
) -> std::io::Result<Option<Vec<u8>>> {
    let header_size = std::mem::size_of::<PacketHeader>();
    let mut chunk = vec![0; RECEIVE_BUFFER_SIZE];

    loop {
        if pending.len() >= header_size {
            let header = parse_packet_header(&pending[..header_size]);
            let packet_size = header.size as usize;
            if !(header_size..=RECEIVE_BUFFER_SIZE).contains(&packet_size) {
                tracing::warn!(
                    pending_bytes = pending.len(),
                    packet_size,
                    "Closing initial connection with invalid packet size"
                );
                pending.clear();
                return Ok(None);
            }
            if pending.len() >= packet_size {
                let first_packet = pending.drain(..packet_size).collect();
                return Ok(Some(first_packet));
            }
        }

        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        pending.extend_from_slice(&chunk[..n]);
    }
}

pub async fn handle_chat_packet(connection: &mut ChatConnection, packet: &[u8]) {
    let segments = connection.parse_packet(packet);
    for segment in segments {
        match &segment.data {
            SegmentData::None() => {}
            SegmentData::Setup { .. } => {
                // Handled before our connection was spawned!
            }
            SegmentData::Ipc(data) => match &data.data {
                ClientChatIpcData::SendTellMessage(tell_data) => {
                    connection.send_tell_message(tell_data).await;
                }
                ClientChatIpcData::SendPartyMessage(data) => {
                    connection.send_party_message(data).await;
                }
                ClientChatIpcData::GetChannelList { unk } => {
                    tracing::info!(
                        "GetChannelList: {:#?} from {}",
                        unk,
                        connection.player_data.actor_id
                    );
                }
                ClientChatIpcData::SendCWLinkshellMessage(data) => {
                    connection.send_linkshell_message(data).await;
                }
                ClientChatIpcData::SendAllianceMessage(_data) => {
                    tracing::info!("Chatting in alliances is unimplemented");
                }
                ClientChatIpcData::DiceRollCWLS(data) | ClientChatIpcData::DiceRollParty(data) => {
                    connection.send_dice_roll(*data).await;
                }
                ClientChatIpcData::Unknown { unk } => {
                    tracing::warn!(
                        "Unknown chat packet {:?} recieved ({} bytes)",
                        data.header.op_code,
                        unk.len()
                    );
                }
            },
            SegmentData::KeepAliveRequest { id, timestamp } => {
                connection.send_keep_alive(*id, *timestamp).await
            }
            SegmentData::KeepAliveResponse { .. } => {
                // these should be safe to ignore
            }
            _ => panic!(
                "ChatConnection: The server is receiving a response or an unknown packet: {segment:#?}"
            ),
        }
    }
}

pub async fn try_send_login_ward_spawn(connection: &mut ZoneConnection) -> bool {
    if connection.player_data.unlock.cutscene_seen.contains(2)
        && let Some(login_ward) = connection.housing_sim.login_ward_spawn.take()
    {
        connection
            .handle
            .send(ToServer::HousingSim(HousingSimToServer::SpawnLoginInWard {
                client_id: connection.id,
                actor_id: connection.player_data.character.actor_id,
                territory_type_id: login_ward.territory_type_id,
                ward_index: login_ward.ward_index,
                division: login_ward.division,
                plot_index: login_ward.plot_index,
                is_apartment_exit: login_ward.is_apartment_exit,
                fallback_pos: connection.player_data.volatile.position,
                fallback_rot: connection.player_data.volatile.rotation as f32,
            }))
            .await;
        return true;
    }
    false
}

fn housing_deadlines(connection: &ZoneConnection) -> (Option<Instant>, Option<Instant>) {
    let deferred_deadline = connection
        .housing_sim
        .pending_indoor_finish_loading_since
        .map(|since| since + Duration::from_millis(1500));
    let reentry_deadline = connection
        .housing_sim
        .pending_same_zone_indoor_reentry
        .as_ref()
        .and_then(|r| r.ready_at);
    (deferred_deadline, reentry_deadline)
}

pub fn next_housing_deadline(connection: &ZoneConnection) -> Option<Instant> {
    let (deferred_deadline, reentry_deadline) = housing_deadlines(connection);
    match (deferred_deadline, reentry_deadline) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

pub async fn sleep_until_optional(deadline: Option<Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline).await;
    }
}

pub async fn handle_housing_deadline(connection: &mut ZoneConnection) {
    let (deferred_deadline, reentry_deadline) = housing_deadlines(connection);
    let now = Instant::now();
    if deferred_deadline.is_some_and(|deadline| now >= deadline) {
        super::flows::flush_deferred_indoor_tail_on_timeout(connection).await;
    }
    if reentry_deadline.is_some_and(|deadline| now >= deadline) {
        super::flows::complete_pending_same_zone_indoor_reentry(connection).await;
    }
}

pub fn on_client_disconnect(connection: &ZoneConnection) {
    if let Some(ref hub) = connection.housing_hub {
        hub.remove_session(connection.id);
    }
}

pub fn init_world_http(
    handle: ServerHandle,
    database: Arc<Mutex<WorldDatabase>>,
    game_data: Arc<Mutex<GameData>>,
    app: Router,
) -> (Arc<HousingSimHub>, Router) {
    let housing_hub = Arc::new(HousingSimHub::new(handle, database, game_data));
    let app = app
        .merge(super::web_ui::router())
        .nest("/api/v1/housing", super::http::router(housing_hub.clone()));
    (housing_hub, app)
}

impl ZoneConnection {
    /// Sends multiple `ServerZoneIpcSegment` items addressed to self in bounded multi-segment packets.
    pub async fn send_ipcs_self(&mut self, ipcs: &[ServerZoneIpcSegment]) {
        const MAX_SEGMENTS_PER_PACKET: usize = 16;
        const MAX_BATCH_BYTES: usize = 8_192;

        let actor_id = self.player_data.character.actor_id;
        let mut batch = Vec::with_capacity(MAX_SEGMENTS_PER_PACKET);
        let mut batch_bytes = 0usize;

        for ipc in ipcs {
            let segment = PacketSegment {
                source_actor: actor_id,
                target_actor: actor_id,
                segment_type: SegmentType::Ipc,
                data: SegmentData::Ipc(ipc.clone()),
            };
            let seg_size = segment.calc_size() as usize;

            if !batch.is_empty()
                && (batch.len() >= MAX_SEGMENTS_PER_PACKET
                    || batch_bytes + seg_size > MAX_BATCH_BYTES)
            {
                Box::pin(send_packet(
                    &mut self.socket,
                    &mut self.state,
                    ConnectionType::Zone,
                    self.config.compression(),
                    &batch,
                ))
                .await;
                batch.clear();
                batch_bytes = 0;
            }

            batch_bytes += seg_size;
            batch.push(segment);
        }

        if !batch.is_empty() {
            Box::pin(send_packet(
                &mut self.socket,
                &mut self.state,
                ConnectionType::Zone,
                self.config.compression(),
                &batch,
            ))
            .await;
        }
    }
}

impl Zone {
    /// Returns the front-gate street exit `(position, rotation)` nudged 8 yalms forward from the plot's `MapRange`.
    pub fn housing_plot_exit_transform(&self, raw_plot_index: usize) -> Option<(Position, f32)> {
        let plot = self.cached_housing_plots.get(raw_plot_index)?;
        Some(super::plots::nudge_plot_entrance_exit(
            Position(plot.entrance_position),
            plot.entrance_rotation,
            8.0,
        ))
    }
}

/// Returns whether an instance with the given `TerritoryIntendedUse` needs a navigation mesh.
pub(crate) fn should_load_navmesh_for_intended_use(intended_use: TerritoryIntendedUse) -> bool {
    !matches!(
        intended_use,
        TerritoryIntendedUse::HousingIndoor
            | TerritoryIntendedUse::HousingOutdoor
            | TerritoryIntendedUse::Inn
    )
}

pub(crate) fn should_retain_instance(instance: &Instance) -> bool {
    matches!(
        instance.generate_navmesh,
        NavmeshGenerationStep::Started { .. }
    ) || instance
        .actors
        .iter()
        .any(|x| matches!(x.1, NetworkedActor::Player { .. }))
}

pub(crate) fn poll_started_navmesh_generation(
    zone_id: u16,
    nvm_path: &str,
    child: &mut Child,
    navmesh: &mut Navmesh,
) -> bool {
    match child.try_wait() {
        Ok(Some(status)) => {
            if status.success() {
                if let Ok(nvm_bytes) = std::fs::read(nvm_path) {
                    if let Some(loaded) = Navmesh::from_existing(&nvm_bytes) {
                        *navmesh = loaded;
                        tracing::info!("Successfully loaded navimesh from {nvm_path:?}");
                    } else {
                        tracing::warn!(
                            "Failed to read {nvm_path:?}, monsters will not function correctly!"
                        );
                    }
                } else {
                    tracing::warn!(
                        zone_id,
                        "kawari-navimesh exited successfully without writing {nvm_path:?}"
                    );
                }
            } else {
                tracing::warn!(
                    zone_id,
                    status = %status,
                    "kawari-navimesh exited without generating {nvm_path:?}; monsters will not use navmesh in this zone"
                );
            }
            true
        }
        Ok(None) => false,
        Err(err) => {
            tracing::warn!(
                zone_id,
                error = %err,
                "Failed polling kawari-navimesh status for {nvm_path:?}"
            );
            true
        }
    }
}
