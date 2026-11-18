use std::path::PathBuf;

use kawari::common::{HouseId, Position};
use kawari::ipc::zone::{
    ActorControlCategory, HousingInteriorDetails, PlotSize, ServerZoneIpcData, ServerZoneIpcSegment,
};

use crate::ZoneConnection;
use crate::inventory::Item;

use super::db::{
    MAX_ESTATE_GREETING_BYTES, MAX_ESTATE_NAME_BYTES, clamp_utf8, create_snapshot,
    delete_furniture_slot, estate_by_land_ident, list_placed_furniture_in_scope, list_snapshots,
    restore_latest_snapshot, save_estate,
};
use super::fixtures::{StoredHouseExterior, StoredHouseInterior};
use super::flows::{
    apply_preset_file_to_estate, build_selection_from_row, connection_catalog,
    ensure_local_apartment_estate, ensure_local_plot_estate, execute_housing_reload,
    publish_session_snapshot, push_indoor_live_refresh, rebuild_house_inventory_from_db,
    warp_into_estate_interior, warp_to_estate_outdoor,
};
use super::gamedata_ext::{
    display_name_for_renovation, normalize_district_style_name, renovation_row_for_style_and_size,
};
use super::plots::{
    city_fallback_aetheryte_for_territory, default_district_for_zone, is_apartment_territory,
    is_house_interior_territory, nearest_plot_to_position,
};
use super::remake_place::{
    PresetScope, export_estate_to_layout, parse_layout_bytes, prepare_apply_layout,
    read_layout_file, resolve_safe_preset_path, scan_preset_directory,
};
use super::session::{HousingSseEvent, ReloadMode};

/// Attempts to parse and execute `!housing ...` chat commands for any player.
pub async fn try_handle_housing_chat_command(
    connection: &mut ZoneConnection,
    raw_message: &str,
) -> bool {
    let trimmed = raw_message.trim();
    let Some(rest) = strip_housing_prefix(trimmed) else {
        return false;
    };

    let tokens = split_command_tokens(rest);
    if tokens.is_empty() {
        send_help(connection).await;
        return true;
    }

    let sub = tokens[0].to_ascii_lowercase();
    let args = &tokens[1..];
    match sub.as_str() {
        "help" | "?" => send_help(connection).await,
        "testhouse" | "house" => cmd_testhouse(connection, args).await,
        "apartment" | "apt" => cmd_apartment(connection, args).await,
        "enter" => cmd_enter(connection, args).await,
        "exit" | "leave" => cmd_exit(connection).await,
        "reload" => cmd_reload(connection, args).await,
        "info" | "status" => cmd_info(connection).await,
        "reset" => cmd_reset(connection, args).await,
        "undo" => cmd_undo(connection).await,
        "name" => cmd_name(connection, args).await,
        "greeting" => cmd_greeting(connection, args).await,
        "light" => cmd_light(connection, args).await,
        "interior" | "pattern" => cmd_interior(connection, args).await,
        "exterior" => cmd_exterior(connection, args).await,
        "preset" => cmd_preset(connection, args).await,
        "export" => cmd_export(connection, args).await,
        "givekit" | "kit" => cmd_givekit(connection, args).await,
        other => {
            connection
                .send_notice(&format!(
                    "[Housing] Unknown subcommand '{other}'. Use !housing help."
                ))
                .await;
        }
    }

    true
}

fn strip_housing_prefix(trimmed: &str) -> Option<&str> {
    let lower = trimmed.to_ascii_lowercase();
    if lower == "!housing" {
        return Some("");
    }
    if lower.starts_with("!housing ") || lower.starts_with("!housing\t") {
        return Some(trimmed[8..].trim());
    }
    None
}

fn split_command_tokens(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
            }
            '\\' if in_quotes => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            other => current.push(other),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

async fn send_help(connection: &mut ZoneConnection) {
    for line in [
        "[Housing Simulator Commands]",
        "  !housing testhouse [ward] [plot] [small|medium|large] [sub|main]",
        "  !housing apartment [ward] [sub|main] [room]",
        "  !housing enter | !housing exit | !housing reload [live|zone] | !housing info",
        "  !housing preset [check|latest|repeat] [all|interior|exterior] <path> [--reload=auto|live|zone|none]",
        "  !housing export [all|interior|exterior] [name] | !housing undo | !housing reset [all|interior|exterior]",
        "  !housing interior <1..21|style> | !housing light <0..5> | !housing name <text> | !housing greeting <text>",
        "  !housing givekit <indoor|outdoor|npc>",
    ] {
        connection.send_notice(line).await;
    }
}

