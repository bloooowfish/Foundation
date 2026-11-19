use std::collections::BTreeMap;

use diesel::prelude::*;
use diesel::sql_types::Text;
use diesel::sqlite::SqliteConnection;
use kawari::common::{HouseId, HouseUnit, HousingFlag, PlotState, timestamp_secs};
use kawari::ipc::zone::{HouseExterior, PlotSize};
use serde_derive::{Deserialize, Serialize};

use super::containers::placed_container_ids_for_scope;

pub const MAX_ESTATE_NAME_BYTES: usize = 20;
pub const MAX_ESTATE_GREETING_BYTES: usize = 192;
pub const MAX_OWNER_NAME_BYTES: usize = 30;
pub const MAX_SNAPSHOTS_PER_ESTATE: usize = 25;

pub mod schema {
    diesel::table! {
        housing_estates (land_ident) {
            land_ident -> Integer,
            house_id -> BigInt,
            territory_type_id -> Integer,
            world_id -> Integer,
            ward_index -> Integer,
            division -> Integer,
            plot_index -> Integer,
            room_number -> Integer,
            is_apartment -> Bool,
            plot_size -> Integer,
            owner_type -> Integer,
            owner_content_id -> BigInt,
            owner_name -> Text,
            fc_id -> BigInt,
            estate_name -> Text,
            greeting -> Text,
            flags -> Integer,
            price -> Integer,
            appeal_tag_0 -> Integer,
            appeal_tag_1 -> Integer,
            appeal_tag_2 -> Integer,
            light_level -> Integer,
            enable_ssao -> Bool,
            exterior_json -> Text,
            interior_json -> Text,
            remake_meta_json -> Text,
            updated_at -> BigInt,
        }
    }

    diesel::table! {
        housing_furniture (land_ident, container_type, slot) {
            land_ident -> Integer,
            container_type -> Integer,
            slot -> Integer,
            item_id -> Integer,
            catalog_id -> Integer,
            material_item_key -> Integer,
            stain -> Integer,
            placed -> Bool,
            pos_x -> Float,
            pos_y -> Float,
            pos_z -> Float,
            rotation -> Float,
            object_data_json -> Text,
            remake_meta_json -> Text,
            created_by_content_id -> BigInt,
            updated_at -> BigInt,
        }
    }

    diesel::table! {
        housing_snapshots (id) {
            id -> Integer,
            land_ident -> Integer,
            label -> Text,
            estate_json -> Text,
            furniture_json -> Text,
            created_at -> BigInt,
        }
    }
}

/// Persistent estate/apartment record in `housing_estates`.
#[derive(
    Insertable,
    Identifiable,
    Queryable,
    Selectable,
    AsChangeset,
    Serialize,
    Deserialize,
    Debug,
    Default,
    Clone,
)]
#[diesel(table_name = schema::housing_estates)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
#[diesel(primary_key(land_ident))]
pub struct HousingEstateRow {
    pub land_ident: i32,
    pub house_id: i64,
    pub territory_type_id: i32,
    pub world_id: i32,
    pub ward_index: i32,
    pub division: i32,
    pub plot_index: i32,
    pub room_number: i32,
    pub is_apartment: bool,
    pub plot_size: i32,
    pub owner_type: i32,
    pub owner_content_id: i64,
    pub owner_name: String,
    pub fc_id: i64,
    pub estate_name: String,
    pub greeting: String,
    pub flags: i32,
    pub price: i32,
    pub appeal_tag_0: i32,
    pub appeal_tag_1: i32,
    pub appeal_tag_2: i32,
    pub light_level: i32,
    pub enable_ssao: bool,
    pub exterior_json: String,
    pub interior_json: String,
    pub remake_meta_json: String,
    pub updated_at: i64,
}

/// Persistent furniture / storeroom record in `housing_furniture`.
#[derive(
    Insertable, Queryable, Selectable, AsChangeset, Serialize, Deserialize, Debug, Default, Clone,
)]
#[diesel(table_name = schema::housing_furniture)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
#[diesel(primary_key(land_ident, container_type, slot))]
pub struct HousingFurnitureRow {
    pub land_ident: i32,
    pub container_type: i32,
    pub slot: i32,
    pub item_id: i32,
    pub catalog_id: i32,
    pub material_item_key: i32,
    pub stain: i32,
    pub placed: bool,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
    pub rotation: f32,
    pub object_data_json: String,
    pub remake_meta_json: String,
    pub created_by_content_id: i64,
    pub updated_at: i64,
}

