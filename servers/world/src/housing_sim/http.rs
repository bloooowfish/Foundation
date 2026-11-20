use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::{self, Stream};
use kawari::common::{HousingFlag, Position};
use kawari::ipc::zone::PlotSize;
use serde_derive::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;

use crate::common::ToServer;

use super::containers::{
    container_from_i32, flat_slot_for_placed_container, is_exterior_placed_container,
    is_interior_placed_container, placed_container_for_flat_slot,
};
use super::db::{
    EnsureEstateSpec, EstateRemakeMeta, FurnitureRemakeMeta, HousingEstateRow, HousingFurnitureRow,
    MAX_ESTATE_GREETING_BYTES, MAX_ESTATE_NAME_BYTES, SnapshotSummary, clamp_utf8, create_snapshot,
    delete_estate_and_furniture, ensure_estate, estate_by_land_ident, list_all_estates,
    list_furniture_for_estate, list_placed_furniture_in_scope, list_snapshots, reset_estate_scope,
    restore_latest_snapshot, restore_snapshot, save_estate, update_furniture_position,
};
use super::fixtures::{StoredHouseExterior, StoredHouseInterior};
use super::flows::{
    apply_preset_layout_with_options, build_selection_from_row, target_indoor_territory_for_estate,
    unlock_characters_in_database,
};
use super::gamedata_ext::{
    default_renovation_row_for_district_and_size, display_name_for_renovation,
    normalize_district_style_name, renovation_row_for_style_and_size,
};
use super::plots::{
    canonical_plot_for_district_and_size, default_district_for_zone, parse_district_territory,
    plot_spec, wrap_angle_radians,
};
use super::remake_place::{
    PresetFileSummary, PresetScope, ReMakePlaceLayout, ReMakePlacePreviewReport,
    export_estate_to_layout, parse_layout_bytes, prepare_apply_layout, read_layout_file,
    resolve_safe_preset_path, sanitize_export_filename, scan_preset_directory,
};
use super::session::{
    HousingBridgeDelta, HousingBridgeMoveItem, HousingSimHub, HousingSimToServer, HousingSseEvent,
    LiveFurnitureMove, OnlineSessionSnapshot, ReloadMode,
};

#[derive(Debug, Serialize)]
struct ApiErrorBody {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiErrorBody>)>;

fn api_err(status: StatusCode, msg: impl Into<String>) -> (StatusCode, Json<ApiErrorBody>) {
    (status, Json(ApiErrorBody { error: msg.into() }))
}

/// Builds the complete `/api/v1/housing` Axum router.
pub fn router(hub: Arc<HousingSimHub>) -> Router {
    Router::new()
        .route("/status", get(status_handler))
        .route("/events", get(events_handler))
        .route("/session/switch", post(switch_session_handler))
        .route("/session/unlock-all", post(unlock_all_session_handler))
        .route("/session/give-kit", post(give_kit_handler))
        .route("/estates", get(list_estates_handler))
        .route("/estates/ensure", post(ensure_estate_handler))
        .route(
            "/estates/{land_ident}",
            get(get_estate_handler)
                .patch(patch_estate_handler)
                .delete(delete_estate_handler),
        )
        .route("/estates/{land_ident}/reset", post(reset_estate_handler))
        .route("/estates/{land_ident}/reload", post(reload_estate_handler))
        .route("/presets", get(list_presets_handler))
        .route("/presets/preview", post(preview_preset_handler))
        .route(
            "/estates/{land_ident}/preset/apply",
            post(apply_preset_handler),
        )
        .route(
            "/estates/{land_ident}/snapshots",
            get(list_snapshots_handler).post(create_snapshot_handler),
        )
        .route(
            "/estates/{land_ident}/snapshots/{snapshot_id}/restore",
            post(restore_snapshot_handler),
        )
        .route("/estates/{land_ident}/undo", post(undo_estate_handler))
        .route(
            "/estates/{land_ident}/export/remakeplace",
            get(export_remakeplace_get_handler).post(export_remakeplace_post_handler),
        )
        .route(
            "/estates/{land_ident}/bridge/snapshot",
            get(bridge_snapshot_handler),
        )
        .route("/estates/{land_ident}/live/move", post(live_move_handler))
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
        .layer(CorsLayer::permissive())
        .with_state(hub)
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub service: &'static str,
    pub protocol_patch: &'static str,
    pub online_sessions: Vec<OnlineSessionSnapshot>,
    pub preset_roots: Vec<String>,
    pub last_applied_preset: Option<String>,
    pub estates_count: usize,
}

async fn status_handler(State(hub): State<Arc<HousingSimHub>>) -> ApiResult<StatusResponse> {
    let estates_count = {
        let mut db = hub.database.lock();
        list_all_estates(db.connection_mut())
            .map(|v| v.len())
            .unwrap_or(0)
    };
    let last = hub
        .last_applied_preset
        .lock()
        .as_ref()
        .map(|(p, _)| p.display().to_string());
    Ok(Json(StatusResponse {
        service: "foundation-housing-simulator",
        protocol_patch: "7.58",
        online_sessions: hub.list_online_sessions(),
        preset_roots: hub
            .configured_preset_roots()
            .into_iter()
            .map(|p| p.display().to_string())
            .collect(),
        last_applied_preset: last,
        estates_count,
    }))
}