fn parse_plot_size(raw: &str) -> Option<PlotSize> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "s" | "small" | "cottage" => Some(PlotSize::Small),
        "m" | "medium" | "house" => Some(PlotSize::Medium),
        "l" | "large" | "mansion" => Some(PlotSize::Large),
        _ => None,
    }
}

fn parse_reload_mode(raw: &str) -> Option<ReloadMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ReloadMode::Auto),
        "live" | "live_packets" | "packets" => Some(ReloadMode::LivePackets),
        "zone" | "zone_reload" => Some(ReloadMode::ZoneReload),
        "none" | "off" => Some(ReloadMode::None),
        _ => None,
    }
}

fn active_land_ident(connection: &ZoneConnection) -> Option<i32> {
    connection
        .housing_sim
        .active_estate
        .as_ref()
        .map(|s| s.land_ident)
}

async fn cmd_testhouse(connection: &mut ZoneConnection, args: &[String]) {
    let zone_id = connection.player_data.volatile.zone_id as u16;
    let in_ward = connection
        .housing_sim
        .display_ward_context
        .filter(|ctx| ctx.territory_type_id == zone_id);

    let mut ward_index = in_ward.map(|c| c.ward_index).unwrap_or(0);
    let mut division = in_ward.map(|c| c.division).unwrap_or(0);
    let mut plot_index: Option<u8> = nearest_plot_to_position(
        default_district_for_zone(zone_id),
        connection.player_data.volatile.position,
    )
    .map(|(div, plot)| {
        division = div;
        plot
    });
    let mut requested_size: Option<PlotSize> = None;
    let mut numeric_args: Vec<u16> = Vec::new();

    for arg in args {
        let low = arg.to_ascii_lowercase();
        if let Some(sz) = parse_plot_size(&low) {
            requested_size = Some(sz);
        } else if low == "sub" || low == "subdivision" {
            division = 1;
        } else if low == "main" {
            division = 0;
        } else if let Ok(n) = low.parse::<u16>() {
            numeric_args.push(n);
        }
    }

    if numeric_args.len() >= 2 {
        ward_index = numeric_args[0].saturating_sub(1).min(29) as u8;
        let raw_plot = numeric_args[1].saturating_sub(1).min(59) as u8;
        if raw_plot >= 30 {
            division = 1;
            plot_index = Some(raw_plot - 30);
        } else {
            plot_index = Some(raw_plot);
        }
    } else if numeric_args.len() == 1 {
        let n = numeric_args[0];
        let raw_plot = n.saturating_sub(1).min(59) as u8;
        if raw_plot >= 30 {
            division = 1;
            plot_index = Some(raw_plot - 30);
        } else {
            plot_index = Some(raw_plot);
        }
    }

    let district = default_district_for_zone(zone_id);
    let plot_idx = plot_index.unwrap_or(0).min(29);

    let Some(estate) = ensure_local_plot_estate(
        connection,
        district,
        ward_index,
        division,
        plot_idx,
        requested_size,
        false,
        None,
    ) else {
        connection
            .send_notice("[Housing] Failed to create local simulator estate.")
            .await;
        return;
    };

    connection.housing_sim.active_estate = Some(build_selection_from_row(&estate, false));
    connection.housing_sim.is_owner_authorized = true;
    super::flows::send_owned_housing(connection).await;

    connection
        .send_notice(&format!(
            "[Housing] Ready: '{}' (land_ident={}, W{}-P{}, {:?}). Warping to estate...",
            estate.estate_name,
            estate.land_ident,
            estate.ward_index + 1,
            estate.division * 30 + estate.plot_index + 1,
            PlotSize::from_repr(estate.plot_size.clamp(0, 2) as u8).unwrap_or_default(),
        ))
        .await;

    warp_to_estate_outdoor(connection, &estate, None, None).await;
}

