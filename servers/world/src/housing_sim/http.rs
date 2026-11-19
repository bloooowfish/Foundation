use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream::{self, Stream};
use kawari::common::HousingFlag;
use kawari::ipc::zone::PlotSize;
use serde_derive::{Deserialize, Serialize};
use tower_http::cors::CorsLayer;

use crate::common::ToServer;

use super::db::{
    EnsureEstateSpec, HousingEstateRow, HousingFurnitureRow, MAX_ESTATE_GREETING_BYTES,
    MAX_ESTATE_NAME_BYTES, SnapshotSummary, clamp_utf8, create_snapshot, ensure_estate,
    estate_by_land_ident, list_all_estates, list_furniture_for_estate,
    list_placed_furniture_in_scope, list_snapshots, reset_estate_scope, restore_latest_snapshot,
    restore_snapshot, save_estate,
};
use super::fixtures::{StoredHouseExterior, StoredHouseInterior};
use super::flows::{
    apply_preset_layout_with_catalog, build_selection_from_row, target_indoor_territory_for_estate,
    unlock_characters_in_database,
};
use super::gamedata_ext::{
    default_renovation_row_for_district_and_size, display_name_for_renovation,
    normalize_district_style_name, renovation_row_for_style_and_size,
};
use super::plots::{
    canonical_plot_for_district_and_size, default_district_for_zone, parse_district_territory,
    plot_spec,
};
use super::remake_place::{
    PresetFileSummary, PresetScope, ReMakePlaceLayout, ReMakePlacePreviewReport,
    export_estate_to_layout, parse_layout_bytes, prepare_apply_layout, read_layout_file,
    resolve_safe_preset_path, sanitize_export_filename, scan_preset_directory,
};
use super::session::{
    HousingSimHub, HousingSimToServer, HousingSseEvent, OnlineSessionSnapshot, ReloadMode,
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
            get(get_estate_handler).patch(patch_estate_handler),
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
            get(list_snapshots_handler),
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
    hub.emit_event(HousingSseEvent {
        kind: "estate_ensured".to_string(),
        land_ident: Some(row.land_ident),
        message: format!(
            "하우스 준비 완료: {}",
            if row.estate_name.is_empty() {
                "테스트 하우스"
            } else {
                &row.estate_name
            }
        ),
        session: None,
    });
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
    hub.emit_event(HousingSseEvent {
        kind: "session_switched".to_string(),
        land_ident: Some(estate.land_ident),
        message: format!(
            "활성 하우스 전환: {}",
            if estate.estate_name.is_empty() {
                "테스트 하우스"
            } else {
                &estate.estate_name
            }
        ),
        session: None,
    });
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

    hub.emit_event(HousingSseEvent {
        kind: "session_unlocked".to_string(),
        land_ident: None,
        message: if names.is_empty() {
            "해금 대상 캐릭터가 없습니다.".to_string()
        } else {
            format!("전체 해금 완료: {}", names.join(", "))
        },
        session: None,
    });

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

    hub.emit_event(HousingSseEvent {
        kind: "kit_granted".to_string(),
        land_ident: None,
        message: format!("샘플 가구 키트 전송 완료 ({normalized})"),
        session: None,
    });

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
    hub.emit_event(HousingSseEvent {
        kind: "estate_updated".to_string(),
        land_ident: Some(land_ident),
        message: "하우스 설정이 업데이트되었습니다.".to_string(),
        session: None,
    });
    if mode != ReloadMode::None {
        hub.send_to_server(ToServer::HousingSim(
            HousingSimToServer::NotifyClientsEstateReload {
                land_ident,
                mode,
                preserve_player_pos: true,
                enter_if_elsewhere: false,
            },
        ))
        .await;
    }

    Ok(Json(updated))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ResetEstateRequest {
    pub scope: PresetScope,
    pub reload_mode: ReloadMode,
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

    hub.emit_event(HousingSseEvent {
        kind: "estate_reset".to_string(),
        land_ident: Some(land_ident),
        message: format!("하우스 비우기 완료 (가구 {removed_count}개 제거)"),
        session: None,
    });
    if req.reload_mode != ReloadMode::None {
        hub.send_to_server(ToServer::HousingSim(
            HousingSimToServer::NotifyClientsEstateReload {
                land_ident,
                mode: req.reload_mode,
                preserve_player_pos: true,
                enter_if_elsewhere: false,
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
        let res = apply_preset_layout_with_catalog(
            &hub.database,
            &catalog,
            land_ident,
            &layout,
            req.scope,
            content_id,
            &label,
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
        apply_preset_layout_with_catalog(
            &hub.database,
            &catalog,
            land_ident,
            &layout,
            req.scope,
            content_id,
            &label,
        )
        .map_err(|e| api_err(StatusCode::BAD_REQUEST, e))?
    };

    let effective_reload =
        if prepared.preview.renovation_territory_changed && req.reload_mode != ReloadMode::None {
            ReloadMode::ZoneReload
        } else {
            req.reload_mode
        };

    hub.emit_event(HousingSseEvent {
        kind: "preset_applied".to_string(),
        land_ident: Some(land_ident),
        message: format!(
            "프리셋 적용 완료: '{}' (실내 {}/{}, 정원 {}/{})",
            label,
            prepared.preview.interior_resolved_count,
            prepared.preview.interior_input_count,
            prepared.preview.exterior_resolved_count,
            prepared.preview.exterior_input_count,
        ),
        session: None,
    });

    if effective_reload != ReloadMode::None {
        hub.send_to_server(ToServer::HousingSim(
            HousingSimToServer::NotifyClientsEstateReload {
                land_ident,
                mode: effective_reload,
                preserve_player_pos: req.preserve_pos.unwrap_or(true),
                enter_if_elsewhere: req.force_enter,
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
pub struct RestoreSnapshotRequest {
    pub reload_mode: ReloadMode,
    pub preserve_pos: Option<bool>,
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
    hub.emit_event(HousingSseEvent {
        kind: "snapshot_restored".to_string(),
        land_ident: Some(land_ident),
        message: format!("스냅샷 #{} 상태로 복원했습니다.", summary.id),
        session: None,
    });
    if req.reload_mode != ReloadMode::None {
        hub.send_to_server(ToServer::HousingSim(
            HousingSimToServer::NotifyClientsEstateReload {
                land_ident,
                mode: req.reload_mode,
                preserve_player_pos: req.preserve_pos.unwrap_or(true),
                enter_if_elsewhere: false,
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
    hub.emit_event(HousingSseEvent {
        kind: "snapshot_restored".to_string(),
        land_ident: Some(land_ident),
        message: format!("실행 취소: 스냅샷 #{snap_id} 상태로 복원했습니다."),
        session: None,
    });
    if req.reload_mode != ReloadMode::None {
        hub.send_to_server(ToServer::HousingSim(
            HousingSimToServer::NotifyClientsEstateReload {
                land_ident,
                mode: req.reload_mode,
                preserve_player_pos: req.preserve_pos.unwrap_or(true),
                enter_if_elsewhere: false,
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
}
