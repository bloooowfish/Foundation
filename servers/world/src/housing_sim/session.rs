use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kawari::common::{ContainerType, HouseId, ObjectId, Position, TerritoryIntendedUse};
use kawari::config::get_config;
use kawari::ipc::zone::FurnitureList;
use parking_lot::{Mutex, RwLock};
use serde_derive::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::common::{ClientId, ServerHandle};
use crate::{GameData, WorldDatabase};

use super::overlays::{HousingOverlayKey, HousingOverlayObject, HousingOverlayScope};
use super::remake_place::PresetScope;

/// Reload behavior requested when applying presets or triggering live refresh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadMode {
    #[default]
    Auto,
    ZoneReload,
    LivePackets,
    None,
}

/// Active selected/authorized estate for a connected player session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveEstateSelection {
    pub land_ident: i32,
    pub house_id: u64,
    pub territory_type_id: u16,
    pub ward_index: u8,
    pub division: u8,
    pub plot_index: u8,
    pub room_number: u16,
    pub is_apartment: bool,
    pub plot_size: u8,
    pub indoors: bool,
}

/// Ward browsing / authoritative context for outdoor housing actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WardContext {
    pub territory_type_id: u16,
    pub ward_index: u8,
    pub division: u8,
}

/// Pending login spawn target inside a residential ward (house gate, apartment building, or saved ward coordinates).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoginWardSpawn {
    pub territory_type_id: u16,
    pub ward_index: u8,
    pub division: u8,
    pub plot_index: Option<u8>,
    pub is_apartment_exit: bool,
}

/// Pending second hop for a same-territory indoor reload bounced through the outdoor ward.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingSameZoneIndoorReentry {
    pub land_ident: i32,
    pub expected_transit_zone_id: u16,
    pub expected_transit_ward_index: u8,
    pub preserve_pos: Option<Position>,
    pub preserve_rot: Option<f32>,
    pub ready_at: Option<tokio::time::Instant>,
}

/// Per-connection runtime state stored in `ZoneConnection.housing_sim`.
#[derive(Debug, Default)]
pub struct HousingSimSession {
    pub active_estate: Option<ActiveEstateSelection>,
    pub authoritative_ward_context: Option<WardContext>,
    pub display_ward_context: Option<WardContext>,
    pub pending_indoor_furniture_list_tail: Option<Vec<FurnitureList>>,
    pub pending_indoor_finish_loading: bool,
    pub pending_indoor_finish_loading_since: Option<tokio::time::Instant>,
    pub pending_indoor_overlay_sync: bool,
    pub pending_remodel_container: Option<ContainerType>,
    pub pending_placard_location: Option<(u8, u8)>,
    pub last_entrance_plot: Option<(u8, u8)>,
    pub login_ward_spawn: Option<LoginWardSpawn>,
    pub pending_same_zone_indoor_reentry: Option<PendingSameZoneIndoorReentry>,
    pub synced_indoor_state: Option<super::live_diff::SyncedIndoorState>,
    pub is_owner_authorized: bool,
}

impl HousingSimSession {
    /// Clears transient indoor loading and deferral flags across zone transitions or tail flushes.
    pub fn clear_pending_indoor_load(&mut self) {
        self.pending_indoor_furniture_list_tail = None;
        self.pending_indoor_finish_loading = false;
        self.pending_indoor_finish_loading_since = None;
        self.pending_indoor_overlay_sync = false;
        self.pending_remodel_container = None;
        self.synced_indoor_state = None;
    }

    /// Cancels any pending same-territory indoor re-entry bounce across explicit switches/exits.
    pub fn clear_pending_indoor_reentry(&mut self) {
        self.pending_same_zone_indoor_reentry = None;
    }
}

/// Public snapshot of an online player session exposed via HTTP `/status` & SSE `/events`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OnlineSessionSnapshot {
    pub client_id_debug: String,
    #[serde(skip)]
    pub client_id: Option<ClientId>,
    pub content_id: u64,
    pub actor_id: u32,
    pub character_name: String,
    pub zone_id: u16,
    pub intended_use: TerritoryIntendedUse,
    pub position: [f32; 3],
    pub rotation: f32,
    pub active_estate: Option<ActiveEstateSelection>,
    pub display_ward_context: Option<WardContext>,
    pub authoritative_ward_context: Option<WardContext>,
}