/// Snapshot metadata and payload stored in `housing_snapshots`.
#[derive(Queryable, Selectable, Serialize, Deserialize, Debug, Default, Clone)]
#[diesel(table_name = schema::housing_snapshots)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct HousingSnapshotRow {
    pub id: i32,
    pub land_ident: i32,
    pub label: String,
    pub estate_json: String,
    pub furniture_json: String,
    pub created_at: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = schema::housing_snapshots)]
struct NewHousingSnapshotRow<'a> {
    pub land_ident: i32,
    pub label: &'a str,
    pub estate_json: &'a str,
    pub furniture_json: &'a str,
    pub created_at: i64,
}

/// Lossless MakePlace/ReMakePlace per-item metadata stored in `housing_furniture.remake_meta_json`.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
#[serde(default)]
pub struct FurnitureRemakeMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_slot: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_color_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_rotation_quat: Option<[f32; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<[f32; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material_item_id: Option<u32>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub extra_properties: BTreeMap<String, serde_json::Value>,
}

/// Interactive furnishing state stored in `housing_furniture.object_data_json`.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
#[serde(default)]
pub struct FurnitureObjectData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aquarium: Option<AquariumObjectState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_a: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_b: Vec<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub param_c: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
#[serde(default)]
pub struct AquariumObjectState {
    pub trim_item_id: u32,
    pub water_param: u16,
    pub fish_item_ids: [u32; 4],
    pub fish_params: [u16; 4],
}

/// Lossless MakePlace/ReMakePlace estate metadata stored in `housing_estates.remake_meta_json`.
#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
#[serde(default)]
pub struct EstateRemakeMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub house_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interior_scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exterior_scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player_transform: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, serde_json::Value>,
}

/// Specification passed when ensuring a local simulator estate exists.
#[derive(Debug, Clone)]
pub struct EnsureEstateSpec {
    pub territory_type_id: u16,
    pub world_id: u16,
    pub ward_index: u8,
    pub division: u8,
    pub plot_index: u8,
    pub room_number: u16,
    pub is_apartment: bool,
    pub plot_size: PlotSize,
    pub is_fc: bool,
    pub owner_content_id: u64,
    pub owner_name: String,
    pub estate_name: String,
    pub exterior_json: String,
    pub interior_json: String,
}

/// Summary descriptor returned when querying snapshots without full layout payload.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SnapshotSummary {
    pub id: i32,
    pub land_ident: i32,
    pub label: String,
    pub created_at: i64,
}

/// Clamps a UTF-8 string to fit within `max_bytes` on a character boundary.
pub fn clamp_utf8(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_string()
}

/// Packs `(territory_type_id, ward_index, division, plot_index, room_number, is_apartment, world_id)` into a protocol `HouseId`.
pub fn make_house_id(
    territory_type_id: u16,
    world_id: u16,
    ward_index: u8,
    division: u8,
    plot_index: u8,
    room_number: u16,
    is_apartment: bool,
) -> HouseId {
    let raw_unit = if is_apartment {
        division & 0x01
    } else {
        (division & 0x01) * 30 + (plot_index % 30)
    };
    HouseId {
        unit: HouseUnit {
            apartment_division_plot_index: raw_unit,
            apartment_flag: is_apartment,
        },
        unk1: 0,
        ward_index: ward_index & 0x3F,
        room_number,
        territory_type_id,
        world_id,
    }
}

/// Computes a deterministic positive `land_ident` for any estate or apartment location.
pub fn compute_land_ident(
    territory_type_id: u16,
    ward_index: u8,
    division: u8,
    plot_index: u8,
    room_number: u16,
    is_apartment: bool,
) -> i32 {
    if is_apartment {
        let div = (division as i32) & 1;
        1_000_000_000
            + ((territory_type_id as i32) & 0x0FFF) * 1_000_000
            + ((ward_index as i32) & 0x3F) * 10_000
            + div * 1_000
            + (room_number as i32)
    } else {
        let raw_plot = ((division as i32) & 1) * 30 + (plot_index as i32 % 30);
        ((territory_type_id as i32) & 0x0FFF) * 100_000
            + ((ward_index as i32) & 0x3F) * 100
            + raw_plot
            + 1
    }
}

#[derive(QueryableByName)]
struct TableColumnName {
    #[diesel(sql_type = Text)]
    name: String,
}

