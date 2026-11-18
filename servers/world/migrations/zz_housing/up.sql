CREATE TABLE IF NOT EXISTS housing_estates (
    land_ident INTEGER PRIMARY KEY NOT NULL,
    house_id BIGINT NOT NULL UNIQUE,
    territory_type_id INTEGER NOT NULL,
    world_id INTEGER NOT NULL,
    ward_index INTEGER NOT NULL,
    division INTEGER NOT NULL,
    plot_index INTEGER NOT NULL,
    room_number INTEGER NOT NULL DEFAULT 0,
    is_apartment BOOLEAN NOT NULL DEFAULT 0,
    plot_size INTEGER NOT NULL DEFAULT 0,
    owner_type INTEGER NOT NULL DEFAULT 0,
    owner_content_id BIGINT NOT NULL DEFAULT 0,
    owner_name TEXT NOT NULL DEFAULT '',
    fc_id BIGINT NOT NULL DEFAULT 0,
    estate_name TEXT NOT NULL DEFAULT '',
    greeting TEXT NOT NULL DEFAULT '',
    flags INTEGER NOT NULL DEFAULT 0,
    price INTEGER NOT NULL DEFAULT 0,
    appeal_tag_0 INTEGER NOT NULL DEFAULT 0,
    appeal_tag_1 INTEGER NOT NULL DEFAULT 0,
    appeal_tag_2 INTEGER NOT NULL DEFAULT 0,
    light_level INTEGER NOT NULL DEFAULT 0,
    enable_ssao BOOLEAN NOT NULL DEFAULT 1,
    exterior_json TEXT NOT NULL DEFAULT '{}',
    interior_json TEXT NOT NULL DEFAULT '{}',
    remake_meta_json TEXT NOT NULL DEFAULT '{}',
    updated_at BIGINT NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_housing_estates_ward
    ON housing_estates (territory_type_id, world_id, ward_index, division, is_apartment);

CREATE INDEX IF NOT EXISTS idx_housing_estates_owner
    ON housing_estates (owner_content_id);

CREATE TABLE IF NOT EXISTS housing_furniture (
    land_ident INTEGER NOT NULL,
    container_type INTEGER NOT NULL,
    slot INTEGER NOT NULL,
    item_id INTEGER NOT NULL,
    catalog_id INTEGER NOT NULL,
    material_item_key INTEGER NOT NULL DEFAULT 0,
    stain INTEGER NOT NULL DEFAULT 0,
    placed BOOLEAN NOT NULL DEFAULT 0,
    pos_x REAL NOT NULL DEFAULT 0.0,
    pos_y REAL NOT NULL DEFAULT 0.0,
    pos_z REAL NOT NULL DEFAULT 0.0,
    rotation REAL NOT NULL DEFAULT 0.0,
    object_data_json TEXT NOT NULL DEFAULT '{}',
    remake_meta_json TEXT NOT NULL DEFAULT '{}',
    created_by_content_id BIGINT NOT NULL DEFAULT 0,
    updated_at BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (land_ident, container_type, slot)
);

CREATE INDEX IF NOT EXISTS idx_housing_furniture_land_placed
    ON housing_furniture (land_ident, placed);

CREATE TABLE IF NOT EXISTS housing_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
    land_ident INTEGER NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    estate_json TEXT NOT NULL,
    furniture_json TEXT NOT NULL,
    created_at BIGINT NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_housing_snapshots_land
    ON housing_snapshots (land_ident, id);