async fn events_handler(
    State(hub): State<Arc<HousingSimHub>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = hub.event_tx.subscribe();
    let stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(evt) => {
                    let json = serde_json::to_string(&evt).unwrap_or_else(|_| "{}".to_string());
                    return Some((Ok(Event::default().event(evt.kind).data(json)), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[derive(Serialize)]
pub struct EstateSummary {
    pub row: HousingEstateRow,
    pub house_id_hex: String,
    pub indoor_count: usize,
    pub outdoor_count: usize,
    pub snapshot_count: usize,
    pub pattern_name: String,
}

async fn list_estates_handler(
    State(hub): State<Arc<HousingSimHub>>,
) -> ApiResult<Vec<EstateSummary>> {
    let mut db = hub.database.lock();
    let conn = db.connection_mut();
    let estates = list_all_estates(conn)
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut out = Vec::with_capacity(estates.len());
    for row in estates {
        let indoor_count = list_placed_furniture_in_scope(conn, row.land_ident, true)
            .map(|r| r.len())
            .unwrap_or(0);
        let outdoor_count = list_placed_furniture_in_scope(conn, row.land_ident, false)
            .map(|r| r.len())
            .unwrap_or(0);
        let snapshot_count = list_snapshots(conn, row.land_ident)
            .map(|s| s.len())
            .unwrap_or(0);
        let int = StoredHouseInterior::from_json(&row.interior_json);
        let house_id_hex = format!("0x{:X}", row.house_id as u64);
        let pattern_name = display_name_for_renovation(int.renovation_row_id).to_string();
        out.push(EstateSummary {
            row,
            house_id_hex,
            indoor_count,
            outdoor_count,
            snapshot_count,
            pattern_name,
        });
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(default)]
pub struct EnsureEstateRequest {
    pub district: Option<String>,
    pub territory_type_id: u16,
    pub world_id: u16,
    pub ward_index: u8,
    pub division: u8,
    pub plot_index: Option<u8>,
    pub room_number: u16,
    pub is_apartment: bool,
    pub plot_size: Option<u8>,
    pub is_fc: bool,
    pub owner_content_id: u64,
    pub owner_name: String,
    pub estate_name: Option<String>,
}

impl Default for EnsureEstateRequest {
    fn default() -> Self {
        Self {
            district: None,
            territory_type_id: 340,
            world_id: kawari::config::get_config().world.world_id,
            ward_index: 0,
            division: 0,
            plot_index: None,
            room_number: 0,
            is_apartment: false,
            plot_size: None,
            is_fc: false,
            owner_content_id: 1,
            owner_name: "Simulator".to_string(),
            estate_name: None,
        }
    }
}

fn ensure_estate_row_with_req(
    hub: &HousingSimHub,
    req: &EnsureEstateRequest,
) -> Result<HousingEstateRow, (StatusCode, Json<ApiErrorBody>)> {
    let catalog = hub.resolve_catalog();
    let raw_territory = req
        .district
        .as_deref()
        .and_then(parse_district_territory)
        .unwrap_or(req.territory_type_id);
    let district = default_district_for_zone(raw_territory);
    let division = req.division.clamp(0, 1);

    let (plot_index, size) = if req.is_apartment {
        (0u8, PlotSize::Small)
    } else if let Some(explicit_plot) = req.plot_index {
        let p = explicit_plot % 30;
        let sz = req
            .plot_size
            .and_then(PlotSize::from_repr)
            .unwrap_or_else(|| {
                plot_spec(district, division, p)
                    .map(|s| s.size)
                    .unwrap_or(PlotSize::Small)
            });
        (p, sz)
    } else {
        let sz = req
            .plot_size
            .and_then(PlotSize::from_repr)
            .unwrap_or(PlotSize::Small);
        let p = canonical_plot_for_district_and_size(district, division, sz).unwrap_or(0);
        (p, sz)
    };

    let exterior = if req.is_apartment {
        StoredHouseExterior::default()
    } else {
        StoredHouseExterior::default_for_location(&catalog, district, size)
    };
    let interior =
        StoredHouseInterior::default_for_location(&catalog, district, size, req.is_apartment);

    let spec = EnsureEstateSpec {
        territory_type_id: district,
        world_id: req.world_id,
        ward_index: req.ward_index.min(29),
        division,
        plot_index,
        room_number: if req.is_apartment {
            req.room_number.clamp(1, 90)
        } else {
            0
        },
        is_apartment: req.is_apartment,
        plot_size: size,
        is_fc: req.is_fc,
        owner_content_id: req.owner_content_id,
        owner_name: req.owner_name.clone(),
        estate_name: req.estate_name.clone().unwrap_or_else(|| {
            if req.is_apartment {
                format!("{}'s Apt #{}", req.owner_name, req.room_number.clamp(1, 90))
            } else {
                format!("{}'s Estate", req.owner_name)
            }
        }),
        exterior_json: exterior.to_json(),
        interior_json: interior.to_json(),
    };

    let mut db = hub.database.lock();
    ensure_estate(db.connection_mut(), &spec)
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn ensure_estate_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Json(req): Json<EnsureEstateRequest>,
) -> ApiResult<HousingEstateRow> {
    let row = ensure_estate_row_with_req(&hub, &req)?;
    hub.emit_event(HousingSseEvent::new(
        "estate_ensured",
        Some(row.land_ident),
        format!(
            "하우스 준비 완료: {}",
            if row.estate_name.is_empty() {
                "테스트 하우스"
            } else {
                &row.estate_name
            }
        ),
    ));
    Ok(Json(row))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SwitchSessionRequest {
    pub land_ident: Option<i32>,
    pub district: Option<String>,
    pub territory_type_id: Option<u16>,
    pub world_id: Option<u16>,
    pub ward_index: Option<u8>,
    pub division: Option<u8>,
    pub plot_index: Option<u8>,
    pub room_number: Option<u16>,
    pub is_apartment: Option<bool>,
    pub plot_size: Option<u8>,
    pub is_fc: Option<bool>,
    pub enter_interior: Option<bool>,
    pub owner_content_id: Option<u64>,
    pub owner_name: Option<String>,
    pub estate_name: Option<String>,
}

#[derive(Serialize)]
pub struct SwitchSessionResponse {
    pub estate: HousingEstateRow,
    pub target_indoor_territory: u16,
    pub online_sessions_notified: usize,
}

async fn switch_session_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Json(req): Json<SwitchSessionRequest>,
) -> ApiResult<SwitchSessionResponse> {
    let sessions = hub.list_online_sessions();
    let active_session = if let Some(cid) = req.owner_content_id {
        sessions.iter().find(|s| s.content_id == cid)
    } else {
        sessions.first()
    };

    let estate = if let Some(ident) = req.land_ident {
        let mut db = hub.database.lock();
        estate_by_land_ident(db.connection_mut(), ident)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Estate not found"))?
    } else {
        let territory_type_id = req
            .district
            .as_deref()
            .and_then(parse_district_territory)
            .or(req.territory_type_id)
            .or_else(|| active_session.map(|s| default_district_for_zone(s.zone_id)))
            .unwrap_or(340);
        let ensure_req = EnsureEstateRequest {
            district: req.district.clone(),
            territory_type_id,
            world_id: req
                .world_id
                .unwrap_or_else(|| kawari::config::get_config().world.world_id),
            ward_index: req
                .ward_index
                .or_else(|| {
                    active_session.and_then(|s| s.display_ward_context.map(|w| w.ward_index))
                })
                .unwrap_or(0),
            division: req.division.unwrap_or(0),
            plot_index: req.plot_index,
            room_number: req.room_number.unwrap_or(1),
            is_apartment: req.is_apartment.unwrap_or(false),
            plot_size: req.plot_size,
            is_fc: req.is_fc.unwrap_or(false),
            owner_content_id: req
                .owner_content_id
                .or_else(|| active_session.map(|s| s.content_id))
                .unwrap_or(1),
            owner_name: req
                .owner_name
                .clone()
                .or_else(|| active_session.map(|s| s.character_name.clone()))
                .unwrap_or_else(|| "Simulator".to_string()),
            estate_name: req.estate_name.clone(),
        };
        ensure_estate_row_with_req(&hub, &ensure_req)?
    };

    let target_indoor_territory = target_indoor_territory_for_estate(&estate);
    let target_content_id = req
        .owner_content_id
        .or_else(|| active_session.map(|s| s.content_id));
    let currently_indoors = active_session
        .is_some_and(|s| s.intended_use == kawari::common::TerritoryIntendedUse::HousingIndoor);
    let go_indoor_hint = req
        .enter_interior
        .unwrap_or(estate.is_apartment || currently_indoors);

    hub.update_active_estate_for_content(
        target_content_id,
        build_selection_from_row(&estate, go_indoor_hint),
    );
    hub.emit_event(HousingSseEvent::new(
        "session_switched",
        Some(estate.land_ident),
        format!(
            "활성 하우스 전환: {}",
            if estate.estate_name.is_empty() {
                "테스트 하우스"
            } else {
                &estate.estate_name
            }
        ),
    ));
    hub.send_to_server(ToServer::HousingSim(
        HousingSimToServer::SwitchClientEstate {
            content_id: target_content_id,
            land_ident: estate.land_ident,
            enter_interior: req.enter_interior,
        },
    ))
    .await;

    let notified = sessions
        .iter()
        .filter(|s| target_content_id.is_none_or(|cid| cid == s.content_id))
        .count();

    Ok(Json(SwitchSessionResponse {
        estate,
        target_indoor_territory,
        online_sessions_notified: notified,
    }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct UnlockAllSessionRequest {
    pub content_id: Option<u64>,
}

#[derive(Serialize)]
pub struct UnlockAllSessionResponse {
    pub unlocked_characters: usize,
    pub online_sessions_notified: usize,
    pub character_names: Vec<String>,
}

async fn unlock_all_session_handler(
    State(hub): State<Arc<HousingSimHub>>,
    body: Option<Json<UnlockAllSessionRequest>>,
) -> ApiResult<UnlockAllSessionResponse> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let names = {
        let gd_guard = hub.game_data.as_ref().map(|gd| gd.lock());
        let mut db = hub.database.lock();
        unlock_characters_in_database(&mut db, gd_guard.as_deref(), req.content_id)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    let online_sessions = hub.list_online_sessions();
    let notified = online_sessions
        .iter()
        .filter(|s| req.content_id.is_none_or(|cid| cid == s.content_id))
        .count();

    hub.send_to_server(ToServer::HousingSim(
        HousingSimToServer::UnlockClientSession {
            content_id: req.content_id,
        },
    ))
    .await;

    hub.emit_event(HousingSseEvent::new(
        "session_unlocked",
        None,
        if names.is_empty() {
            "해금 대상 캐릭터가 없습니다.".to_string()
        } else {
            format!("전체 해금 완료: {}", names.join(", "))
        },
    ));

    Ok(Json(UnlockAllSessionResponse {
        unlocked_characters: names.len(),
        online_sessions_notified: notified,
        character_names: names,
    }))
}

#[derive(Deserialize)]
pub struct GiveKitRequest {
    #[serde(default = "default_kit_kind")]
    pub kind: String,
    #[serde(default)]
    pub content_id: Option<u64>,
}

fn default_kit_kind() -> String {
    "indoor".to_string()
}

#[derive(Serialize)]
pub struct GiveKitResponse {
    pub kind: String,
    pub online_sessions_notified: usize,
}

async fn give_kit_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Json(req): Json<GiveKitRequest>,
) -> ApiResult<GiveKitResponse> {
    let normalized = req.kind.trim().to_ascii_lowercase();
    if !matches!(normalized.as_str(), "indoor" | "outdoor" | "npc") {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "kind must be one of 'indoor', 'outdoor', or 'npc'",
        ));
    }

    let online_sessions = hub.list_online_sessions();
    let notified = online_sessions
        .iter()
        .filter(|s| req.content_id.is_none_or(|cid| cid == s.content_id))
        .count();

    hub.send_to_server(ToServer::HousingSim(
        HousingSimToServer::GiveClientSampleKit {
            content_id: req.content_id,
            kind: normalized.clone(),
        },
    ))
    .await;

    hub.emit_event(HousingSseEvent::new(
        "kit_granted",
        None,
        format!("샘플 가구 키트 전송 완료 ({normalized})"),
    ));

    Ok(Json(GiveKitResponse {
        kind: normalized,
        online_sessions_notified: notified,
    }))
}

#[derive(Serialize)]
pub struct EstateDetailResponse {
    pub estate: HousingEstateRow,
    pub exterior: StoredHouseExterior,
    pub interior: StoredHouseInterior,
    pub furniture: Vec<HousingFurnitureRow>,
    pub snapshots: Vec<SnapshotSummary>,
}

async fn get_estate_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path(land_ident): Path<i32>,
) -> ApiResult<EstateDetailResponse> {
    let mut db = hub.database.lock();
    let conn = db.connection_mut();
    let estate = estate_by_land_ident(conn, land_ident)
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Estate not found"))?;
    let exterior = StoredHouseExterior::from_json(&estate.exterior_json);
    let interior = StoredHouseInterior::from_json(&estate.interior_json);
    let furniture = list_furniture_for_estate(conn, land_ident, None).unwrap_or_default();
    let snapshots = list_snapshots(conn, land_ident).unwrap_or_default();
    Ok(Json(EstateDetailResponse {
        estate,
        exterior,
        interior,
        furniture,
        snapshots,
    }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct PatchEstateRequest {
    pub estate_name: Option<String>,
    pub greeting: Option<String>,
    pub light_level: Option<u8>,
    pub enable_ssao: Option<bool>,
    pub visitors_allowed: Option<bool>,
    pub plot_size: Option<u8>,
    pub interior_renovation_row_id: Option<u16>,
    pub interior_style: Option<String>,
    pub exterior: Option<StoredHouseExterior>,
    pub interior: Option<StoredHouseInterior>,
    pub reload_mode: ReloadMode,
    pub source: Option<String>,
}

async fn patch_estate_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path(land_ident): Path<i32>,
    Json(req): Json<PatchEstateRequest>,
) -> ApiResult<HousingEstateRow> {
    let mut needs_zone_reload = false;
    let updated = {
        let mut db = hub.database.lock();
        let conn = db.connection_mut();
        let mut est = estate_by_land_ident(conn, land_ident)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Estate not found"))?;
        let _ = create_snapshot(conn, land_ident, "patch estate");

        if let Some(name) = req.estate_name {
            est.estate_name = clamp_utf8(&name, MAX_ESTATE_NAME_BYTES);
        }
        if let Some(greeting) = req.greeting {
            est.greeting = clamp_utf8(&greeting, MAX_ESTATE_GREETING_BYTES);
        }
        if let Some(light) = req.light_level {
            est.light_level = light.min(5) as i32;
        }
        if let Some(ssao) = req.enable_ssao {
            est.enable_ssao = ssao;
        }
        if let Some(open) = req.visitors_allowed {
            let bit = HousingFlag::OPEN.bits() as i32;
            if open {
                est.flags |= bit;
            } else {
                est.flags &= !bit;
            }
        }
        let prev_plot_size = est.plot_size;
        if let Some(sz) = req.plot_size {
            let clamped = sz.min(2) as i32;
            if est.plot_size != clamped {
                est.plot_size = clamped;
                needs_zone_reload = true;
            }
        }
        if let Some(ext) = req.exterior {
            est.exterior_json = ext.to_json();
        }
        let has_explicit_interior = req.interior.is_some();
        let mut int = req
            .interior
            .unwrap_or_else(|| StoredHouseInterior::from_json(&est.interior_json));
        if !est.is_apartment {
            if let Some(row_id) = req.interior_renovation_row_id
                && (1..=21).contains(&row_id)
                && int.renovation_row_id != row_id
            {
                int.renovation_row_id = row_id;
                needs_zone_reload = true;
            } else if let Some(ref style) = req.interior_style
                && let Some(canon) = normalize_district_style_name(style)
            {
                let size = PlotSize::from_repr(est.plot_size.clamp(0, 2) as u8).unwrap_or_default();
                if let Some(row_id) = renovation_row_for_style_and_size(canon, size)
                    && int.renovation_row_id != row_id
                {
                    int.renovation_row_id = row_id;
                    needs_zone_reload = true;
                }
            } else if est.plot_size != prev_plot_size && !has_explicit_interior {
                let new_size =
                    PlotSize::from_repr(est.plot_size.clamp(0, 2) as u8).unwrap_or_default();
                let remapped_row = normalize_district_style_name(display_name_for_renovation(
                    int.renovation_row_id,
                ))
                .and_then(|canon| renovation_row_for_style_and_size(canon, new_size))
                .unwrap_or_else(|| {
                    default_renovation_row_for_district_and_size(
                        est.territory_type_id as u16,
                        new_size,
                    )
                });
                if int.renovation_row_id != remapped_row {
                    int.renovation_row_id = remapped_row;
                    needs_zone_reload = true;
                }
            }
        }
        est.interior_json = int.to_json();
        save_estate(conn, &est)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        est
    };

    let mode = if needs_zone_reload && req.reload_mode != ReloadMode::None {
        ReloadMode::ZoneReload
    } else {
        req.reload_mode
    };
    hub.emit_event(
        HousingSseEvent::new(
            "estate_updated",
            Some(land_ident),
            "하우스 설정이 업데이트되었습니다.",
        )
        .with_source(req.source.clone()),
    );
    if mode != ReloadMode::None {
        hub.send_to_server(ToServer::HousingSim(
            HousingSimToServer::NotifyClientsEstateReload {
                land_ident,
                mode,
                preserve_player_pos: true,
                enter_if_elsewhere: false,
                source: req.source,
            },
        ))
        .await;
    }

    Ok(Json(updated))
}

async fn delete_estate_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path(land_ident): Path<i32>,
) -> ApiResult<serde_json::Value> {
    let deleted = {
        let mut db = hub.database.lock();
        delete_estate_and_furniture(db.connection_mut(), land_ident)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    if deleted == 0 {
        return Err(api_err(StatusCode::NOT_FOUND, "Estate not found"));
    }

    hub.emit_event(HousingSseEvent::new(
        "estate_deleted",
        Some(land_ident),
        format!("하우스 #{land_ident} 삭제 완료"),
    ));

    Ok(Json(serde_json::json!({
        "ok": true,
        "land_ident": land_ident,
    })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ResetEstateRequest {
    pub scope: PresetScope,
    pub reload_mode: ReloadMode,
    pub source: Option<String>,
}

#[derive(Serialize)]
pub struct ResetEstateResponse {
    pub snapshot_id: Option<i32>,
    pub removed_count: usize,
}

async fn reset_estate_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path(land_ident): Path<i32>,
    Json(req): Json<ResetEstateRequest>,
) -> ApiResult<ResetEstateResponse> {
    let catalog = hub.resolve_catalog();
    enum ResetTxErr {
        NotFound,
        Diesel(diesel::result::Error),
    }
    impl From<diesel::result::Error> for ResetTxErr {
        fn from(value: diesel::result::Error) -> Self {
            Self::Diesel(value)
        }
    }
    let (snapshot_id, removed_count) = {
        use diesel::Connection;

        let mut db = hub.database.lock();
        let conn = db.connection_mut();
        conn.transaction::<_, ResetTxErr, _>(|conn| {
            let mut est = estate_by_land_ident(conn, land_ident)?.ok_or(ResetTxErr::NotFound)?;
            let snap = create_snapshot(conn, land_ident, &format!("reset ({:?})", req.scope)).ok();
            let size = PlotSize::from_repr(est.plot_size.clamp(0, 2) as u8).unwrap_or_default();
            if req.scope.includes_interior() {
                est.interior_json = StoredHouseInterior::default_for_location(
                    &catalog,
                    est.territory_type_id as u16,
                    size,
                    est.is_apartment,
                )
                .to_json();
            }
            if req.scope.includes_exterior() && !est.is_apartment {
                est.exterior_json = StoredHouseExterior::default_for_location(
                    &catalog,
                    est.territory_type_id as u16,
                    size,
                )
                .to_json();
            }
            let removed = reset_estate_scope(
                conn,
                &est,
                req.scope.includes_interior(),
                req.scope.includes_exterior(),
            )?;
            Ok((snap, removed))
        })
        .map_err(|e| match e {
            ResetTxErr::NotFound => api_err(StatusCode::NOT_FOUND, "Estate not found"),
            ResetTxErr::Diesel(err) => api_err(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
        })?
    };

    hub.emit_event(
        HousingSseEvent::new(
            "estate_reset",
            Some(land_ident),
            format!("하우스 비우기 완료 (가구 {removed_count}개 제거)"),
        )
        .with_source(req.source.clone()),
    );
    if req.reload_mode != ReloadMode::None {
        hub.send_to_server(ToServer::HousingSim(
            HousingSimToServer::NotifyClientsEstateReload {
                land_ident,
                mode: req.reload_mode,
                preserve_player_pos: true,
                enter_if_elsewhere: false,
                source: req.source,
            },
        ))
        .await;
    }

    Ok(Json(ResetEstateResponse {
        snapshot_id,
        removed_count,
    }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ReloadEstateRequest {
    pub mode: ReloadMode,
    pub preserve_pos: Option<bool>,
    pub force_enter: bool,
}

async fn reload_estate_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path(land_ident): Path<i32>,
    Json(req): Json<ReloadEstateRequest>,
) -> impl IntoResponse {
    hub.send_to_server(ToServer::HousingSim(
        HousingSimToServer::NotifyClientsEstateReload {
            land_ident,
            mode: req.mode,
            preserve_player_pos: req.preserve_pos.unwrap_or(true),
            enter_if_elsewhere: req.force_enter,
            source: None,
        },
    ))
    .await;
    Json(serde_json::json!({ "ok": true, "land_ident": land_ident }))
}

#[derive(Serialize)]
pub struct PresetsListResponse {
    pub roots: Vec<String>,
    pub files: Vec<PresetFileSummary>,
}

async fn list_presets_handler(
    State(hub): State<Arc<HousingSimHub>>,
) -> ApiResult<PresetsListResponse> {
    let roots = hub.configured_preset_roots();
    let mut files = Vec::new();
    for root in &roots {
        files.extend(scan_preset_directory(root));
    }
    files.sort_by(|a, b| b.modified_unix_ms.cmp(&a.modified_unix_ms));
    Ok(Json(PresetsListResponse {
        roots: roots.into_iter().map(|p| p.display().to_string()).collect(),
        files,
    }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct PreviewPresetRequest {
    pub path: Option<String>,
    pub layout: Option<ReMakePlaceLayout>,
    pub raw_json: Option<String>,
    pub scope: PresetScope,
    pub land_ident: Option<i32>,
}

async fn preview_preset_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Json(req): Json<PreviewPresetRequest>,
) -> ApiResult<ReMakePlacePreviewReport> {
    let layout = resolve_layout_from_request(
        &hub.configured_preset_roots(),
        req.path.as_deref(),
        req.layout.as_ref(),
        req.raw_json.as_deref(),
    )
    .map_err(|e| api_err(StatusCode::BAD_REQUEST, e))?;

    let estate = if let Some(ident) = req.land_ident {
        let mut db = hub.database.lock();
        estate_by_land_ident(db.connection_mut(), ident)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Estate not found"))?
    } else {
        HousingEstateRow {
            land_ident: 0,
            territory_type_id: 340,
            plot_size: 2,
            ..Default::default()
        }
    };

    let catalog = hub.resolve_catalog();
    let prepared = prepare_apply_layout(&estate, &layout, &catalog, req.scope, 0);
    Ok(Json(prepared.preview))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ApplyPresetRequest {
    pub path: Option<String>,
    pub layout: Option<ReMakePlaceLayout>,
    pub raw_json: Option<String>,
    pub scope: PresetScope,
    pub reload_mode: ReloadMode,
    pub preserve_pos: Option<bool>,
    pub force_enter: bool,
    pub actor_content_id: Option<u64>,
    pub label: Option<String>,
    pub source: Option<String>,
    pub skip_snapshot: bool,
}

#[derive(Serialize)]
pub struct ApplyPresetResponse {
    pub land_ident: i32,
    pub snapshot_id: Option<i32>,
    pub reload_mode: ReloadMode,
    pub preview: ReMakePlacePreviewReport,
}

async fn apply_preset_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path(land_ident): Path<i32>,
    Json(req): Json<ApplyPresetRequest>,
) -> ApiResult<ApplyPresetResponse> {
    let roots = hub.configured_preset_roots();
    let content_id = req.actor_content_id.unwrap_or(0);
    let label = req.label.clone().unwrap_or_else(|| {
        req.path
            .clone()
            .unwrap_or_else(|| "overlay preset apply".to_string())
    });

    let catalog = hub.resolve_catalog();
    let (prepared, snapshot_id) = if let Some(ref raw_path) = req.path
        && req.layout.is_none()
        && req.raw_json.is_none()
    {
        let resolved = resolve_safe_preset_path(raw_path, &roots)
            .map_err(|e| api_err(StatusCode::BAD_REQUEST, e))?;
        let layout =
            read_layout_file(&resolved).map_err(|e| api_err(StatusCode::BAD_REQUEST, e))?;
        let res = apply_preset_layout_with_options(
            &hub.database,
            &catalog,
            land_ident,
            &layout,
            req.scope,
            content_id,
            &label,
            req.skip_snapshot,
        )
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e))?;
        *hub.last_applied_preset.lock() = Some((resolved, req.scope));
        res
    } else {
        let layout = resolve_layout_from_request(
            &roots,
            req.path.as_deref(),
            req.layout.as_ref(),
            req.raw_json.as_deref(),
        )
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e))?;
        apply_preset_layout_with_options(
            &hub.database,
            &catalog,
            land_ident,
            &layout,
            req.scope,
            content_id,
            &label,
            req.skip_snapshot,
        )
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e))?
    };

    let effective_reload =
        if prepared.preview.renovation_territory_changed && req.reload_mode != ReloadMode::None {
            ReloadMode::ZoneReload
        } else {
            req.reload_mode
        };

    hub.emit_event(
        HousingSseEvent::new(
            "preset_applied",
            Some(land_ident),
            format!(
                "프리셋 적용 완료: '{}' (실내 {}/{}, 정원 {}/{})",
                label,
                prepared.preview.interior_resolved_count,
                prepared.preview.interior_input_count,
                prepared.preview.exterior_resolved_count,
                prepared.preview.exterior_input_count,
            ),
        )
        .with_source(req.source.clone()),
    );

    if effective_reload != ReloadMode::None {
        hub.send_to_server(ToServer::HousingSim(
            HousingSimToServer::NotifyClientsEstateReload {
                land_ident,
                mode: effective_reload,
                preserve_player_pos: req.preserve_pos.unwrap_or(true),
                enter_if_elsewhere: req.force_enter,
                source: req.source,
            },
        ))
        .await;
    }

    Ok(Json(ApplyPresetResponse {
        land_ident,
        snapshot_id,
        reload_mode: effective_reload,
        preview: prepared.preview,
    }))
}

async fn list_snapshots_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path(land_ident): Path<i32>,
) -> ApiResult<Vec<SnapshotSummary>> {
    let mut db = hub.database.lock();
    list_snapshots(db.connection_mut(), land_ident)
        .map(Json)
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct CreateSnapshotRequest {
    pub label: Option<String>,
}

async fn create_snapshot_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path(land_ident): Path<i32>,
    body: Option<Json<CreateSnapshotRequest>>,
) -> ApiResult<SnapshotSummary> {
    let raw_label = body
        .and_then(|b| b.0.label)
        .unwrap_or_default()
        .trim()
        .to_string();
    let label = if raw_label.is_empty() {
        "수동 스냅샷".to_string()
    } else {
        clamp_utf8(&raw_label, 80)
    };

    let summary = {
        let mut db = hub.database.lock();
        let conn = db.connection_mut();
        let snap_id = match create_snapshot(conn, land_ident, &label) {
            Ok(id) => id,
            Err(diesel::result::Error::NotFound) => {
                return Err(api_err(StatusCode::NOT_FOUND, "Estate not found"));
            }
            Err(e) => {
                return Err(api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
            }
        };
        list_snapshots(conn, land_ident)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .into_iter()
            .find(|s| s.id == snap_id)
            .unwrap_or(SnapshotSummary {
                id: snap_id,
                land_ident,
                label: label.clone(),
                created_at: 0,
            })
    };

    hub.emit_event(HousingSseEvent::new(
        "snapshot_created",
        Some(land_ident),
        format!("스냅샷 저장 완료 (#{} · {})", summary.id, summary.label),
    ));

    Ok(Json(summary))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct RestoreSnapshotRequest {
    pub reload_mode: ReloadMode,
    pub preserve_pos: Option<bool>,
    pub source: Option<String>,
}

async fn restore_snapshot_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path((land_ident, snapshot_id)): Path<(i32, i32)>,
    body: Option<Json<RestoreSnapshotRequest>>,
) -> ApiResult<SnapshotSummary> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let restored = {
        let mut db = hub.database.lock();
        match restore_snapshot(db.connection_mut(), land_ident, snapshot_id) {
            Ok(row) => row,
            Err(diesel::result::Error::NotFound) => {
                return Err(api_err(StatusCode::NOT_FOUND, "Snapshot not found"));
            }
            Err(e) => {
                return Err(api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
            }
        }
    };

    let summary = SnapshotSummary {
        id: snapshot_id,
        land_ident,
        label: format!("restored #{snapshot_id}"),
        created_at: restored.updated_at,
    };
    hub.emit_event(
        HousingSseEvent::new(
            "snapshot_restored",
            Some(land_ident),
            format!("스냅샷 #{} 상태로 복원했습니다.", summary.id),
        )
        .with_source(req.source.clone()),
    );
    if req.reload_mode != ReloadMode::None {
        hub.send_to_server(ToServer::HousingSim(
            HousingSimToServer::NotifyClientsEstateReload {
                land_ident,
                mode: req.reload_mode,
                preserve_player_pos: req.preserve_pos.unwrap_or(true),
                enter_if_elsewhere: false,
                source: req.source,
            },
        ))
        .await;
    }
    Ok(Json(summary))
}

async fn undo_estate_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path(land_ident): Path<i32>,
    body: Option<Json<RestoreSnapshotRequest>>,
) -> ApiResult<SnapshotSummary> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let (snap_id, restored) = {
        let mut db = hub.database.lock();
        match restore_latest_snapshot(db.connection_mut(), land_ident) {
            Ok(pair) => pair,
            Err(diesel::result::Error::NotFound) => {
                return Err(api_err(StatusCode::NOT_FOUND, "No snapshot available"));
            }
            Err(e) => {
                return Err(api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
            }
        }
    };

    let summary = SnapshotSummary {
        id: snap_id,
        land_ident,
        label: format!("undo #{snap_id}"),
        created_at: restored.updated_at,
    };
    hub.emit_event(
        HousingSseEvent::new(
            "snapshot_restored",
            Some(land_ident),
            format!("실행 취소: 스냅샷 #{snap_id} 상태로 복원했습니다."),
        )
        .with_source(req.source.clone()),
    );
    if req.reload_mode != ReloadMode::None {
        hub.send_to_server(ToServer::HousingSim(
            HousingSimToServer::NotifyClientsEstateReload {
                land_ident,
                mode: req.reload_mode,
                preserve_player_pos: req.preserve_pos.unwrap_or(true),
                enter_if_elsewhere: false,
                source: req.source,
            },
        ))
        .await;
    }
    Ok(Json(summary))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ExportQuery {
    pub scope: PresetScope,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ExportPostRequest {
    pub scope: PresetScope,
    pub save_to_disk: bool,
    pub filename: Option<String>,
}

async fn export_remakeplace_get_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path(land_ident): Path<i32>,
    Query(query): Query<ExportQuery>,
) -> ApiResult<ReMakePlaceLayout> {
    let layout = build_export_for_ident(&hub, land_ident, query.scope)?;
    Ok(Json(layout))
}

async fn export_remakeplace_post_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path(land_ident): Path<i32>,
    Json(req): Json<ExportPostRequest>,
) -> ApiResult<serde_json::Value> {
    let layout = build_export_for_ident(&hub, land_ident, req.scope)?;
    let mut saved_path: Option<String> = None;
    if req.save_to_disk {
        let dir = PathBuf::from("housing-exports");
        let _ = std::fs::create_dir_all(&dir);
        let raw_name = req
            .filename
            .unwrap_or_else(|| format!("estate_{land_ident}"));
        let file = sanitize_export_filename(&raw_name);
        let path = dir.join(file);
        let text = serde_json::to_string_pretty(&layout)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        std::fs::write(&path, text)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        saved_path = Some(path.display().to_string());
    }
    Ok(Json(serde_json::json!({
        "land_ident": land_ident,
        "saved_path": saved_path,
        "layout": layout,
    })))
}

fn build_export_for_ident(
    hub: &Arc<HousingSimHub>,
    land_ident: i32,
    scope: PresetScope,
) -> Result<ReMakePlaceLayout, (StatusCode, Json<ApiErrorBody>)> {
    let (estate, rows) = {
        let mut db = hub.database.lock();
        let conn = db.connection_mut();
        let est = estate_by_land_ident(conn, land_ident)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Estate not found"))?;
        let rows = list_furniture_for_estate(conn, land_ident, None).unwrap_or_default();
        (est, rows)
    };
    let catalog = hub.resolve_catalog();
    Ok(export_estate_to_layout(&estate, &rows, &catalog, scope))
}

fn resolve_layout_from_request(
    roots: &[PathBuf],
    path: Option<&str>,
    layout: Option<&ReMakePlaceLayout>,
    raw_json: Option<&str>,
) -> Result<ReMakePlaceLayout, String> {
    if let Some(l) = layout {
        return Ok(l.clone());
    }
    if let Some(raw) = raw_json {
        return parse_layout_bytes(raw.as_bytes());
    }
    if let Some(p) = path {
        let resolved = resolve_safe_preset_path(p, roots)?;
        let bytes = std::fs::read(&resolved)
            .map_err(|e| format!("Failed to read {}: {e}", resolved.display()))?;
        return parse_layout_bytes(&bytes);
    }
    Err("Provide either 'path', 'layout', or 'raw_json'".to_string())
}

#[derive(Serialize)]
pub struct BridgeFurnitureSlot {
    pub flat_slot: u16,
    pub item_id: u32,
    pub catalog_id: u16,
    pub name: String,
    pub stain: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_hex: Option<String>,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
    pub rotation: f32,
    pub local_x: f32,
    pub local_y: f32,
    pub local_z: f32,
    pub local_rotation: f32,
    pub ue_loc: [f32; 3],
    pub ue_yaw_deg: f32,
}

#[derive(Serialize)]
pub struct BridgeEstateSnapshot {
    pub land_ident: i32,
    pub revision: u64,
    pub territory_type_id: u16,
    pub plot_size: u8,
    pub is_apartment: bool,
    pub house_size: String,
    pub light_level: i32,
    pub indoor: Vec<BridgeFurnitureSlot>,
    pub outdoor: Vec<BridgeFurnitureSlot>,
}

async fn bridge_snapshot_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path(land_ident): Path<i32>,
) -> ApiResult<BridgeEstateSnapshot> {
    let (estate, rows) = {
        let mut db = hub.database.lock();
        let conn = db.connection_mut();
        let est = estate_by_land_ident(conn, land_ident)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Estate not found"))?;
        let rows = list_furniture_for_estate(conn, land_ident, Some(true)).unwrap_or_default();
        (est, rows)
    };

    let catalog = hub.resolve_catalog();
    let estate_meta: EstateRemakeMeta =
        serde_json::from_str(&estate.remake_meta_json).unwrap_or_default();
    let plot_size_u8 = estate.plot_size.clamp(0, 2) as u8;
    let house_size = estate_meta.house_size.unwrap_or_else(|| {
        if estate.is_apartment {
            "Apartment".to_string()
        } else {
            match plot_size_u8 {
                0 => "Small".to_string(),
                1 => "Medium".to_string(),
                _ => "Large".to_string(),
            }
        }
    });

    let plot_origin = if estate.is_apartment {
        None
    } else {
        plot_spec(
            estate.territory_type_id as u16,
            estate.division as u8,
            estate.plot_index as u8,
        )
    };

    let mut indoor = Vec::new();
    let mut outdoor = Vec::new();

    for row in &rows {
        let Some(ctype) = container_from_i32(row.container_type) else {
            continue;
        };
        let indoors = if is_interior_placed_container(ctype) {
            true
        } else if is_exterior_placed_container(ctype) {
            false
        } else {
            continue;
        };
        let Some(flat_slot) = flat_slot_for_placed_container(ctype, row.slot.max(0) as u16) else {
            continue;
        };

        let item_id = row.item_id.max(0) as u32;
        let catalog_id = row.catalog_id.max(0) as u16;
        let name = catalog
            .items_by_id
            .get(&item_id)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| format!("Item #{item_id}"));
        let stain = row.stain.clamp(0, u8::MAX as i32) as u8;
        let fmeta: FurnitureRemakeMeta =
            serde_json::from_str(&row.remake_meta_json).unwrap_or_default();
        let color_hex = fmeta
            .raw_color_hex
            .or_else(|| (stain > 0).then(|| catalog.stain_rgba_hex(stain)).flatten());

        let world_pos = Position(glam::Vec3A::new(row.pos_x, row.pos_y, row.pos_z));
        let (local_pos, local_rot) = if !indoors && let Some(spec) = plot_origin {
            spec.world_to_local(world_pos, row.rotation)
        } else {
            (world_pos, row.rotation)
        };

        let ue_loc = [
            local_pos.0.x * 100.0,
            local_pos.0.z * 100.0,
            local_pos.0.y * 100.0,
        ];
        let ue_yaw_deg = -local_rot.to_degrees();

        let slot_entry = BridgeFurnitureSlot {
            flat_slot,
            item_id,
            catalog_id,
            name,
            stain,
            color_hex,
            pos_x: row.pos_x,
            pos_y: row.pos_y,
            pos_z: row.pos_z,
            rotation: row.rotation,
            local_x: local_pos.0.x,
            local_y: local_pos.0.y,
            local_z: local_pos.0.z,
            local_rotation: local_rot,
            ue_loc,
            ue_yaw_deg,
        };
        if indoors {
            indoor.push(slot_entry);
        } else {
            outdoor.push(slot_entry);
        }
    }

    indoor.sort_by_key(|s| s.flat_slot);
    outdoor.sort_by_key(|s| s.flat_slot);

    Ok(Json(BridgeEstateSnapshot {
        land_ident,
        revision: hub.current_revision(),
        territory_type_id: estate.territory_type_id as u16,
        plot_size: plot_size_u8,
        is_apartment: estate.is_apartment,
        house_size,
        light_level: estate.light_level,
        indoor,
        outdoor,
    }))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct LiveMoveEntryRequest {
    pub flat_slot: u16,
    pub item_id: Option<u32>,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
    pub rotation: f32,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct LiveMoveRequest {
    pub source: Option<String>,
    pub indoors: bool,
    pub plot_local: bool,
    pub commit_db: bool,
    pub moves: Vec<LiveMoveEntryRequest>,
}

#[derive(Serialize)]
pub struct LiveMoveResponse {
    pub ok: bool,
    pub land_ident: i32,
    pub revision: u64,
    pub applied_count: usize,
}

async fn live_move_handler(
    State(hub): State<Arc<HousingSimHub>>,
    Path(land_ident): Path<i32>,
    Json(req): Json<LiveMoveRequest>,
) -> ApiResult<LiveMoveResponse> {
    if req.moves.is_empty() || req.moves.len() > 64 {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "moves must contain between 1 and 64 entries",
        ));
    }

    for entry in &req.moves {
        if !(entry.pos_x.is_finite()
            && entry.pos_y.is_finite()
            && entry.pos_z.is_finite()
            && entry.rotation.is_finite())
        {
            return Err(api_err(
                StatusCode::BAD_REQUEST,
                "All coordinates and rotations must be finite numbers",
            ));
        }
        if placed_container_for_flat_slot(req.indoors, entry.flat_slot).is_none() {
            return Err(api_err(
                StatusCode::BAD_REQUEST,
                format!("Invalid flat_slot {}", entry.flat_slot),
            ));
        }
    }

    let estate = {
        let mut db = hub.database.lock();
        estate_by_land_ident(db.connection_mut(), land_ident)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or_else(|| api_err(StatusCode::NOT_FOUND, "Estate not found"))?
    };

    let plot_origin = if !req.indoors && !estate.is_apartment {
        plot_spec(
            estate.territory_type_id as u16,
            estate.division as u8,
            estate.plot_index as u8,
        )
    } else {
        None
    };

    struct ResolvedLiveMove {
        flat_slot: u16,
        item_id_hint: Option<u32>,
        world_x: f32,
        world_y: f32,
        world_z: f32,
        world_rot: f32,
        local_x: f32,
        local_y: f32,
        local_z: f32,
        local_rot: f32,
    }

    let mut resolved = Vec::with_capacity(req.moves.len());
    let mut live_moves = Vec::with_capacity(req.moves.len());

    for entry in &req.moves {
        let in_pos = Position(glam::Vec3A::new(entry.pos_x, entry.pos_y, entry.pos_z));
        let in_rot = wrap_angle_radians(entry.rotation);

        let ((world_pos, world_rot), (local_pos, local_rot)) = if !req.indoors
            && let Some(spec) = plot_origin
        {
            if req.plot_local {
                (spec.local_to_world(in_pos, in_rot), (in_pos, in_rot))
            } else {
                ((in_pos, in_rot), spec.world_to_local(in_pos, in_rot))
            }
        } else {
            ((in_pos, in_rot), (in_pos, in_rot))
        };

        live_moves.push(LiveFurnitureMove {
            flat_slot: entry.flat_slot,
            pos_x: world_pos.0.x,
            pos_y: world_pos.0.y,
            pos_z: world_pos.0.z,
            rotation: world_rot,
        });
        resolved.push(ResolvedLiveMove {
            flat_slot: entry.flat_slot,
            item_id_hint: entry.item_id,
            world_x: world_pos.0.x,
            world_y: world_pos.0.y,
            world_z: world_pos.0.z,
            world_rot,
            local_x: local_pos.0.x,
            local_y: local_pos.0.y,
            local_z: local_pos.0.z,
            local_rot,
        });
    }

    let mut delta_items = Vec::new();
    if req.commit_db {
        use diesel::Connection;

        let mut db = hub.database.lock();
        let conn = db.connection_mut();
        let existing_rows = list_placed_furniture_in_scope(conn, land_ident, req.indoors)
            .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let mut slot_meta = std::collections::HashMap::with_capacity(existing_rows.len());
        for row in &existing_rows {
            if let Some(ctype) = container_from_i32(row.container_type)
                && let Some(flat) = flat_slot_for_placed_container(ctype, row.slot.max(0) as u16)
            {
                slot_meta.insert(
                    flat,
                    (row.item_id.max(0) as u32, row.catalog_id.max(0) as u16),
                );
            }
        }

        conn.transaction::<_, diesel::result::Error, _>(|conn| {
            for mv in &resolved {
                if let Some((ctype, slot_in_page)) =
                    placed_container_for_flat_slot(req.indoors, mv.flat_slot)
                {
                    update_furniture_position(
                        conn,
                        land_ident,
                        ctype as i32,
                        slot_in_page as i32,
                        mv.world_x,
                        mv.world_y,
                        mv.world_z,
                        mv.world_rot,
                    )?;
                }
            }
            Ok(())
        })
        .map_err(|e| api_err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        for mv in &resolved {
            let (row_item, row_cat) = slot_meta.get(&mv.flat_slot).copied().unwrap_or((0, 0));
            delta_items.push(HousingBridgeMoveItem {
                flat_slot: mv.flat_slot,
                item_id: mv.item_id_hint.unwrap_or(row_item),
                catalog_id: row_cat,
                pos_x: mv.world_x,
                pos_y: mv.world_y,
                pos_z: mv.world_z,
                rotation: mv.world_rot,
                local_x: mv.local_x,
                local_y: mv.local_y,
                local_z: mv.local_z,
                local_rotation: mv.local_rot,
            });
        }
    }

    let applied_count = live_moves.len();
    hub.send_to_server(ToServer::HousingSim(
        HousingSimToServer::BroadcastEstateLiveMoves {
            land_ident,
            indoors: req.indoors,
            moves: live_moves,
        },
    ))
    .await;

    let rev = if req.commit_db {
        let rev = hub.next_revision();
        hub.emit_event(
            HousingSseEvent::new(
                "furniture_moved",
                Some(land_ident),
                format!("실시간 가구 이동 ({applied_count}개)"),
            )
            .with_source(req.source)
            .with_revision(rev)
            .with_delta(HousingBridgeDelta::FurnitureMoved {
                indoors: req.indoors,
                moves: delta_items,
            }),
        );
        rev
    } else {
        hub.next_revision()
    };

    Ok(Json(LiveMoveResponse {
        ok: true,
        land_ident,
        revision: rev,
        applied_count,
    }))
}

#[cfg(test)]
mod tests {
    use super::super::gamedata_ext::HousingCatalog;
    use super::*;
    use crate::database::WorldDatabase;
    use crate::housing_sim::remake_place::{ReMakePlaceFurniture, ReMakePlaceTransform};
    use crate::housing_sim::session::HousingSimToServer;
    use parking_lot::Mutex;
    use tokio::sync::mpsc;

    fn test_hub() -> (Arc<HousingSimHub>, mpsc::Receiver<ToServer>) {
        let (tx, rx) = mpsc::channel(32);
        let db = Arc::new(Mutex::new(WorldDatabase::new_in_memory()));
        let handle = crate::common::ServerHandle {
            chan: tx,
            next_id: Arc::new(std::sync::atomic::AtomicUsize::new(1)),
        };
        let hub = Arc::new(HousingSimHub::new_for_tests(
            handle,
            db,
            HousingCatalog::mock_for_tests(),
        ));
        (hub, rx)
    }

    #[tokio::test]
    async fn http_api_end_to_end_ensure_apply_export_undo() {
        let (hub, mut rx) = test_hub();

        let ensured = ensure_estate_handler(
            State(hub.clone()),
            Json(EnsureEstateRequest {
                territory_type_id: 340,
                world_id: 79,
                ward_index: 0,
                division: 0,
                plot_index: Some(0),
                plot_size: Some(2),
                owner_content_id: 42,
                owner_name: "Tester".to_string(),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;

        let layout = ReMakePlaceLayout {
            house_size: "Large".to_string(),
            interior_furniture: vec![ReMakePlaceFurniture {
                name: "Alpine Chair".to_string(),
                item_id: 21873,
                transform: ReMakePlaceTransform {
                    location: [100.0, 200.0, 50.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                ..Default::default()
            }],
            ..Default::default()
        };

        let applied = apply_preset_handler(
            State(hub.clone()),
            Path(ensured.land_ident),
            Json(ApplyPresetRequest {
                layout: Some(layout),
                scope: PresetScope::Interior,
                reload_mode: ReloadMode::LivePackets,
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(applied.preview.interior_resolved_count, 1);
        assert!(applied.snapshot_id.is_some());

        // Expect reload notification dispatched to server channel
        let server_msg = rx.try_recv().unwrap();
        assert!(matches!(
            server_msg,
            ToServer::HousingSim(HousingSimToServer::NotifyClientsEstateReload {
                mode: ReloadMode::LivePackets,
                ..
            })
        ));

        let exported = export_remakeplace_get_handler(
            State(hub.clone()),
            Path(ensured.land_ident),
            Query(ExportQuery {
                scope: PresetScope::Interior,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(exported.interior_furniture.len(), 1);
        assert_eq!(exported.interior_furniture[0].item_id, 21873);

        let undone = undo_estate_handler(
            State(hub.clone()),
            Path(ensured.land_ident),
            Some(Json(RestoreSnapshotRequest {
                reload_mode: ReloadMode::None,
                ..Default::default()
            })),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(undone.id, applied.snapshot_id.unwrap());

        let detail = get_estate_handler(State(hub), Path(ensured.land_ident))
            .await
            .unwrap()
            .0;
        assert!(detail.furniture.is_empty());
    }

    #[tokio::test]
    async fn http_session_switch_creates_and_switches_across_districts_and_sizes_for_one_character()
    {
        let (hub, mut rx) = test_hub();

        let res_mist_large = switch_session_handler(
            State(hub.clone()),
            Json(SwitchSessionRequest {
                district: Some("mist".to_string()),
                plot_size: Some(2),
                enter_interior: Some(true),
                owner_content_id: Some(42),
                owner_name: Some("Tester".to_string()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(res_mist_large.estate.territory_type_id, 339);
        assert_eq!(res_mist_large.estate.plot_size, 2);
        assert_eq!(res_mist_large.target_indoor_territory, 284);
        assert_eq!(
            plot_spec(339, 0, res_mist_large.estate.plot_index as u8)
                .unwrap()
                .size,
            PlotSize::Large
        );
        assert!(matches!(
            rx.try_recv().unwrap(),
            ToServer::HousingSim(HousingSimToServer::SwitchClientEstate {
                content_id: Some(42),
                land_ident,
                enter_interior: Some(true),
            }) if land_ident == res_mist_large.estate.land_ident
        ));

        let res_shiro_small = switch_session_handler(
            State(hub.clone()),
            Json(SwitchSessionRequest {
                district: Some("shirogane".to_string()),
                plot_size: Some(0),
                enter_interior: Some(true),
                owner_content_id: Some(42),
                owner_name: Some("Tester".to_string()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(res_shiro_small.estate.territory_type_id, 641);
        assert_eq!(res_shiro_small.estate.plot_size, 0);
        assert_eq!(res_shiro_small.target_indoor_territory, 649);
        assert_eq!(
            plot_spec(641, 0, res_shiro_small.estate.plot_index as u8)
                .unwrap()
                .size,
            PlotSize::Small
        );

        let res_emp_apt = switch_session_handler(
            State(hub.clone()),
            Json(SwitchSessionRequest {
                district: Some("엠피레움".to_string()),
                is_apartment: Some(true),
                room_number: Some(7),
                owner_content_id: Some(42),
                owner_name: Some("Tester".to_string()),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(res_emp_apt.estate.territory_type_id, 979);
        assert!(res_emp_apt.estate.is_apartment);
        assert_eq!(res_emp_apt.target_indoor_territory, 999);

        let all = list_estates_handler(State(hub)).await.unwrap().0;
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn http_unlock_all_and_give_kit_dispatch_server_messages() {
        let Some(game_data) = crate::test_game_data() else {
            return;
        };
        let (hub, mut rx) = test_hub();
        {
            let mut db = hub.database.lock();
            db.create_player_data(
                1,
                "UnlockTarget",
                &crate::CharaMake {
                    customize: physis::savedata::chardat::CustomizeData {
                        race: physis::race::Race::Hyur,
                        tribe: physis::race::Tribe::Highlander,
                        ..Default::default()
                    },
                    classjob_id: 1,
                    ..Default::default()
                }
                .to_json(),
                1,
                181,
                crate::inventory::Inventory::default(),
                game_data,
            );
        }

        let res = unlock_all_session_handler(State(hub.clone()), None)
            .await
            .unwrap()
            .0;
        assert_eq!(res.unlocked_characters, 1);
        assert_eq!(res.character_names, vec!["UnlockTarget".to_string()]);
        assert!(matches!(
            rx.try_recv().unwrap(),
            ToServer::HousingSim(HousingSimToServer::UnlockClientSession { content_id: None })
        ));

        let kit = give_kit_handler(
            State(hub),
            Json(GiveKitRequest {
                kind: "indoor".to_string(),
                content_id: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(kit.kind, "indoor");
        assert!(matches!(
            rx.try_recv().unwrap(),
            ToServer::HousingSim(HousingSimToServer::GiveClientSampleKit {
                content_id: None,
                ref kind,
            }) if kind == "indoor"
        ));
    }

    #[tokio::test]
    async fn live_move_streams_without_db_churn_and_commits_when_requested() {
        let (hub, mut rx) = test_hub();
        let mut event_rx = hub.event_tx.subscribe();

        let ensured = ensure_estate_handler(
            State(hub.clone()),
            Json(EnsureEstateRequest {
                territory_type_id: 340,
                world_id: 79,
                ward_index: 0,
                division: 0,
                plot_index: Some(0),
                plot_size: Some(2),
                owner_content_id: 42,
                owner_name: "Tester".to_string(),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;

        let layout = ReMakePlaceLayout {
            house_size: "Large".to_string(),
            interior_scale: 100.0,
            exterior_scale: 100.0,
            interior_furniture: vec![ReMakePlaceFurniture {
                name: "Alpine Chair".to_string(),
                item_id: 21873,
                transform: ReMakePlaceTransform {
                    location: [100.0, 200.0, 50.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                ..Default::default()
            }],
            exterior_furniture: vec![ReMakePlaceFurniture {
                name: "Yard Bench".to_string(),
                item_id: 12000,
                transform: ReMakePlaceTransform {
                    location: [400.0, 200.0, 100.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0, 1.0, 1.0],
                },
                ..Default::default()
            }],
            ..Default::default()
        };

        let applied = apply_preset_handler(
            State(hub.clone()),
            Path(ensured.land_ident),
            Json(ApplyPresetRequest {
                layout: Some(layout),
                scope: PresetScope::All,
                reload_mode: ReloadMode::LivePackets,
                source: Some("rmp".to_string()),
                skip_snapshot: true,
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(
            applied.snapshot_id.is_none(),
            "skip_snapshot=true should skip snapshot"
        );
        let _ = rx.try_recv().unwrap();

        // Drain initial SSE events up to preset_applied
        let mut saw_rmp_preset_sse = false;
        while let Ok(evt) = event_rx.try_recv() {
            if evt.kind == "preset_applied" {
                assert_eq!(evt.source.as_deref(), Some("rmp"));
                assert!(evt.revision.is_some());
                saw_rmp_preset_sse = true;
            }
        }
        assert!(saw_rmp_preset_sse);

        // 1. Live drag frame (commit_db=false): broadcasts live packets without mutating SQLite or SSE.
        let drag_res = live_move_handler(
            State(hub.clone()),
            Path(ensured.land_ident),
            Json(LiveMoveRequest {
                source: Some("rmp".to_string()),
                indoors: true,
                plot_local: true,
                commit_db: false,
                moves: vec![LiveMoveEntryRequest {
                    flat_slot: 0,
                    item_id: Some(21873),
                    pos_x: 3.5,
                    pos_y: 1.25,
                    pos_z: -2.0,
                    rotation: 0.75,
                }],
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(drag_res.ok);
        assert_eq!(drag_res.applied_count, 1);
        assert!(
            event_rx.try_recv().is_err(),
            "drag frame should not emit SSE"
        );

        assert!(matches!(
            rx.try_recv().unwrap(),
            ToServer::HousingSim(HousingSimToServer::BroadcastEstateLiveMoves {
                land_ident,
                indoors: true,
                ref moves,
            }) if land_ident == ensured.land_ident && moves.len() == 1 && (moves[0].pos_x - 3.5).abs() < 1e-4
        ));

        let detail_before_commit = get_estate_handler(State(hub.clone()), Path(ensured.land_ident))
            .await
            .unwrap()
            .0;
        let pre_commit_indoor = detail_before_commit
            .furniture
            .iter()
            .find(|r| r.item_id == 21873)
            .unwrap();
        assert!((pre_commit_indoor.pos_x - 1.0).abs() < 1e-4);

        // 2. Final commit move (commit_db=true): updates DB and emits structured SSE delta.
        let commit_res = live_move_handler(
            State(hub.clone()),
            Path(ensured.land_ident),
            Json(LiveMoveRequest {
                source: Some("rmp".to_string()),
                indoors: true,
                plot_local: true,
                commit_db: true,
                moves: vec![LiveMoveEntryRequest {
                    flat_slot: 0,
                    item_id: Some(21873),
                    pos_x: 3.5,
                    pos_y: 1.25,
                    pos_z: -2.0,
                    rotation: 0.75,
                }],
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(commit_res.revision, drag_res.revision + 1);

        let evt = event_rx.try_recv().unwrap();
        assert_eq!(evt.kind, "furniture_moved");
        assert_eq!(evt.source.as_deref(), Some("rmp"));
        assert_eq!(evt.revision, Some(commit_res.revision));
        assert!(matches!(
            evt.delta,
            Some(HousingBridgeDelta::FurnitureMoved { indoors: true, ref moves }) if moves.len() == 1 && (moves[0].local_x - 3.5).abs() < 1e-4
        ));

        let detail_after_commit = get_estate_handler(State(hub.clone()), Path(ensured.land_ident))
            .await
            .unwrap()
            .0;
        let committed_indoor = detail_after_commit
            .furniture
            .iter()
            .find(|r| r.item_id == 21873)
            .unwrap();
        assert!((committed_indoor.pos_x - 3.5).abs() < 1e-4);

        // 3. Outdoor plot_local live move transforms local -> world for DB and broadcasts,
        //    and bridge_snapshot returns exact UE cm local coordinates!
        let ext_commit = live_move_handler(
            State(hub.clone()),
            Path(ensured.land_ident),
            Json(LiveMoveRequest {
                source: Some("rmp".to_string()),
                indoors: false,
                plot_local: true,
                commit_db: true,
                moves: vec![LiveMoveEntryRequest {
                    flat_slot: 0,
                    item_id: Some(12000),
                    pos_x: 2.25,
                    pos_y: 0.75,
                    pos_z: -1.5,
                    rotation: 0.4,
                }],
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(ext_commit.ok);
        let _ = rx.try_recv().unwrap();
        let ext_evt = event_rx.try_recv().unwrap();
        assert!(matches!(
            ext_evt.delta,
            Some(HousingBridgeDelta::FurnitureMoved { indoors: false, ref moves })
                if moves.len() == 1
                    && (moves[0].local_x - 2.25).abs() < 1e-3
                    && (moves[0].pos_x - 2.25).abs() > 5.0
        ));

        let snap = bridge_snapshot_handler(State(hub), Path(ensured.land_ident))
            .await
            .unwrap()
            .0;
        assert_eq!(snap.indoor.len(), 1);
        assert_eq!(snap.indoor[0].flat_slot, 0);
        assert!((snap.indoor[0].ue_loc[0] - 350.0).abs() < 0.1);
        assert!((snap.indoor[0].ue_loc[1] - (-200.0)).abs() < 0.1);
        assert!((snap.indoor[0].ue_loc[2] - 125.0).abs() < 0.1);
        assert!((snap.indoor[0].ue_yaw_deg - (-0.75f32.to_degrees())).abs() < 0.1);

        assert!(snap.outdoor.len() == 1);
        assert_eq!(snap.outdoor[0].flat_slot, 0);
        assert!((snap.outdoor[0].ue_loc[0] - 225.0).abs() < 0.5);
        assert!((snap.outdoor[0].ue_loc[1] - (-150.0)).abs() < 0.5);
        assert!((snap.outdoor[0].ue_loc[2] - 75.0).abs() < 0.5);
        assert!((snap.outdoor[0].ue_yaw_deg - (-0.4f32.to_degrees())).abs() < 0.5);
    }

    #[tokio::test]
    async fn http_manual_snapshot_and_delete_estate_lifecycle() {
        let (hub, _rx) = test_hub();

        let ensured = ensure_estate_handler(
            State(hub.clone()),
            Json(EnsureEstateRequest {
                territory_type_id: 340,
                world_id: 79,
                ward_index: 2,
                division: 0,
                plot_index: Some(0),
                plot_size: Some(0),
                owner_content_id: 77,
                owner_name: "SnapshotTester".to_string(),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .0;

        let created = create_snapshot_handler(
            State(hub.clone()),
            Path(ensured.land_ident),
            Some(Json(CreateSnapshotRequest {
                label: Some("  before remodel  ".to_string()),
            })),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(created.land_ident, ensured.land_ident);
        assert_eq!(created.label, "before remodel");

        let listed = list_snapshots_handler(State(hub.clone()), Path(ensured.land_ident))
            .await
            .unwrap()
            .0;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let deleted = delete_estate_handler(State(hub.clone()), Path(ensured.land_ident))
            .await
            .unwrap()
            .0;
        assert_eq!(deleted["ok"], true);
        assert_eq!(deleted["land_ident"], ensured.land_ident);

        let second_delete =
            delete_estate_handler(State(hub.clone()), Path(ensured.land_ident)).await;
        assert!(matches!(second_delete, Err((StatusCode::NOT_FOUND, _))));

        let get_after_delete = get_estate_handler(State(hub), Path(ensured.land_ident)).await;
        assert!(matches!(get_after_delete, Err((StatusCode::NOT_FOUND, _))));
    }
}