/// Idempotently creates or repairs the `housing_sim` SQLite tables on pre-existing or fresh databases.
pub fn ensure_schema(conn: &mut SqliteConnection) {
    let ddl = include_str!("../../migrations/zz_housing/up.sql");
    for stmt in ddl.split(';') {
        let trimmed = stmt.trim();
        if trimmed.is_empty() {
            continue;
        }
        diesel::sql_query(trimmed)
            .execute(conn)
            .expect("Failed to execute housing_sim DDL statement");
    }

    ensure_column(
        conn,
        "housing_estates",
        "enable_ssao",
        "BOOLEAN NOT NULL DEFAULT 1",
    );
    ensure_column(
        conn,
        "housing_estates",
        "remake_meta_json",
        "TEXT NOT NULL DEFAULT '{}'",
    );
    ensure_column(
        conn,
        "housing_furniture",
        "material_item_key",
        "INTEGER NOT NULL DEFAULT 0",
    );
    ensure_column(
        conn,
        "housing_furniture",
        "object_data_json",
        "TEXT NOT NULL DEFAULT '{}'",
    );
    ensure_column(
        conn,
        "housing_furniture",
        "remake_meta_json",
        "TEXT NOT NULL DEFAULT '{}'",
    );
}

fn ensure_column(conn: &mut SqliteConnection, table: &str, col_name: &str, col_def: &str) {
    let query = format!("PRAGMA table_info({table})");
    if let Ok(cols) = diesel::sql_query(query).load::<TableColumnName>(conn)
        && !cols.iter().any(|c| c.name == col_name)
    {
        let alter = format!("ALTER TABLE {table} ADD COLUMN {col_name} {col_def}");
        diesel::sql_query(alter)
            .execute(conn)
            .expect("Failed to add housing_sim compatibility column");
    }
}

/// Ensures an estate or apartment exists at the target location, updating owner/name metadata when created.
pub fn ensure_estate(
    conn: &mut SqliteConnection,
    spec: &EnsureEstateSpec,
) -> QueryResult<HousingEstateRow> {
    use schema::housing_estates::dsl::*;

    let raw_house_id = u64::from(make_house_id(
        spec.territory_type_id,
        spec.world_id,
        spec.ward_index,
        spec.division,
        spec.plot_index,
        spec.room_number,
        spec.is_apartment,
    )) as i64;

    let ident = compute_land_ident(
        spec.territory_type_id,
        spec.ward_index,
        spec.division,
        spec.plot_index,
        spec.room_number,
        spec.is_apartment,
    );

    if let Some(existing) = housing_estates
        .filter(house_id.eq(raw_house_id).or(land_ident.eq(ident)))
        .select(HousingEstateRow::as_select())
        .first::<HousingEstateRow>(conn)
        .optional()?
    {
        return Ok(existing);
    }

    let mut flag_bits = HousingFlag::OPEN;
    if spec.is_fc {
        flag_bits |= HousingFlag::OWNED_BY_FC;
    }

    let now = timestamp_secs() as i64;
    let new_row = HousingEstateRow {
        land_ident: ident,
        house_id: raw_house_id,
        territory_type_id: spec.territory_type_id as i32,
        world_id: spec.world_id as i32,
        ward_index: spec.ward_index as i32,
        division: (spec.division & 1) as i32,
        plot_index: if spec.is_apartment {
            0
        } else {
            (spec.plot_index % 30) as i32
        },
        room_number: spec.room_number as i32,
        is_apartment: spec.is_apartment,
        plot_size: spec.plot_size as i32,
        owner_type: if spec.is_fc { 1 } else { 0 },
        owner_content_id: spec.owner_content_id as i64,
        owner_name: clamp_utf8(&spec.owner_name, MAX_OWNER_NAME_BYTES),
        fc_id: if spec.is_fc {
            spec.owner_content_id as i64 & 0x7FFF_FFFF
        } else {
            0
        },
        estate_name: clamp_utf8(&spec.estate_name, MAX_ESTATE_NAME_BYTES),
        greeting: String::new(),
        flags: flag_bits.bits() as i32,
        price: 0,
        appeal_tag_0: 0,
        appeal_tag_1: 0,
        appeal_tag_2: 0,
        light_level: 0,
        enable_ssao: true,
        exterior_json: spec.exterior_json.clone(),
        interior_json: spec.interior_json.clone(),
        remake_meta_json: "{}".to_string(),
        updated_at: now,
    };

    diesel::insert_into(housing_estates)
        .values(&new_row)
        .execute(conn)?;

    Ok(new_row)
}