/// Live transform entry carried in SSE deltas for external bridge synchronization.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HousingBridgeMoveItem {
    pub flat_slot: u16,
    pub item_id: u32,
    pub catalog_id: u16,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
    pub rotation: f32,
    pub local_x: f32,
    pub local_y: f32,
    pub local_z: f32,
    pub local_rotation: f32,
}

/// Structured delta metadata included on SSE events for bidirectional external bridges (e.g. ReMakePlace).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HousingBridgeDelta {
    FurnitureMoved {
        indoors: bool,
        moves: Vec<HousingBridgeMoveItem>,
    },
    FurnitureDyed {
        indoors: bool,
        flat_slot: u16,
        item_id: u32,
        stain: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color_hex: Option<String>,
    },
    FurnitureAdded {
        indoors: bool,
        flat_slot: u16,
        item_id: u32,
        catalog_id: u16,
    },
    FurnitureRemoved {
        indoors: bool,
        flat_slot: u16,
        item_id: u32,
    },
    LayoutChanged {
        indoors: bool,
        reason: String,
    },
}

/// Lightweight live transform update dispatched directly to online zone connections.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveFurnitureMove {
    pub flat_slot: u16,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
    pub rotation: f32,
}

/// Server-Sent Event payload broadcast to overlay & dashboard subscribers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HousingSseEvent {
    pub kind: String,
    pub land_ident: Option<i32>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<OnlineSessionSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<HousingBridgeDelta>,
}

impl HousingSseEvent {
    pub fn new(
        kind: impl Into<String>,
        land_ident: Option<i32>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            land_ident,
            message: message.into(),
            session: None,
            source: None,
            revision: None,
            delta: None,
        }
    }

    pub fn with_session(mut self, session: OnlineSessionSnapshot) -> Self {
        self.session = Some(session);
        self
    }

    pub fn with_source(mut self, source: Option<String>) -> Self {
        self.source = source;
        self
    }

    pub fn with_revision(mut self, revision: u64) -> Self {
        self.revision = Some(revision);
        self
    }

    pub fn with_delta(mut self, delta: HousingBridgeDelta) -> Self {
        self.delta = Some(delta);
        self
    }
}

/// Messages sent from `ZoneConnection` or HTTP handlers into `server_main_loop`.
#[derive(Clone, Debug)]
pub enum HousingSimToServer {
    SyncScopeOverlays {
        actor_id: ObjectId,
        scope: HousingOverlayScope,
        objects: Vec<HousingOverlayObject>,
    },
    UpsertSingleOverlay {
        actor_id: ObjectId,
        object: HousingOverlayObject,
    },
    TranslateSingleOverlay {
        actor_id: ObjectId,
        key: HousingOverlayKey,
        position: Position,
        rotation: f32,
    },
    RemoveSingleOverlay {
        actor_id: ObjectId,
        key: HousingOverlayKey,
    },
    WarpToHouseInterior {
        client_id: ClientId,
        actor_id: ObjectId,
        house_id: HouseId,
        territory_type_id: u16,
        preserve_pos: Option<Position>,
        preserve_rot: Option<f32>,
    },
    WarpToHousingPlot {
        client_id: ClientId,
        actor_id: ObjectId,
        territory_type_id: u16,
        ward_index: u8,
        division: u8,
        plot_index: u8,
        override_pos: Option<Position>,
        override_rot: Option<f32>,
    },
    WarpToApartmentOutdoor {
        client_id: ClientId,
        actor_id: ObjectId,
        territory_type_id: u16,
        ward_index: u8,
        division: u8,
    },
    SpawnLoginInWard {
        client_id: ClientId,
        actor_id: ObjectId,
        territory_type_id: u16,
        ward_index: u8,
        division: u8,
        plot_index: Option<u8>,
        is_apartment_exit: bool,
        fallback_pos: Position,
        fallback_rot: f32,
    },
    SwitchClientEstate {
        content_id: Option<u64>,
        land_ident: i32,
        enter_interior: Option<bool>,
    },
    BroadcastEstateLivePackets {
        land_ident: i32,
        indoors_only: bool,
        packets: Vec<kawari::ipc::zone::ServerZoneIpcSegment>,
    },
    BroadcastEstateLiveMoves {
        land_ident: i32,
        indoors: bool,
        moves: Vec<LiveFurnitureMove>,
    },
    NotifyClientsEstateReload {
        land_ident: i32,
        mode: ReloadMode,
        preserve_player_pos: bool,
        enter_if_elsewhere: bool,
        source: Option<String>,
    },
    NotifyClientsEstateDeleted {
        land_ident: i32,
        territory_type_id: u16,
        ward_index: u8,
        division: u8,
        plot_index: u8,
        is_apartment: bool,
    },
    UnlockClientSession {
        content_id: Option<u64>,
    },
    GiveClientSampleKit {
        content_id: Option<u64>,
        kind: String,
    },
}