async fn cmd_apartment(connection: &mut ZoneConnection, args: &[String]) {
    let zone_id = connection.player_data.volatile.zone_id as u16;
    let district = default_district_for_zone(zone_id);
    let mut ward_index = connection
        .housing_sim
        .display_ward_context
        .map(|c| c.ward_index)
        .unwrap_or(0);
    let mut division = connection
        .housing_sim
        .display_ward_context
        .map(|c| c.division)
        .unwrap_or(0);
    let mut room_number = 1u16;
    let mut nums = Vec::new();

    for arg in args {
        let low = arg.to_ascii_lowercase();
        if low == "sub" || low == "subdivision" {
            division = 1;
        } else if low == "main" {
            division = 0;
        } else if let Ok(n) = low.parse::<u16>() {
            nums.push(n);
        }
    }
    if nums.len() >= 2 {
        ward_index = nums[0].saturating_sub(1).min(29) as u8;
        room_number = nums[1].clamp(1, 90);
    } else if nums.len() == 1 {
        room_number = nums[0].clamp(1, 90);
    }

    let Some(estate) =
        ensure_local_apartment_estate(connection, district, ward_index, division, room_number)
    else {
        connection
            .send_notice("[Housing] Failed to create local apartment.")
            .await;
        return;
    };

    connection.housing_sim.active_estate = Some(build_selection_from_row(&estate, true));
    connection.housing_sim.is_owner_authorized = true;
    super::flows::send_owned_housing(connection).await;
    connection
        .send_notice(&format!(
            "[Housing] Entering apartment room #{} (land_ident={})...",
            estate.room_number, estate.land_ident
        ))
        .await;
    warp_into_estate_interior(connection, &estate, None, None).await;
}

async fn cmd_enter(connection: &mut ZoneConnection, args: &[String]) {
    let zone_id = connection.player_data.volatile.zone_id as u16;
    let estate = if !args.is_empty()
        && let Ok(raw_plot) = args[0].parse::<u8>()
    {
        let district = default_district_for_zone(zone_id);
        let ward_index = connection
            .housing_sim
            .display_ward_context
            .map(|c| c.ward_index)
            .unwrap_or(0);
        let idx = raw_plot.saturating_sub(1).min(59);
        let (division, plot_index) = if idx >= 30 { (1, idx - 30) } else { (0, idx) };
        ensure_local_plot_estate(
            connection, district, ward_index, division, plot_index, None, false, None,
        )
    } else if let Some(ident) = active_land_ident(connection) {
        let mut db = connection.database.lock();
        estate_by_land_ident(db.connection_mut(), ident)
            .ok()
            .flatten()
    } else if let Some((div, plot)) = nearest_plot_to_position(
        default_district_for_zone(zone_id),
        connection.player_data.volatile.position,
    ) {
        let district = default_district_for_zone(zone_id);
        let ward_index = connection
            .housing_sim
            .display_ward_context
            .map(|c| c.ward_index)
            .unwrap_or(0);
        ensure_local_plot_estate(
            connection, district, ward_index, div, plot, None, false, None,
        )
    } else {
        None
    };

    let Some(est) = estate else {
        connection
            .send_notice("[Housing] No estate found to enter. Run !housing testhouse first.")
            .await;
        return;
    };
    warp_into_estate_interior(connection, &est, None, None).await;
}

async fn cmd_exit(connection: &mut ZoneConnection) {
    let Some(ident) = active_land_ident(connection) else {
        let zone_id = connection.player_data.volatile.zone_id as u16;
        let aetheryte = city_fallback_aetheryte_for_territory(zone_id);
        let _ = aetheryte;
        connection
            .send_notice("[Housing] No active indoor estate selected.")
            .await;
        return;
    };
    let estate = {
        let mut db = connection.database.lock();
        estate_by_land_ident(db.connection_mut(), ident)
            .ok()
            .flatten()
    };
    let Some(est) = estate else {
        connection
            .send_notice("[Housing] Active estate record missing.")
            .await;
        return;
    };
    warp_to_estate_outdoor(connection, &est, None, None).await;
}

async fn cmd_reload(connection: &mut ZoneConnection, args: &[String]) {
    let mode = args
        .first()
        .and_then(|s| parse_reload_mode(s))
        .unwrap_or(ReloadMode::Auto);
    execute_housing_reload(connection, None, mode, true, false).await;
    connection
        .send_notice(&format!("[Housing] Reload triggered ({mode:?})."))
        .await;
}