pub fn list_all_estates(conn: &mut SqliteConnection) -> QueryResult<Vec<HousingEstateRow>> {
    use schema::housing_estates::dsl::*;
    housing_estates
        .select(HousingEstateRow::as_select())
        .order((
            territory_type_id.asc(),
            ward_index.asc(),
            division.asc(),
            plot_index.asc(),
            room_number.asc(),
        ))
        .load::<HousingEstateRow>(conn)
}

pub fn estate_by_land_ident(
    conn: &mut SqliteConnection,
    ident: i32,
) -> QueryResult<Option<HousingEstateRow>> {
    use schema::housing_estates::dsl::*;
    housing_estates
        .filter(land_ident.eq(ident))
        .select(HousingEstateRow::as_select())
        .first::<HousingEstateRow>(conn)
        .optional()
}

pub fn estate_by_house_id(
    conn: &mut SqliteConnection,
    hid: HouseId,
) -> QueryResult<Option<HousingEstateRow>> {
    use schema::housing_estates::dsl::*;
    let raw = u64::from(hid) as i64;
    housing_estates
        .filter(house_id.eq(raw))
        .select(HousingEstateRow::as_select())
        .first::<HousingEstateRow>(conn)
        .optional()
}

pub fn estate_by_location(
    conn: &mut SqliteConnection,
    zone_id: u16,
    world: u16,
    ward: u8,
    div: u8,
    plot: u8,
) -> QueryResult<Option<HousingEstateRow>> {
    use schema::housing_estates::dsl::*;
    housing_estates
        .filter(territory_type_id.eq(zone_id as i32))
        .filter(world_id.eq(world as i32))
        .filter(ward_index.eq(ward as i32))
        .filter(division.eq((div & 1) as i32))
        .filter(plot_index.eq((plot % 30) as i32))
        .filter(is_apartment.eq(false))
        .select(HousingEstateRow::as_select())
        .first::<HousingEstateRow>(conn)
        .optional()
}

pub fn estates_in_ward(
    conn: &mut SqliteConnection,
    zone_id: u16,
    world: u16,
    ward: u8,
) -> QueryResult<Vec<HousingEstateRow>> {
    use schema::housing_estates::dsl::*;
    housing_estates
        .filter(territory_type_id.eq(zone_id as i32))
        .filter(world_id.eq(world as i32))
        .filter(ward_index.eq(ward as i32))
        .filter(is_apartment.eq(false))
        .select(HousingEstateRow::as_select())
        .order((division.asc(), plot_index.asc()))
        .load::<HousingEstateRow>(conn)
}

pub fn apartments_in_ward(
    conn: &mut SqliteConnection,
    zone_id: u16,
    world: u16,
    ward: u8,
    div: u8,
) -> QueryResult<Vec<HousingEstateRow>> {
    use schema::housing_estates::dsl::*;
    housing_estates
        .filter(territory_type_id.eq(zone_id as i32))
        .filter(world_id.eq(world as i32))
        .filter(ward_index.eq(ward as i32))
        .filter(division.eq((div & 1) as i32))
        .filter(is_apartment.eq(true))
        .select(HousingEstateRow::as_select())
        .order(room_number.asc())
        .load::<HousingEstateRow>(conn)
}

pub fn owned_estates_for_character(
    conn: &mut SqliteConnection,
    cid: u64,
) -> QueryResult<Vec<HousingEstateRow>> {
    use schema::housing_estates::dsl::*;
    housing_estates
        .filter(owner_content_id.eq(cid as i64))
        .select(HousingEstateRow::as_select())
        .order(updated_at.desc())
        .load::<HousingEstateRow>(conn)
}

pub fn save_estate(conn: &mut SqliteConnection, row: &HousingEstateRow) -> QueryResult<()> {
    use schema::housing_estates::dsl::*;
    let mut updated = row.clone();
    updated.estate_name = clamp_utf8(&updated.estate_name, MAX_ESTATE_NAME_BYTES);
    updated.greeting = clamp_utf8(&updated.greeting, MAX_ESTATE_GREETING_BYTES);
    updated.owner_name = clamp_utf8(&updated.owner_name, MAX_OWNER_NAME_BYTES);
    updated.updated_at = timestamp_secs() as i64;
    diesel::update(housing_estates.filter(land_ident.eq(updated.land_ident)))
        .set(&updated)
        .execute(conn)?;
    Ok(())
}