/// Messages sent from `server_main_loop` back to specific `ZoneConnection`s.
#[derive(Clone, Debug)]
pub enum HousingSimFromServer {
    TriggerReload {
        land_ident: i32,
        mode: ReloadMode,
        preserve_player_pos: bool,
        force_enter: bool,
        source: Option<String>,
    },
    EstateDeleted {
        land_ident: i32,
        territory_type_id: u16,
        ward_index: u8,
        division: u8,
        plot_index: u8,
        is_apartment: bool,
    },
    ApplyLiveMoves {
        land_ident: i32,
        indoors: bool,
        moves: Vec<LiveFurnitureMove>,
    },
    SwitchActiveEstate {
        land_ident: i32,
        enter_interior: Option<bool>,
    },
    UnlockCharacter,
    GiveSampleKit {
        kind: String,
    },
    PushOverlaySync,
}

/// Process-wide coordinator bridging `world` game connections, database, catalog, and Axum HTTP/SSE endpoints.
pub struct HousingSimHub {
    pub server_handle: ServerHandle,
    pub database: Arc<Mutex<WorldDatabase>>,
    pub game_data: Option<Arc<Mutex<GameData>>>,
    pub sessions: RwLock<HashMap<ClientId, OnlineSessionSnapshot>>,
    pub event_tx: broadcast::Sender<HousingSseEvent>,
    pub revision_seq: AtomicU64,
    pub last_applied_preset: Mutex<Option<(PathBuf, PresetScope)>>,
    pub preset_roots: RwLock<Vec<PathBuf>>,
    #[cfg(test)]
    pub catalog_override: Option<Arc<super::gamedata_ext::HousingCatalog>>,
}

impl HousingSimHub {
    pub fn new(
        server_handle: ServerHandle,
        database: Arc<Mutex<WorldDatabase>>,
        game_data: Arc<Mutex<GameData>>,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(128);
        let default_roots = detect_default_preset_roots();
        Self {
            server_handle,
            database,
            game_data: Some(game_data),
            sessions: RwLock::new(HashMap::new()),
            event_tx,
            revision_seq: AtomicU64::new(1),
            last_applied_preset: Mutex::new(None),
            preset_roots: RwLock::new(default_roots),
            #[cfg(test)]
            catalog_override: None,
        }
    }

    #[cfg(test)]
    pub fn new_for_tests(
        server_handle: ServerHandle,
        database: Arc<Mutex<WorldDatabase>>,
        catalog: super::gamedata_ext::HousingCatalog,
    ) -> Self {
        let (event_tx, _) = broadcast::channel(128);
        Self {
            server_handle,
            database,
            game_data: None,
            sessions: RwLock::new(HashMap::new()),
            event_tx,
            revision_seq: AtomicU64::new(1),
            last_applied_preset: Mutex::new(None),
            preset_roots: RwLock::new(Vec::new()),
            catalog_override: Some(Arc::new(catalog)),
        }
    }

    pub fn next_revision(&self) -> u64 {
        self.revision_seq.fetch_add(1, Ordering::Relaxed)
    }

    pub fn current_revision(&self) -> u64 {
        self.revision_seq.load(Ordering::Relaxed)
    }

    pub fn resolve_catalog(&self) -> Arc<super::gamedata_ext::HousingCatalog> {
        #[cfg(test)]
        if let Some(ref c) = self.catalog_override {
            return c.clone();
        }
        self.game_data
            .as_ref()
            .map(|gd| super::gamedata_ext::cached_housing_catalog(&mut gd.lock()))
            .unwrap_or_default()
    }