async fn cmd_info(connection: &mut ZoneConnection) {
    let Some(ident) = active_land_ident(connection) else {
        connection
            .send_notice("[Housing] No active estate selected.")
            .await;
        return;
    };
    let (estate, indoor_count, outdoor_count, snap_count) = {
        let mut db = connection.database.lock();
        let conn = db.connection_mut();
        let estate = estate_by_land_ident(conn, ident).ok().flatten();
        let indoor = list_placed_furniture_in_scope(conn, ident, true)
            .map(|r| r.len())
            .unwrap_or(0);
        let outdoor = list_placed_furniture_in_scope(conn, ident, false)
            .map(|r| r.len())
            .unwrap_or(0);
        let snaps = list_snapshots(conn, ident).map(|s| s.len()).unwrap_or(0);
        (estate, indoor, outdoor, snaps)
    };
    let Some(est) = estate else {
        connection
            .send_notice("[Housing] Active estate no longer exists.")
            .await;
        return;
    };
    let int = StoredHouseInterior::from_json(&est.interior_json);
    connection
        .send_notice(&format!(
            "[Housing] land_ident={} house_id=0x{:X} name='{}' territory={} W{}-P{} apt={} room={} size={:?} pattern={} ('{}') light={} ssao={} indoor={} outdoor={} snapshots={}",
            est.land_ident,
            est.house_id as u64,
            est.estate_name,
            est.territory_type_id,
            est.ward_index + 1,
            est.division * 30 + est.plot_index + 1,
            est.is_apartment,
            est.room_number,
            PlotSize::from_repr(est.plot_size.clamp(0, 2) as u8).unwrap_or_default(),
            int.renovation_row_id,
            display_name_for_renovation(int.renovation_row_id),
            est.light_level,
            est.enable_ssao,
            indoor_count,
            outdoor_count,
            snap_count,
        ))
        .await;
}

async fn cmd_reset(connection: &mut ZoneConnection, args: &[String]) {
    let Some(ident) = active_land_ident(connection) else {
        connection
            .send_notice("[Housing] No active estate selected.")
            .await;
        return;
    };
    let scope = args
        .first()
        .and_then(|s| PresetScope::from_str_loose(s))
        .unwrap_or(PresetScope::All);
    let catalog = connection_catalog(connection);
    {
        let mut db = connection.database.lock();
        let conn = db.connection_mut();
        let Some(mut est) = estate_by_land_ident(conn, ident).ok().flatten() else {
            return;
        };
        let _ = create_snapshot(conn, ident, &format!("reset ({scope:?})"));
        let size = PlotSize::from_repr(est.plot_size.clamp(0, 2) as u8).unwrap_or_default();
        if scope.includes_interior() {
            let def_int = StoredHouseInterior::default_for_location(
                &catalog,
                est.territory_type_id as u16,
                size,
                est.is_apartment,
            );
            est.interior_json = def_int.to_json();
        }
        if scope.includes_exterior() && !est.is_apartment {
            let def_ext = StoredHouseExterior::default_for_location(
                &catalog,
                est.territory_type_id as u16,
                size,
            );
            est.exterior_json = def_ext.to_json();
        }
        let _ = save_estate(conn, &est);

        if scope.includes_interior() {
            for r in list_placed_furniture_in_scope(conn, ident, true).unwrap_or_default() {
                let _ = delete_furniture_slot(conn, ident, r.container_type, r.slot);
            }
        }
        if scope.includes_exterior() {
            for r in list_placed_furniture_in_scope(conn, ident, false).unwrap_or_default() {
                let _ = delete_furniture_slot(conn, ident, r.container_type, r.slot);
            }
        }
    }
    execute_housing_reload(connection, Some(ident), ReloadMode::Auto, true, false).await;
    connection
        .send_notice(&format!("[Housing] Reset completed for scope {scope:?}."))
        .await;
}

async fn cmd_undo(connection: &mut ZoneConnection) {
    let Some(ident) = active_land_ident(connection) else {
        connection
            .send_notice("[Housing] No active estate selected.")
            .await;
        return;
    };
    let restored = {
        let mut db = connection.database.lock();
        restore_latest_snapshot(db.connection_mut(), ident)
    };
    match restored {
        Ok((snap_id, _)) => {
            if let Some(ref hub) = connection.housing_hub {
                hub.emit_event(HousingSseEvent {
                    kind: "snapshot_restored".to_string(),
                    land_ident: Some(ident),
                    message: format!("Restored snapshot #{snap_id}"),
                    session: None,
                });
            }
            execute_housing_reload(connection, Some(ident), ReloadMode::Auto, true, false).await;
            connection
                .send_notice(&format!("[Housing] Restored snapshot #{snap_id}."))
                .await;
        }
        Err(diesel::result::Error::NotFound) => {
            connection
                .send_notice("[Housing] No snapshots available to undo.")
                .await;
        }
        Err(err) => {
            connection
                .send_notice(&format!("[Housing] Undo failed: {err}"))
                .await;
        }
    }
}