pub fn list_furniture_for_estate(
    conn: &mut SqliteConnection,
    ident: i32,
    placed_only: Option<bool>,
) -> QueryResult<Vec<HousingFurnitureRow>> {
    use schema::housing_furniture::dsl::*;
    let mut query = housing_furniture.filter(land_ident.eq(ident)).into_boxed();
    if let Some(placed_flag) = placed_only {
        query = query.filter(placed.eq(placed_flag));
    }
    query
        .select(HousingFurnitureRow::as_select())
        .order((container_type.asc(), slot.asc()))
        .load::<HousingFurnitureRow>(conn)
}

pub fn list_placed_furniture_in_scope(
    conn: &mut SqliteConnection,
    ident: i32,
    indoors: bool,
) -> QueryResult<Vec<HousingFurnitureRow>> {
    use schema::housing_furniture::dsl::*;
    let containers = placed_container_ids_for_scope(indoors);
    housing_furniture
        .filter(land_ident.eq(ident))
        .filter(placed.eq(true))
        .filter(container_type.eq_any(containers))
        .select(HousingFurnitureRow::as_select())
        .order((container_type.asc(), slot.asc()))
        .load::<HousingFurnitureRow>(conn)
}

pub fn upsert_furniture(conn: &mut SqliteConnection, row: &HousingFurnitureRow) -> QueryResult<()> {
    use schema::housing_furniture::dsl::*;
    let mut row = row.clone();
    row.updated_at = timestamp_secs() as i64;
    diesel::replace_into(housing_furniture)
        .values(&row)
        .execute(conn)?;
    Ok(())
}

pub fn update_furniture_position(
    conn: &mut SqliteConnection,
    ident: i32,
    container_id: i32,
    slot_idx: i32,
    x: f32,
    y: f32,
    z: f32,
    rot: f32,
) -> QueryResult<usize> {
    use schema::housing_furniture::dsl::*;
    let now = timestamp_secs() as i64;
    // Clear raw_rotation_quat and parent_slot if present in remake_meta_json on manual move.
    let existing = housing_furniture
        .filter(land_ident.eq(ident))
        .filter(container_type.eq(container_id))
        .filter(slot.eq(slot_idx))
        .select(HousingFurnitureRow::as_select())
        .first::<HousingFurnitureRow>(conn)
        .optional()?;

    let updated_meta = existing
        .as_ref()
        .and_then(|r| serde_json::from_str::<FurnitureRemakeMeta>(&r.remake_meta_json).ok())
        .map(|mut meta| {
            meta.raw_rotation_quat = None;
            meta.parent_slot = None;
            serde_json::to_string(&meta).unwrap_or_else(|_| "{}".to_string())
        })
        .unwrap_or_else(|| "{}".to_string());

    diesel::update(
        housing_furniture
            .filter(land_ident.eq(ident))
            .filter(container_type.eq(container_id))
            .filter(slot.eq(slot_idx)),
    )
    .set((
        pos_x.eq(x),
        pos_y.eq(y),
        pos_z.eq(z),
        rotation.eq(rot),
        remake_meta_json.eq(updated_meta),
        updated_at.eq(now),
    ))
    .execute(conn)
}

pub fn update_furniture_stain(
    conn: &mut SqliteConnection,
    ident: i32,
    container_id: i32,
    slot_idx: i32,
    new_stain: u8,
) -> QueryResult<usize> {
    use schema::housing_furniture::dsl::*;
    let now = timestamp_secs() as i64;
    let existing = housing_furniture
        .filter(land_ident.eq(ident))
        .filter(container_type.eq(container_id))
        .filter(slot.eq(slot_idx))
        .select(HousingFurnitureRow::as_select())
        .first::<HousingFurnitureRow>(conn)
        .optional()?;

    let updated_meta = existing
        .as_ref()
        .and_then(|r| serde_json::from_str::<FurnitureRemakeMeta>(&r.remake_meta_json).ok())
        .map(|mut meta| {
            meta.raw_color_hex = None;
            serde_json::to_string(&meta).unwrap_or_else(|_| "{}".to_string())
        })
        .unwrap_or_else(|| "{}".to_string());

    diesel::update(
        housing_furniture
            .filter(land_ident.eq(ident))
            .filter(container_type.eq(container_id))
            .filter(slot.eq(slot_idx)),
    )
    .set((
        stain.eq(new_stain as i32),
        remake_meta_json.eq(updated_meta),
        updated_at.eq(now),
    ))
    .execute(conn)
}

