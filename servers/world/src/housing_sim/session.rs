use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

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

/// Per-connection runtime state stored in `ZoneConnection.housing_sim`.
#[derive(Debug, Default)]
pub struct HousingSimSession {
    pub active_estate: Option<ActiveEstateSelection>,
    pub authoritative_ward_context: Option<WardContext>,
    pub display_ward_context: Option<WardContext>,
    pub pending_indoor_furniture_list_tail: Option<Vec<FurnitureList>>,
    pub pending_indoor_finish_loading: bool,
    pub pending_indoor_overlay_sync: bool,
    pub pending_remodel_container: Option<ContainerType>,
    pub pending_placard_location: Option<(u8, u8)>,
    pub last_entrance_plot: Option<(u8, u8)>,
    pub login_ward_spawn: Option<LoginWardSpawn>,
    pub is_owner_authorized: bool,
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

/// Server-Sent Event payload broadcast to overlay & dashboard subscribers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HousingSseEvent {
    pub kind: String,
    pub land_ident: Option<i32>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<OnlineSessionSnapshot>,
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
    NotifyClientsEstateReload {
        land_ident: i32,
        mode: ReloadMode,
        preserve_player_pos: bool,
        enter_if_elsewhere: bool,
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
            last_applied_preset: Mutex::new(None),
            preset_roots: RwLock::new(Vec::new()),
            catalog_override: Some(Arc::new(catalog)),
        }
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
        self.emit_event(HousingSseEvent {
            kind: "session_state".to_string(),
            land_ident: snap.active_estate.as_ref().map(|e| e.land_ident),
            message: format!("{} @ zone {}", snap.character_name, snap.zone_id),
            session: Some(snap),
        });
    }

    pub fn remove_session(&self, client_id: ClientId) {
        if let Some(removed) = self.sessions.write().remove(&client_id) {
            self.emit_event(HousingSseEvent {
                kind: "session_offline".to_string(),
                land_ident: removed.active_estate.as_ref().map(|e| e.land_ident),
                message: format!("{} disconnected", removed.character_name),
                session: None,
            });
        }
    }

    pub fn list_online_sessions(&self) -> Vec<OnlineSessionSnapshot> {
        self.sessions.read().values().cloned().collect()
    }

    pub fn emit_event(&self, evt: HousingSseEvent) {
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