async fn cmd_name(connection: &mut ZoneConnection, args: &[String]) {
    let Some(ident) = active_land_ident(connection) else {
        return;
    };
    let new_name = clamp_utf8(&args.join(" "), MAX_ESTATE_NAME_BYTES);
    {
        let mut db = connection.database.lock();
        if let Some(mut est) = estate_by_land_ident(db.connection_mut(), ident)
            .ok()
            .flatten()
        {
            est.estate_name = new_name.clone();
            let _ = save_estate(db.connection_mut(), &est);
        }
    }
    connection
        .send_notice(&format!("[Housing] Estate name set to '{new_name}'."))
        .await;
}

async fn cmd_greeting(connection: &mut ZoneConnection, args: &[String]) {
    let Some(ident) = active_land_ident(connection) else {
        return;
    };
    let new_greeting = clamp_utf8(&args.join(" "), MAX_ESTATE_GREETING_BYTES);
    {
        let mut db = connection.database.lock();
        if let Some(mut est) = estate_by_land_ident(db.connection_mut(), ident)
            .ok()
            .flatten()
        {
            est.greeting = new_greeting.clone();
            let _ = save_estate(db.connection_mut(), &est);
        }
    }
    connection
        .send_notice(&format!(
            "[Housing] Greeting updated ({} bytes).",
            new_greeting.len()
        ))
        .await;
}

async fn cmd_light(connection: &mut ZoneConnection, args: &[String]) {
    let Some(ident) = active_land_ident(connection) else {
        return;
    };
    let Some(level) = args.first().and_then(|s| s.parse::<u8>().ok()) else {
        connection
            .send_notice("[Housing] Usage: !housing light <0..5> [ssao:on|off]")
            .await;
        return;
    };
    let level = level.min(5);
    let ssao_override = args
        .get(1)
        .and_then(|s| match s.to_ascii_lowercase().as_str() {
            "on" | "1" | "true" | "ssao:on" => Some(true),
            "off" | "0" | "false" | "ssao:off" => Some(false),
            _ => None,
        });
    let est_opt = {
        let mut db = connection.database.lock();
        if let Some(mut est) = estate_by_land_ident(db.connection_mut(), ident)
            .ok()
            .flatten()
        {
            est.light_level = level as i32;
            if let Some(ssao) = ssao_override {
                est.enable_ssao = ssao;
            }
            let _ = save_estate(db.connection_mut(), &est);
            Some(est)
        } else {
            None
        }
    };
    if let Some(est) = est_opt {
        let z = connection.player_data.volatile.zone_id as u16;
        if is_house_interior_territory(z) || is_apartment_territory(z) {
            push_indoor_live_refresh(connection, &est).await;
            connection
                .actor_control_self(ActorControlCategory::InteriorLightLevel {
                    unk1: 0,
                    level: level as u32,
                    unk2: u32::from(est.enable_ssao),
                })
                .await;
        }
        connection
            .send_notice(&format!(
                "[Housing] Light level set to {level}, SSAO={}.",
                est.enable_ssao
            ))
            .await;
    }
}

async fn cmd_interior(connection: &mut ZoneConnection, args: &[String]) {
    let Some(ident) = active_land_ident(connection) else {
        return;
    };
    if args.is_empty() {
        connection
            .send_notice("[Housing] Usage: !housing interior <1..21 | minimal|mist|lavender|goblet|shirogane|empyreum>")
            .await;
        return;
    }
    let arg = args.join(" ");
    let est_opt = {
        let mut db = connection.database.lock();
        let conn = db.connection_mut();
        estate_by_land_ident(conn, ident).ok().flatten()
    };
    let Some(mut est) = est_opt else {
        return;
    };
    let size = PlotSize::from_repr(est.plot_size.clamp(0, 2) as u8).unwrap_or_default();
    let row_id = if let Ok(n) = arg.trim().parse::<u16>() {
        n
    } else if let Some(style) = normalize_district_style_name(&arg) {
        renovation_row_for_style_and_size(style, size).unwrap_or(0)
    } else {
        0
    };
    if !(1..=21).contains(&row_id) {
        connection
            .send_notice(
                "[Housing] Invalid interior renovation pattern (expected 1..21 or district style).",
            )
            .await;
        return;
    }

    {
        let mut db = connection.database.lock();
        let conn = db.connection_mut();
        let _ = create_snapshot(conn, ident, &format!("interior pattern {row_id}"));
        let mut int = StoredHouseInterior::from_json(&est.interior_json);
        int.renovation_row_id = row_id;
        est.interior_json = int.to_json();
        let _ = save_estate(conn, &est);
    }
    execute_housing_reload(connection, Some(ident), ReloadMode::ZoneReload, true, false).await;
    connection
        .send_notice(&format!(
            "[Housing] Interior layout set to pattern {row_id} ('{}').",
            display_name_for_renovation(row_id)
        ))
        .await;
}