pub fn update_furniture_object_data(
    conn: &mut SqliteConnection,
    ident: i32,
    container_id: i32,
    slot_idx: i32,
    material_key: Option<u16>,
    obj_data: &FurnitureObjectData,
) -> QueryResult<usize> {
    use schema::housing_furniture::dsl::*;
    let now = timestamp_secs() as i64;
    let encoded = serde_json::to_string(obj_data).unwrap_or_else(|_| "{}".to_string());
    let filter = housing_furniture
        .filter(land_ident.eq(ident))
        .filter(container_type.eq(container_id))
        .filter(slot.eq(slot_idx));
    if let Some(mkey) = material_key {
        diesel::update(filter)
            .set((
                material_item_key.eq(mkey as i32),
                object_data_json.eq(encoded),
                updated_at.eq(now),
            ))
            .execute(conn)
    } else {
        diesel::update(filter)
            .set((object_data_json.eq(encoded), updated_at.eq(now)))
            .execute(conn)
    }
}

pub fn delete_furniture_slot(
    conn: &mut SqliteConnection,
    ident: i32,
    container_id: i32,
    slot_idx: i32,
) -> QueryResult<usize> {
    use schema::housing_furniture::dsl::*;
    diesel::delete(
        housing_furniture
            .filter(land_ident.eq(ident))
            .filter(container_type.eq(container_id))
            .filter(slot.eq(slot_idx)),
    )
    .execute(conn)
}

pub fn move_furniture_row(
    conn: &mut SqliteConnection,
    ident: i32,
    src_container: i32,
    src_slot: i32,
    dst_container: i32,
    dst_slot: i32,
    dst_placed: bool,
) -> QueryResult<Option<HousingFurnitureRow>> {
    use schema::housing_furniture::dsl::*;
    conn.transaction(|conn| {
        let Some(mut row) = housing_furniture
            .filter(land_ident.eq(ident))
            .filter(container_type.eq(src_container))
            .filter(slot.eq(src_slot))
            .select(HousingFurnitureRow::as_select())
            .first::<HousingFurnitureRow>(conn)
            .optional()?
        else {
            return Ok(None);
        };

        diesel::delete(
            housing_furniture
                .filter(land_ident.eq(ident))
                .filter(container_type.eq(src_container))
                .filter(slot.eq(src_slot)),
        )
        .execute(conn)?;

        row.container_type = dst_container;
        row.slot = dst_slot;
        row.placed = dst_placed;
        row.updated_at = timestamp_secs() as i64;
        diesel::replace_into(housing_furniture)
            .values(&row)
            .execute(conn)?;
        Ok(Some(row))
    })
}

/// Replaces placed furniture in either indoor scope, outdoor scope, or both while preserving storeroom items.
pub fn replace_placed_furniture_for_scope(
    conn: &mut SqliteConnection,
    ident: i32,
    replace_indoor: bool,
    replace_outdoor: bool,
    new_rows: &[HousingFurnitureRow],
) -> QueryResult<()> {
    use schema::housing_furniture::dsl::*;
    let mut target_containers = Vec::new();
    if replace_indoor {
        target_containers.extend(placed_container_ids_for_scope(true));
    }
    if replace_outdoor {
        target_containers.extend(placed_container_ids_for_scope(false));
    }
    if target_containers.is_empty() && new_rows.is_empty() {
        return Ok(());
    }

    conn.transaction(|conn| {
        if !target_containers.is_empty() {
            diesel::delete(
                housing_furniture
                    .filter(land_ident.eq(ident))
                    .filter(container_type.eq_any(&target_containers)),
            )
            .execute(conn)?;
        }
        if !new_rows.is_empty() {
            diesel::replace_into(housing_furniture)
                .values(new_rows)
                .execute(conn)?;
        }
        Ok(())
    })
}

pub fn clear_furniture_for_estate(
    conn: &mut SqliteConnection,
    ident: i32,
    include_storeroom: bool,
) -> QueryResult<usize> {
    use schema::housing_furniture::dsl::*;
    if include_storeroom {
        diesel::delete(housing_furniture.filter(land_ident.eq(ident))).execute(conn)
    } else {
        diesel::delete(
            housing_furniture
                .filter(land_ident.eq(ident))
                .filter(placed.eq(true)),
        )
        .execute(conn)
    }
}