    pub async fn send_to_server(&self, msg: crate::common::ToServer) {
        let _ = self.server_handle.chan.send(msg).await;
    }

    pub fn update_session(&self, client_id: ClientId, snap: OnlineSessionSnapshot) {
        self.sessions.write().insert(client_id, snap.clone());
        let evt = HousingSseEvent::new(
            "session_state",
            snap.active_estate.as_ref().map(|e| e.land_ident),
            format!("{} @ zone {}", snap.character_name, snap.zone_id),
        )
        .with_session(snap);
        self.emit_event(evt);
    }

    /// Updates `active_estate` on matching online session snapshots immediately and notifies SSE subscribers.
    pub fn update_active_estate_for_content(
        &self,
        content_id: Option<u64>,
        selection: ActiveEstateSelection,
    ) {
        let mut updated_snaps = Vec::new();
        {
            let mut guard = self.sessions.write();
            for snap in guard.values_mut() {
                if content_id.is_none_or(|cid| cid == snap.content_id) {
                    snap.active_estate = Some(selection.clone());
                    updated_snaps.push(snap.clone());
                }
            }
        }
        for snap in updated_snaps {
            let evt = HousingSseEvent::new(
                "session_state",
                Some(selection.land_ident),
                format!("{} @ zone {}", snap.character_name, snap.zone_id),
            )
            .with_session(snap);
            self.emit_event(evt);
        }
    }

    /// Clears `active_estate` on any online session referencing a deleted `land_ident` and notifies SSE subscribers.
    pub fn clear_active_estate_for_land_ident(&self, land_ident: i32) {
        let mut updated_snaps = Vec::new();
        {
            let mut guard = self.sessions.write();
            for snap in guard.values_mut() {
                if snap
                    .active_estate
                    .as_ref()
                    .is_some_and(|e| e.land_ident == land_ident)
                {
                    snap.active_estate = None;
                    updated_snaps.push(snap.clone());
                }
            }
        }
        for snap in updated_snaps {
            let evt = HousingSseEvent::new(
                "session_state",
                None,
                format!("{} @ zone {}", snap.character_name, snap.zone_id),
            )
            .with_session(snap);
            self.emit_event(evt);
        }
    }

    pub fn remove_session(&self, client_id: ClientId) {
        let removed = {
            let mut guard = self.sessions.write();
            guard.remove(&client_id)
        };
        if let Some(removed) = removed {
            let next_primary = self.list_online_sessions().into_iter().next();
            let mut evt = HousingSseEvent::new(
                "session_offline",
                removed.active_estate.as_ref().map(|e| e.land_ident),
                format!("{} disconnected", removed.character_name),
            );
            if let Some(next) = next_primary {
                evt = evt.with_session(next);
            }
            self.emit_event(evt);
        }
    }

    pub fn list_online_sessions(&self) -> Vec<OnlineSessionSnapshot> {
        let mut sessions: Vec<OnlineSessionSnapshot> =
            self.sessions.read().values().cloned().collect();
        sessions.sort_by(|a, b| {
            (a.content_id, a.actor_id, &a.client_id_debug).cmp(&(
                b.content_id,
                b.actor_id,
                &b.client_id_debug,
            ))
        });
        sessions
    }

    pub fn emit_event(&self, mut evt: HousingSseEvent) {
        if evt.revision.is_none() {
            evt.revision = Some(self.next_revision());
        }
        let _ = self.event_tx.send(evt);
    }

    pub fn configured_preset_roots(&self) -> Vec<PathBuf> {
        self.preset_roots.read().clone()
    }
}