async fn cmd_exterior(connection: &mut ZoneConnection, args: &[String]) {
    let Some(ident) = active_land_ident(connection) else {
        return;
    };
    if args
        .first()
        .map(|s| s.eq_ignore_ascii_case("reset"))
        .unwrap_or(false)
    {
        let catalog = connection_catalog(connection);
        {
            let mut db = connection.database.lock();
            if let Some(mut est) = estate_by_land_ident(db.connection_mut(), ident)
                .ok()
                .flatten()
            {
                let _ = create_snapshot(db.connection_mut(), ident, "exterior reset");
                let size = PlotSize::from_repr(est.plot_size.clamp(0, 2) as u8).unwrap_or_default();
                est.exterior_json = StoredHouseExterior::default_for_location(
                    &catalog,
                    est.territory_type_id as u16,
                    size,
                )
                .to_json();
                let _ = save_estate(db.connection_mut(), &est);
            }
        }
        execute_housing_reload(connection, Some(ident), ReloadMode::Auto, true, false).await;
        connection
            .send_notice("[Housing] Exterior fixtures reset to district default.")
            .await;
        return;
    }

    let est_opt = {
        let mut db = connection.database.lock();
        estate_by_land_ident(db.connection_mut(), ident)
            .ok()
            .flatten()
    };
    if let Some(est) = est_opt {
        let ext = StoredHouseExterior::from_json(&est.exterior_json);
        connection
            .send_notice(&format!(
                "[Housing Exterior] roof={}:{} walls={}:{} win={}:{} door={}:{} roofFx={}:{} wallFx={}:{} placard={}:{} fence={}:{}",
                ext.roof_id,
                ext.roof_stain,
                ext.walls_id,
                ext.walls_stain,
                ext.windows_id,
                ext.windows_stain,
                ext.door_id,
                ext.door_stain,
                ext.roof_fixture_id,
                ext.roof_fixture_stain,
                ext.wall_fixture_id,
                ext.wall_fixture_stain,
                ext.above_door_banner_id,
                ext.above_door_banner_stain,
                ext.fence_id,
                ext.fence_stain,
            ))
            .await;
    }
}