/// Atomically resets an estate's appearance JSON and deletes placed furnishings for `clear_indoor`/`clear_outdoor` in one transaction.
pub fn reset_estate_scope(
    conn: &mut SqliteConnection,
    updated_estate: &HousingEstateRow,
    clear_indoor: bool,
    clear_outdoor: bool,
) -> QueryResult<usize> {
    use schema::housing_furniture::dsl::*;
    let mut target_containers = Vec::new();
    if clear_indoor {
        target_containers.extend(placed_container_ids_for_scope(true));
    }
    if clear_outdoor {
        target_containers.extend(placed_container_ids_for_scope(false));
    }
    conn.transaction(|conn| {
        save_estate(conn, updated_estate)?;
        if target_containers.is_empty() {
            return Ok(0);
        }
        diesel::delete(
            housing_furniture
                .filter(land_ident.eq(updated_estate.land_ident))
                .filter(placed.eq(true))
                .filter(container_type.eq_any(&target_containers)),
        )
        .execute(conn)
    })
}

pub fn delete_estate_and_furniture(conn: &mut SqliteConnection, ident: i32) -> QueryResult<usize> {
    conn.transaction(|conn| {
        diesel::delete(
            schema::housing_furniture::dsl::housing_furniture
                .filter(schema::housing_furniture::dsl::land_ident.eq(ident)),
        )
        .execute(conn)?;
        diesel::delete(
            schema::housing_snapshots::dsl::housing_snapshots
                .filter(schema::housing_snapshots::dsl::land_ident.eq(ident)),
        )
        .execute(conn)?;
        diesel::delete(
            schema::housing_estates::dsl::housing_estates
                .filter(schema::housing_estates::dsl::land_ident.eq(ident)),
        )
        .execute(conn)
    })
}

/// Captures a full snapshot of `estate` and its furniture rows, pruning oldest snapshots beyond `MAX_SNAPSHOTS_PER_ESTATE`.
pub fn create_snapshot(
    conn: &mut SqliteConnection,
    ident: i32,
    snapshot_label: &str,
) -> QueryResult<i32> {
    use schema::housing_snapshots::dsl::*;

    conn.transaction(|conn| {
        let Some(estate) = estate_by_land_ident(conn, ident)? else {
            return Err(diesel::result::Error::NotFound);
        };
        let furniture = list_furniture_for_estate(conn, ident, None)?;

        let est_json = serde_json::to_string(&estate).unwrap_or_else(|_| "{}".to_string());
        let furn_json = serde_json::to_string(&furniture).unwrap_or_else(|_| "[]".to_string());
        let now = timestamp_secs() as i64;

        let new_snap = NewHousingSnapshotRow {
            land_ident: ident,
            label: snapshot_label,
            estate_json: &est_json,
            furniture_json: &furn_json,
            created_at: now,
        };
        diesel::insert_into(housing_snapshots)
            .values(&new_snap)
            .execute(conn)?;

        let inserted_id: i32 = housing_snapshots
            .filter(land_ident.eq(ident))
            .select(id)
            .order(id.desc())
            .first(conn)?;

        let all_ids: Vec<i32> = housing_snapshots
            .filter(land_ident.eq(ident))
            .select(id)
            .order(id.desc())
            .load(conn)?;

        if all_ids.len() > MAX_SNAPSHOTS_PER_ESTATE {
            let stale = &all_ids[MAX_SNAPSHOTS_PER_ESTATE..];
            diesel::delete(housing_snapshots.filter(id.eq_any(stale))).execute(conn)?;
        }

        Ok(inserted_id)
    })
}

pub fn list_snapshots(
    conn: &mut SqliteConnection,
    ident: i32,
) -> QueryResult<Vec<SnapshotSummary>> {
    use schema::housing_snapshots::dsl::*;
    let rows: Vec<(i32, i32, String, i64)> = housing_snapshots
        .filter(land_ident.eq(ident))
        .select((id, land_ident, label, created_at))
        .order(id.desc())
        .load(conn)?;
    Ok(rows
        .into_iter()
        .map(|(snap_id, land, lbl, ts)| SnapshotSummary {
            id: snap_id,
            land_ident: land,
            label: lbl,
            created_at: ts,
        })
        .collect())
}