fn detect_default_preset_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(env_root) = std::env::var("KAWARI_REMAKE_PLACE_LAYOUT_ROOT")
        && !env_root.trim().is_empty()
    {
        roots.push(PathBuf::from(env_root));
    }
    let cfg = get_config();
    for extra in &cfg.filesystem.additional_resource_paths {
        let candidate = PathBuf::from(extra).join("housing-presets");
        roots.push(candidate);
    }
    roots.push(PathBuf::from("housing-exports"));
    roots.push(PathBuf::from("resources/housing-presets"));
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_pending_indoor_load_resets_all_transient_fields() {
        let mut session = HousingSimSession {
            pending_indoor_furniture_list_tail: Some(vec![FurnitureList::default()]),
            pending_indoor_finish_loading: true,
            pending_indoor_finish_loading_since: Some(tokio::time::Instant::now()),
            pending_indoor_overlay_sync: true,
            pending_remodel_container: Some(ContainerType::HousingInteriorPlacedItems1),
            pending_same_zone_indoor_reentry: Some(PendingSameZoneIndoorReentry {
                land_ident: 34000003,
                expected_transit_zone_id: 340,
                expected_transit_ward_index: 0,
                preserve_pos: None,
                preserve_rot: None,
                ready_at: None,
            }),
            synced_indoor_state: Some(crate::housing_sim::live_diff::SyncedIndoorState::from_db(
                &crate::housing_sim::db::HousingEstateRow::default(),
                343,
                &[],
            )),
            ..Default::default()
        };

        session.clear_pending_indoor_load();

        assert!(session.pending_indoor_furniture_list_tail.is_none());
        assert!(!session.pending_indoor_finish_loading);
        assert!(session.pending_indoor_finish_loading_since.is_none());
        assert!(!session.pending_indoor_overlay_sync);
        assert!(session.pending_remodel_container.is_none());
        assert!(session.synced_indoor_state.is_none());
        assert!(session.pending_same_zone_indoor_reentry.is_some());

        session.clear_pending_indoor_reentry();
        assert!(session.pending_same_zone_indoor_reentry.is_none());
    }

    #[test]
    fn clear_active_estate_and_remove_session_preserve_remaining_primary() {
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let db = Arc::new(Mutex::new(WorldDatabase::new_in_memory()));
        let handle = ServerHandle {
            chan: tx,
            next_id: Arc::new(std::sync::atomic::AtomicUsize::new(1)),
        };
        let hub = HousingSimHub::new_for_tests(
            handle.clone(),
            db,
            crate::housing_sim::gamedata_ext::HousingCatalog::default(),
        );
        let mut rx = hub.event_tx.subscribe();

        let cid_a = handle.next_id();
        let cid_b = handle.next_id();
        let snap_a = OnlineSessionSnapshot {
            client_id_debug: format!("{cid_a:?}"),
            client_id: Some(cid_a),
            content_id: 100,
            actor_id: 10,
            character_name: "Alpha".to_string(),
            zone_id: 342,
            intended_use: TerritoryIntendedUse::HousingIndoor,
            position: [0.0; 3],
            rotation: 0.0,
            active_estate: Some(ActiveEstateSelection {
                land_ident: 34000001,
                house_id: 1,
                territory_type_id: 340,
                ward_index: 0,
                division: 0,
                plot_index: 0,
                room_number: 0,
                is_apartment: false,
                plot_size: 0,
                indoors: true,
            }),
            display_ward_context: None,
            authoritative_ward_context: None,
        };
        let snap_b = OnlineSessionSnapshot {
            client_id_debug: format!("{cid_b:?}"),
            client_id: Some(cid_b),
            content_id: 200,
            actor_id: 20,
            character_name: "Beta".to_string(),
            zone_id: 340,
            intended_use: TerritoryIntendedUse::HousingOutdoor,
            position: [0.0; 3],
            rotation: 0.0,
            active_estate: None,
            display_ward_context: None,
            authoritative_ward_context: None,
        };
        hub.update_session(cid_a, snap_a);
        hub.update_session(cid_b, snap_b);
        let _ = rx.try_recv();
        let _ = rx.try_recv();

        hub.clear_active_estate_for_land_ident(34000001);
        let cleared_evt = rx.try_recv().unwrap();
        assert_eq!(cleared_evt.kind, "session_state");
        assert!(
            cleared_evt
                .session
                .as_ref()
                .is_some_and(|s| s.active_estate.is_none())
        );
        assert!(hub.list_online_sessions()[0].active_estate.is_none());

        hub.remove_session(cid_a);
        let off_evt_1 = rx.try_recv().unwrap();
        assert_eq!(off_evt_1.kind, "session_offline");
        assert_eq!(
            off_evt_1
                .session
                .as_ref()
                .map(|s| s.character_name.as_str()),
            Some("Beta")
        );

        hub.remove_session(cid_b);
        let off_evt_2 = rx.try_recv().unwrap();
        assert_eq!(off_evt_2.kind, "session_offline");
        assert!(off_evt_2.session.is_none());
    }
}