async fn cmd_preset(connection: &mut ZoneConnection, args: &[String]) {
    let mut dry_run = false;
    let mut use_latest = false;
    let mut use_repeat = false;
    let mut scope = PresetScope::All;
    let mut reload_mode = ReloadMode::Auto;
    let mut path_parts: Vec<String> = Vec::new();

    for arg in args {
        let low = arg.to_ascii_lowercase();
        if low == "check" || low == "--check" || low == "preview" {
            dry_run = true;
        } else if low == "latest" {
            use_latest = true;
        } else if low == "repeat" {
            use_repeat = true;
        } else if let Some(s) = PresetScope::from_str_loose(&low) {
            scope = s;
        } else if let Some(rest) = low.strip_prefix("--reload=")
            && let Some(m) = parse_reload_mode(rest)
        {
            reload_mode = m;
        } else {
            path_parts.push(arg.clone());
        }
    }

    let roots = connection
        .housing_hub
        .as_ref()
        .map(|h| h.configured_preset_roots())
        .unwrap_or_else(|| {
            vec![
                PathBuf::from("housing-exports"),
                PathBuf::from("resources/housing-presets"),
            ]
        });

    let resolved_path = if use_repeat {
        if let Some(ref hub) = connection.housing_hub
            && let Some((last_path, last_scope)) = hub.last_applied_preset.lock().clone()
        {
            if !args
                .iter()
                .any(|a| PresetScope::from_str_loose(a).is_some())
            {
                scope = last_scope;
            }
            last_path
        } else {
            connection
                .send_notice("[Housing] No previous preset recorded for repeat.")
                .await;
            return;
        }
    } else if use_latest {
        let mut all_files = Vec::new();
        for root in &roots {
            all_files.extend(scan_preset_directory(root));
        }
        all_files.sort_by(|a, b| b.modified_unix_ms.cmp(&a.modified_unix_ms));
        if let Some(newest) = all_files.first() {
            PathBuf::from(&newest.path)
        } else {
            connection
                .send_notice("[Housing] No preset .json files found under configured preset roots.")
                .await;
            return;
        }
    } else {
        let raw_path = path_parts.join(" ");
        if raw_path.is_empty() {
            connection
                .send_notice("[Housing] Usage: !housing preset [check|latest|repeat] [all|interior|exterior] <path> [--reload=auto|live|zone|none]")
                .await;
            return;
        }
        match resolve_safe_preset_path(&raw_path, &roots) {
            Ok(p) => p,
            Err(err) => {
                connection
                    .send_notice(&format!("[Housing] Invalid preset path: {err}"))
                    .await;
                return;
            }
        }
    };

    let Some(ident) = active_land_ident(connection) else {
        connection
            .send_notice("[Housing] No active estate selected. Run !housing testhouse or !housing apartment first.")
            .await;
        return;
    };

    if dry_run {
        let bytes = match std::fs::read(&resolved_path) {
            Ok(b) => b,
            Err(err) => {
                connection
                    .send_notice(&format!("[Housing] Read failed: {err}"))
                    .await;
                return;
            }
        };
        let layout = match parse_layout_bytes(&bytes) {
            Ok(l) => l,
            Err(err) => {
                connection
                    .send_notice(&format!("[Housing] Parse failed: {err}"))
                    .await;
                return;
            }
        };
        let estate = {
            let mut db = connection.database.lock();
            estate_by_land_ident(db.connection_mut(), ident)
                .ok()
                .flatten()
        };
        let Some(est) = estate else {
            return;
        };
        let catalog = connection_catalog(connection);
        let prepared = prepare_apply_layout(
            &est,
            &layout,
            &catalog,
            scope,
            connection.player_data.character.content_id as u64,
        );
        let p = &prepared.preview;
        connection
            .send_notice(&format!(
                "[Housing Preset Check] {} | size={} | indoor {}/{} (trunc={}) | outdoor {}/{} (trunc={}) | unresolved={} | pattern_changed={}",
                resolved_path.display(),
                p.house_size,
                p.interior_resolved_count,
                p.interior_input_count,
                p.interior_truncated_count,
                p.exterior_resolved_count,
                p.exterior_input_count,
                p.exterior_truncated_count,
                p.interior_unresolved_items.len() + p.exterior_unresolved_items.len(),
                p.renovation_territory_changed,
            ))
            .await;
        for w in p.warnings.iter().take(4) {
            connection.send_notice(&format!("  warn: {w}")).await;
        }
        return;
    }

    let content_id = connection.player_data.character.content_id as u64;
    let label = format!(
        "preset: {}",
        resolved_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("layout.json")
    );
    let res = apply_preset_file_to_estate(
        &connection.database,
        &connection.game_data,
        ident,
        &resolved_path,
        scope,
        content_id,
        &label,
    );
    match res {
        Ok((prepared, snap_id)) => {
            if let Some(ref hub) = connection.housing_hub {
                *hub.last_applied_preset.lock() = Some((resolved_path.clone(), scope));
                hub.emit_event(HousingSseEvent {
                    kind: "preset_applied".to_string(),
                    land_ident: Some(ident),
                    message: format!(
                        "Applied {} (indoor={}/{}, outdoor={}/{}, snap={:?})",
                        resolved_path.display(),
                        prepared.preview.interior_resolved_count,
                        prepared.preview.interior_input_count,
                        prepared.preview.exterior_resolved_count,
                        prepared.preview.exterior_input_count,
                        snap_id,
                    ),
                    session: None,
                });
            }
            connection
                .send_notice(&format!(
                    "[Housing] Applied {} (snap={:?}): indoor {}/{}, outdoor {}/{}, warnings={}.",
                    resolved_path.display(),
                    snap_id.unwrap_or(0),
                    prepared.preview.interior_resolved_count,
                    prepared.preview.interior_input_count,
                    prepared.preview.exterior_resolved_count,
                    prepared.preview.exterior_input_count,
                    prepared.preview.warnings.len(),
                ))
                .await;

            // Force zone reload automatically when renovation territory changed.
            let effective_reload = if prepared.preview.renovation_territory_changed
                && reload_mode != ReloadMode::None
            {
                ReloadMode::ZoneReload
            } else {
                reload_mode
            };
            execute_housing_reload(connection, Some(ident), effective_reload, true, false).await;
        }
        Err(err) => {
            connection
                .send_notice(&format!("[Housing] Apply failed: {err}"))
                .await;
        }
    }
    let _ = read_layout_file;
    let _ = publish_session_snapshot;
}