pub fn restore_snapshot(
    conn: &mut SqliteConnection,
    ident: i32,
    snapshot_id: i32,
) -> QueryResult<HousingEstateRow> {
    use schema::housing_furniture::dsl as f_dsl;
    use schema::housing_snapshots::dsl::*;

    conn.transaction(|conn| {
        let snap: HousingSnapshotRow = housing_snapshots
            .filter(land_ident.eq(ident))
            .filter(id.eq(snapshot_id))
            .select(HousingSnapshotRow::as_select())
            .first(conn)?;

        let mut restored_estate: HousingEstateRow = serde_json::from_str(&snap.estate_json)
            .map_err(|e| diesel::result::Error::DeserializationError(Box::new(e)))?;
        restored_estate.land_ident = ident;
        restored_estate.updated_at = timestamp_secs() as i64;

        let mut restored_furniture: Vec<HousingFurnitureRow> =
            serde_json::from_str(&snap.furniture_json)
                .map_err(|e| diesel::result::Error::DeserializationError(Box::new(e)))?;
        for item in &mut restored_furniture {
            item.land_ident = ident;
        }

        save_estate(conn, &restored_estate)?;
        diesel::delete(f_dsl::housing_furniture.filter(f_dsl::land_ident.eq(ident)))
            .execute(conn)?;
        if !restored_furniture.is_empty() {
            diesel::replace_into(f_dsl::housing_furniture)
                .values(&restored_furniture)
                .execute(conn)?;
        }

        Ok(restored_estate)
    })
}

pub fn restore_latest_snapshot(
    conn: &mut SqliteConnection,
    ident: i32,
) -> QueryResult<(i32, HousingEstateRow)> {
    use schema::housing_snapshots::dsl::*;
    conn.transaction(|conn| {
        let latest_id: i32 = housing_snapshots
            .filter(land_ident.eq(ident))
            .select(id)
            .order(id.desc())
            .first(conn)?;
        let restored = restore_snapshot(conn, ident, latest_id)?;
        diesel::delete(housing_snapshots.filter(id.eq(latest_id))).execute(conn)?;
        Ok((latest_id, restored))
    })
}

pub fn plot_state_for_row(row: &HousingEstateRow, exterior: &HouseExterior) -> PlotState {
    if row.is_apartment {
        PlotState::OwnedEstate
    } else if exterior.is_under_construction() {
        PlotState::OwnedLand
    } else {
        PlotState::OwnedEstate
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldDatabase;

    #[test]
    fn zz_housing_migration_crud_and_snapshot_ring_buffer() {
        let mut world_db = WorldDatabase::new_in_memory();
        let conn = world_db.connection_mut();

        let spec = EnsureEstateSpec {
            territory_type_id: 340,
            world_id: 79,
            ward_index: 0,
            division: 0,
            plot_index: 5,
            room_number: 0,
            is_apartment: false,
            plot_size: PlotSize::Large,
            is_fc: false,
            owner_content_id: 1001,
            owner_name: "Tester".to_string(),
            estate_name: "Foundation Manor".to_string(),
            exterior_json: "{}".to_string(),
            interior_json: "{}".to_string(),
        };

        let estate = ensure_estate(conn, &spec).expect("ensure_estate should succeed");
        let ident = estate.land_ident;

        let item = HousingFurnitureRow {
            land_ident: ident,
            container_type: 25003,
            slot: 0,
            item_id: 28901,
            catalog_id: 123,
            material_item_key: 2010,
            stain: 42,
            placed: true,
            pos_x: 1.5,
            pos_y: 0.0,
            pos_z: -2.25,
            rotation: 0.75,
            object_data_json: "{}".to_string(),
            remake_meta_json: "{}".to_string(),
            created_by_content_id: 1001,
            updated_at: 1,
        };
        upsert_furniture(conn, &item).expect("upsert furniture");

        // Push 27 snapshots to verify the ring buffer caps at MAX_SNAPSHOTS_PER_ESTATE (25).
        for idx in 0..27 {
            create_snapshot(conn, ident, &format!("snap-{idx}")).expect("create snapshot");
        }
        let snaps = list_snapshots(conn, ident).expect("list snapshots");
        assert_eq!(snaps.len(), MAX_SNAPSHOTS_PER_ESTATE);
        assert_eq!(snaps[0].label, "snap-26");

        // Mutate and undo back to snap-26.
        delete_furniture_slot(conn, ident, 25003, 0).expect("delete slot");
        assert!(
            list_placed_furniture_in_scope(conn, ident, true)
                .unwrap()
                .is_empty()
        );
        let (_, _restored) = restore_latest_snapshot(conn, ident).expect("undo latest");
        let after = list_placed_furniture_in_scope(conn, ident, true).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].material_item_key, 2010);
    }
}