async fn cmd_export(connection: &mut ZoneConnection, args: &[String]) {
    let Some(ident) = active_land_ident(connection) else {
        connection
            .send_notice("[Housing] No active estate selected.")
            .await;
        return;
    };

    let mut scope = PresetScope::All;
    let mut name_arg: Option<String> = None;
    for arg in args {
        if let Some(s) = PresetScope::from_str_loose(arg) {
            scope = s;
        } else {
            name_arg = Some(arg.clone());
        }
    }

    let (estate, rows) = {
        let mut db = connection.database.lock();
        let conn = db.connection_mut();
        let est = estate_by_land_ident(conn, ident).ok().flatten();
        let rows = super::db::list_furniture_for_estate(conn, ident, None).unwrap_or_default();
        (est, rows)
    };
    let Some(est) = estate else {
        return;
    };

    let catalog = connection_catalog(connection);
    let layout = export_estate_to_layout(&est, &rows, &catalog, scope);

    let export_dir = PathBuf::from("housing-exports");
    let _ = std::fs::create_dir_all(&export_dir);
    let raw_name = name_arg.unwrap_or_else(|| format!("estate_{ident}"));
    let safe_stem: String = raw_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let filename = if safe_stem.ends_with(".json") {
        safe_stem
    } else {
        format!("{safe_stem}.json")
    };
    let out_path = export_dir.join(filename);
    match serde_json::to_string_pretty(&layout) {
        Ok(json) => match std::fs::write(&out_path, json) {
            Ok(()) => {
                connection
                    .send_notice(&format!(
                        "[Housing] Exported to {} (indoor={}, outdoor={}).",
                        out_path.display(),
                        layout.interior_furniture.len(),
                        layout.exterior_furniture.len(),
                    ))
                    .await;
            }
            Err(err) => {
                connection
                    .send_notice(&format!("[Housing] Write export failed: {err}"))
                    .await;
            }
        },
        Err(err) => {
            connection
                .send_notice(&format!("[Housing] Serialize export failed: {err}"))
                .await;
        }
    }
}

async fn cmd_givekit(connection: &mut ZoneConnection, args: &[String]) {
    let kind = args.first().map(|s| s.as_str()).unwrap_or("indoor");
    let catalog = connection_catalog(connection);
    let items = catalog.sample_kit_item_ids(kind);
    if items.is_empty() {
        connection
            .send_notice("[Housing] Usage: !housing givekit <indoor|outdoor|npc>")
            .await;
        return;
    }

    let mut added = 0usize;
    for item_id in items {
        let mut inserted = false;
        for page_idx in 0..4 {
            for slot_idx in 0..35u16 {
                let ctype = match page_idx {
                    0 => kawari::common::ContainerType::Inventory0,
                    1 => kawari::common::ContainerType::Inventory1,
                    2 => kawari::common::ContainerType::Inventory2,
                    _ => kawari::common::ContainerType::Inventory3,
                };
                if let Some(slot) = connection.player_data.inventory.get_item(ctype, slot_idx)
                    && slot.item_id == 0
                {
                    connection.player_data.inventory.add_in_slot(
                        Item {
                            item_id,
                            quantity: 1,
                            ..Default::default()
                        },
                        &ctype,
                        slot_idx,
                    );
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
    connection.send_inventory().await;
    connection
        .send_notice(&format!(
            "[Housing] Added {added} sample '{kind}' furnishings to inventory."
        ))
        .await;
    let _ = (
        Position::default(),
        HouseId::default(),
        HousingInteriorDetails::default(),
        ServerZoneIpcData::InitResponse {
            actor_id: Default::default(),
        },
        ServerZoneIpcSegment::new(ServerZoneIpcData::InitResponse {
            actor_id: Default::default(),
        }),
        rebuild_house_inventory_from_db,
    );
}
