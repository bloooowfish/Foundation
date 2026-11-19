use std::collections::{BTreeMap, HashMap};
use std::f32::consts::{PI, TAU};

use kawari::common::ContainerType;

use super::containers::{
    container_from_i32, flat_slot_for_placed_container, is_interior_placed_container,
};
use super::db::{HousingEstateRow, HousingFurnitureRow};

/// Maximum `removed + added` count automatically pushed with zero loading screen under `ReloadMode::Auto`.
pub const LIVE_DIFF_AUTO_MAX_STRUCTURAL: usize = 120;
/// Maximum total delta operations automatically pushed with zero loading screen under `ReloadMode::Auto`.
pub const LIVE_DIFF_AUTO_MAX_TOTAL_OPS: usize = 250;

const POS_EPSILON: f32 = 1e-3;
const ROT_EPSILON: f32 = 1e-3;

/// Client-synced snapshot of a single indoor placed furnishing slot.
#[derive(Clone, Debug, PartialEq)]
pub struct SyncedIndoorFurnitureSlot {
    pub flat_slot: u16,
    pub container_type: ContainerType,
    pub slot_in_page: u16,
    pub item_id: u32,
    pub catalog_id: u16,
    pub stain: u8,
    pub material_item_key: u16,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
    pub rotation: f32,
    pub object_data_json: String,
}

/// Mirror of the indoor state currently loaded in a connected client's memory.
#[derive(Clone, Debug, PartialEq)]
pub struct SyncedIndoorState {
    pub land_ident: i32,
    pub territory_type_id: u16,
    pub interior_json: String,
    pub light_level: i32,
    pub enable_ssao: bool,
    pub slots: BTreeMap<u16, SyncedIndoorFurnitureSlot>,
}

impl SyncedIndoorState {
    /// Captures a comparable client-state snapshot from DB estate and placed indoor furniture rows.
    pub fn from_db(
        estate: &HousingEstateRow,
        territory_type_id: u16,
        rows: &[HousingFurnitureRow],
    ) -> Self {
        let mut slots = BTreeMap::new();
        for row in rows {
            if !row.placed {
                continue;
            }
            let Some(container_type) = container_from_i32(row.container_type) else {
                continue;
            };
            if !is_interior_placed_container(container_type) {
                continue;
            }
            let Some(flat_slot) = flat_slot_for_placed_container(container_type, row.slot as u16)
            else {
                continue;
            };
            slots.insert(
                flat_slot,
                SyncedIndoorFurnitureSlot {
                    flat_slot,
                    container_type,
                    slot_in_page: row.slot.max(0) as u16,
                    item_id: row.item_id.max(0) as u32,
                    catalog_id: row.catalog_id.max(0) as u16,
                    stain: row.stain.clamp(0, u8::MAX as i32) as u8,
                    material_item_key: row.material_item_key.max(0) as u16,
                    pos_x: row.pos_x,
                    pos_y: row.pos_y,
                    pos_z: row.pos_z,
                    rotation: row.rotation,
                    object_data_json: row.object_data_json.clone(),
                },
            );
        }

        Self {
            land_ident: estate.land_ident,
            territory_type_id,
            interior_json: estate.interior_json.clone(),
            light_level: estate.light_level,
            enable_ssao: estate.enable_ssao,
            slots,
        }
    }
}

/// Candidate incoming indoor furnishing used to match against existing flat slots.
#[derive(Clone, Copy, Debug)]
pub struct SlotMatchCandidate {
    pub catalog_id: u16,
    pub item_id: u32,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
    pub rotation: f32,
    pub stain: u8,
    pub material_item_key: u16,
}

/// Minimal live-packet delta between two `SyncedIndoorState` snapshots within the same indoor shell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IndoorLiveDiff {
    pub fixtures_or_lighting_changed: bool,
    pub removed: Vec<SyncedIndoorFurnitureSlot>,
    pub added: Vec<SyncedIndoorFurnitureSlot>,
    pub moved: Vec<SyncedIndoorFurnitureSlot>,
    pub stained: Vec<SyncedIndoorFurnitureSlot>,
    pub material_changed: Vec<SyncedIndoorFurnitureSlot>,
    pub object_data_changed: Vec<SyncedIndoorFurnitureSlot>,
}

impl IndoorLiveDiff {
    pub fn is_empty(&self) -> bool {
        !self.fixtures_or_lighting_changed
            && self.removed.is_empty()
            && self.added.is_empty()
            && self.moved.is_empty()
            && self.stained.is_empty()
            && self.material_changed.is_empty()
            && self.object_data_changed.is_empty()
    }

    pub fn structural_change_count(&self) -> usize {
        self.removed.len() + self.added.len()
    }

    pub fn total_furniture_ops(&self) -> usize {
        self.removed.len()
            + self.added.len()
            + self.moved.len()
            + self.stained.len()
            + self.material_changed.len()
            + self.object_data_changed.len()
    }

    pub fn should_use_live_packets_in_auto(&self) -> bool {
        self.structural_change_count() <= LIVE_DIFF_AUTO_MAX_STRUCTURAL
            && self.total_furniture_ops() <= LIVE_DIFF_AUTO_MAX_TOTAL_OPS
    }

    pub fn needs_overlay_sync(&self) -> bool {
        !self.removed.is_empty() || !self.added.is_empty() || !self.moved.is_empty()
    }

    pub fn summary_ko(&self) -> String {
        let mut parts = Vec::new();
        if !self.moved.is_empty() {
            parts.push(format!("이동 {}", self.moved.len()));
        }
        if !self.added.is_empty() {
            parts.push(format!("추가 {}", self.added.len()));
        }
        if !self.removed.is_empty() {
            parts.push(format!("제거 {}", self.removed.len()));
        }
        if !self.stained.is_empty() {
            parts.push(format!("염색 {}", self.stained.len()));
        }
        if !self.material_changed.is_empty() {
            parts.push(format!("재질 {}", self.material_changed.len()));
        }
        if !self.object_data_changed.is_empty() {
            parts.push(format!("세부데이터 {}", self.object_data_changed.len()));
        }
        if self.fixtures_or_lighting_changed {
            parts.push("내장재·조명".to_string());
        }
        if parts.is_empty() {
            "변경 없음".to_string()
        } else {
            parts.join(" · ")
        }
    }
}

fn minimal_angle_diff(a: f32, b: f32) -> f32 {
    ((a - b + PI).rem_euclid(TAU) - PI).abs()
}

fn euclidean_distance_sq(ax: f32, ay: f32, az: f32, bx: f32, by: f32, bz: f32) -> f32 {
    let dx = ax - bx;
    let dy = ay - by;
    let dz = az - bz;
    dx * dx + dy * dy + dz * dz
}

fn positions_close(ax: f32, ay: f32, az: f32, bx: f32, by: f32, bz: f32) -> bool {
    euclidean_distance_sq(ax, ay, az, bx, by, bz) <= POS_EPSILON * POS_EPSILON
}

/// Computes minimal slot-level delta packets between `prev` and `next` states.
pub fn compute_indoor_live_diff(
    prev: &SyncedIndoorState,
    next: &SyncedIndoorState,
) -> IndoorLiveDiff {
    let mut diff = IndoorLiveDiff {
        fixtures_or_lighting_changed: prev.interior_json != next.interior_json
            || prev.light_level != next.light_level
            || prev.enable_ssao != next.enable_ssao,
        ..Default::default()
    };

    for (flat_slot, old) in &prev.slots {
        match next.slots.get(flat_slot) {
            None => diff.removed.push(old.clone()),
            Some(new) if old.catalog_id != new.catalog_id || old.item_id != new.item_id => {
                diff.removed.push(old.clone());
                diff.added.push(new.clone());
            }
            Some(new) => {
                if !positions_close(
                    old.pos_x, old.pos_y, old.pos_z, new.pos_x, new.pos_y, new.pos_z,
                ) || minimal_angle_diff(old.rotation, new.rotation) > ROT_EPSILON
                {
                    diff.moved.push(new.clone());
                }
                if old.stain != new.stain {
                    diff.stained.push(new.clone());
                }
                if old.material_item_key != new.material_item_key {
                    diff.material_changed.push(new.clone());
                }
                if old.object_data_json != new.object_data_json {
                    diff.object_data_changed.push(new.clone());
                }
            }
        }
    }

    for (flat_slot, new) in &next.slots {
        if !prev.slots.contains_key(flat_slot) {
            diff.added.push(new.clone());
        }
    }

    diff
}

struct ExistingSlotInfo {
    flat_slot: u16,
    item_id: u32,
    catalog_id: u16,
    stain: u8,
    material_item_key: u16,
    pos_x: f32,
    pos_y: f32,
    pos_z: f32,
    rotation: f32,
}

/// Assigns flat indoor slots (`0..capacity`) to `incoming` furnishings while keeping matching existing items in their prior slots.
pub fn assign_indoor_slots_preserving_existing(
    existing_rows: &[HousingFurnitureRow],
    incoming: &[SlotMatchCandidate],
    capacity: usize,
) -> Vec<Option<u16>> {
    let mut assigned = vec![None; incoming.len()];
    if capacity == 0 || incoming.is_empty() {
        return assigned;
    }

    let mut existing_by_slot: BTreeMap<u16, ExistingSlotInfo> = BTreeMap::new();
    for row in existing_rows {
        if !row.placed {
            continue;
        }
        let Some(ctype) = container_from_i32(row.container_type) else {
            continue;
        };
        if !is_interior_placed_container(ctype) {
            continue;
        }
        let Some(flat_slot) = flat_slot_for_placed_container(ctype, row.slot as u16) else {
            continue;
        };
        if (flat_slot as usize) >= capacity {
            continue;
        }
        existing_by_slot.insert(
            flat_slot,
            ExistingSlotInfo {
                flat_slot,
                item_id: row.item_id.max(0) as u32,
                catalog_id: row.catalog_id.max(0) as u16,
                stain: row.stain.clamp(0, u8::MAX as i32) as u8,
                material_item_key: row.material_item_key.max(0) as u16,
                pos_x: row.pos_x,
                pos_y: row.pos_y,
                pos_z: row.pos_z,
                rotation: row.rotation,
            },
        );
    }

    let existing: Vec<ExistingSlotInfo> = existing_by_slot.into_values().collect();
    let mut occupied_prev = vec![false; capacity];
    let mut by_key: HashMap<(u16, u32), Vec<usize>> = HashMap::new();
    for (idx, ex) in existing.iter().enumerate() {
        occupied_prev[ex.flat_slot as usize] = true;
        by_key
            .entry((ex.catalog_id, ex.item_id))
            .or_default()
            .push(idx);
    }

    let mut used_existing = vec![false; existing.len()];
    let mut used_slot = vec![false; capacity];

    // Pass 1: exact transform + stain + material matches keep their exact prior slot.
    for (i, cand) in incoming.iter().enumerate() {
        let Some(bucket) = by_key.get(&(cand.catalog_id, cand.item_id)) else {
            continue;
        };
        for &ei in bucket {
            if used_existing[ei] {
                continue;
            }
            let ex = &existing[ei];
            if used_slot[ex.flat_slot as usize] {
                continue;
            }
            if ex.stain == cand.stain
                && ex.material_item_key == cand.material_item_key
                && positions_close(
                    ex.pos_x, ex.pos_y, ex.pos_z, cand.pos_x, cand.pos_y, cand.pos_z,
                )
                && minimal_angle_diff(ex.rotation, cand.rotation) <= ROT_EPSILON
            {
                used_existing[ei] = true;
                used_slot[ex.flat_slot as usize] = true;
                assigned[i] = Some(ex.flat_slot);
                break;
            }
        }
    }

    // Pass 2: moved/rotated/dyed/re-textured items claim the nearest unused prior slot of the same item.
    for (i, cand) in incoming.iter().enumerate() {
        if assigned[i].is_some() {
            continue;
        }
        let Some(bucket) = by_key.get(&(cand.catalog_id, cand.item_id)) else {
            continue;
        };
        let mut best: Option<(usize, (f32, u8, u8, u16))> = None;
        for &ei in bucket {
            if used_existing[ei] {
                continue;
            }
            let ex = &existing[ei];
            if used_slot[ex.flat_slot as usize] {
                continue;
            }
            let score = (
                euclidean_distance_sq(
                    ex.pos_x, ex.pos_y, ex.pos_z, cand.pos_x, cand.pos_y, cand.pos_z,
                ),
                u8::from(ex.stain != cand.stain),
                u8::from(ex.material_item_key != cand.material_item_key),
                ex.flat_slot,
            );
            let better = match best {
                None => true,
                Some((_, best_score)) => {
                    if (score.0 - best_score.0).abs() <= 1e-6 {
                        (score.1, score.2, score.3) < (best_score.1, best_score.2, best_score.3)
                    } else {
                        score.0 < best_score.0
                    }
                }
            };
            if better {
                best = Some((ei, score));
            }
        }
        if let Some((ei, _)) = best {
            let slot = existing[ei].flat_slot;
            used_existing[ei] = true;
            used_slot[slot as usize] = true;
            assigned[i] = Some(slot);
        }
    }

    // Pass 3: newly added items take brand-new empty slots first, then freed slots when near capacity.
    let mut fresh_cursor = 0usize;
    let mut freed_cursor = 0usize;
    for slot_opt in &mut assigned {
        if slot_opt.is_some() {
            continue;
        }
        while fresh_cursor < capacity && (used_slot[fresh_cursor] || occupied_prev[fresh_cursor]) {
            fresh_cursor += 1;
        }
        if fresh_cursor < capacity {
            used_slot[fresh_cursor] = true;
            *slot_opt = Some(fresh_cursor as u16);
            fresh_cursor += 1;
            continue;
        }
        while freed_cursor < capacity && used_slot[freed_cursor] {
            freed_cursor += 1;
        }
        if freed_cursor < capacity {
            used_slot[freed_cursor] = true;
            *slot_opt = Some(freed_cursor as u16);
            freed_cursor += 1;
        }
    }

    assigned
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::housing_sim::containers::indoor_container_for_flat_slot;

    fn make_row(
        flat_slot: u16,
        catalog_id: u16,
        item_id: u32,
        pos: (f32, f32, f32),
        rot: f32,
        stain: u8,
        mat: u16,
    ) -> HousingFurnitureRow {
        let (ctype, slot) = indoor_container_for_flat_slot(flat_slot).unwrap();
        HousingFurnitureRow {
            land_ident: 34000001,
            container_type: ctype as i32,
            slot: slot as i32,
            item_id: item_id as i32,
            catalog_id: catalog_id as i32,
            material_item_key: mat as i32,
            stain: stain as i32,
            placed: true,
            pos_x: pos.0,
            pos_y: pos.1,
            pos_z: pos.2,
            rotation: rot,
            object_data_json: "{}".to_string(),
            remake_meta_json: "{}".to_string(),
            created_by_content_id: 1,
            updated_at: 0,
        }
    }

    fn make_cand(
        catalog_id: u16,
        item_id: u32,
        pos: (f32, f32, f32),
        rot: f32,
        stain: u8,
        mat: u16,
    ) -> SlotMatchCandidate {
        SlotMatchCandidate {
            catalog_id,
            item_id,
            pos_x: pos.0,
            pos_y: pos.1,
            pos_z: pos.2,
            rotation: rot,
            stain,
            material_item_key: mat,
        }
    }

    #[test]
    fn assign_indoor_slots_preserving_existing_keeps_unchanged_and_moved_slots_stable() {
        let existing = vec![
            make_row(0, 100, 1000, (1.0, 0.0, 1.0), 0.0, 5, 0),
            make_row(1, 200, 2000, (2.0, 0.0, 2.0), 0.5, 0, 10),
            make_row(2, 100, 1000, (5.0, 0.0, 5.0), 1.0, 0, 0),
        ];

        // Reverse order, delete slot 1, move slot 2 slightly, add a brand new item.
        let incoming = vec![
            make_cand(100, 1000, (5.4, 0.0, 5.2), 1.2, 0, 0), // should keep slot 2
            make_cand(300, 3000, (0.0, 1.0, 0.0), 0.0, 0, 0), // new item -> slot 3 (not freeing/colliding slot 1)
            make_cand(100, 1000, (1.0, 0.0, 1.0), 0.0, 5, 0), // exact -> keeps slot 0
        ];

        let slots = assign_indoor_slots_preserving_existing(&existing, &incoming, 10);
        assert_eq!(slots, vec![Some(2), Some(3), Some(0)]);
    }

    #[test]
    fn assign_indoor_slots_reuses_freed_slots_when_at_capacity() {
        let existing = vec![
            make_row(0, 100, 1000, (0.0, 0.0, 0.0), 0.0, 0, 0),
            make_row(1, 200, 2000, (1.0, 0.0, 0.0), 0.0, 0, 0),
            make_row(2, 300, 3000, (2.0, 0.0, 0.0), 0.0, 0, 0),
        ];
        // Slot 1 deleted, capacity is 3 (all 0..2 were previously occupied), so new item reuses slot 1.
        let incoming = vec![
            make_cand(100, 1000, (0.0, 0.0, 0.0), 0.0, 0, 0),
            make_cand(300, 3000, (2.0, 0.0, 0.0), 0.0, 0, 0),
            make_cand(400, 4000, (3.0, 0.0, 0.0), 0.0, 0, 0),
        ];

        let slots = assign_indoor_slots_preserving_existing(&existing, &incoming, 3);
        assert_eq!(slots, vec![Some(0), Some(2), Some(1)]);
    }

    #[test]
    fn compute_indoor_live_diff_classifies_all_delta_types_and_rotation_wraparound() {
        let prev_rows = vec![
            make_row(0, 100, 1000, (1.0, 0.0, 1.0), PI - 0.0002, 5, 0),
            make_row(1, 200, 2000, (2.0, 0.0, 2.0), 0.0, 0, 0),
            make_row(2, 300, 3000, (3.0, 0.0, 3.0), 0.0, 1, 10),
            make_row(3, 400, 4000, (4.0, 0.0, 4.0), 0.0, 0, 0),
        ];
        let mut next_rows = vec![
            // Slot 0: -PI + 0.0002 wraps cleanly within ROT_EPSILON of PI - 0.0002 -> NO move!
            make_row(0, 100, 1000, (1.0, 0.0, 1.0), -PI + 0.0002, 5, 0),
            // Slot 1 replaced with a completely different item -> 1 removal + 1 addition
            make_row(1, 201, 2001, (2.0, 0.0, 2.0), 0.0, 0, 0),
            // Slot 2: moved + dyed + material changed + object_data changed
            make_row(2, 300, 3000, (3.5, 0.0, 3.0), 0.5, 24, 99),
            // Slot 4: added
            make_row(4, 500, 5000, (5.0, 0.0, 5.0), 0.0, 0, 0),
        ];
        next_rows[2].object_data_json = r#"{"values":[1]}"#.to_string();

        let prev_est = HousingEstateRow {
            land_ident: 34000001,
            interior_json: "{\"a\":1}".to_string(),
            light_level: 2,
            enable_ssao: true,
            ..Default::default()
        };
        let next_est = HousingEstateRow {
            light_level: 4,
            ..prev_est.clone()
        };

        let prev = SyncedIndoorState::from_db(&prev_est, 343, &prev_rows);
        let next = SyncedIndoorState::from_db(&next_est, 343, &next_rows);
        let diff = compute_indoor_live_diff(&prev, &next);

        assert!(diff.fixtures_or_lighting_changed);
        assert_eq!(diff.removed.len(), 2); // slots 1 and 3
        assert_eq!(diff.added.len(), 2); // slots 1 (new item) and 4
        assert_eq!(diff.moved.len(), 1);
        assert_eq!(diff.moved[0].flat_slot, 2);
        assert_eq!(diff.stained.len(), 1);
        assert_eq!(diff.material_changed.len(), 1);
        assert_eq!(diff.object_data_changed.len(), 1);
        assert!(!diff.is_empty());
        assert!(diff.needs_overlay_sync());
    }

    #[test]
    fn auto_live_diff_threshold_respects_structural_and_total_limits() {
        let mut diff = IndoorLiveDiff::default();
        assert!(diff.should_use_live_packets_in_auto());

        diff.moved = vec![
            SyncedIndoorState::from_db(
                &HousingEstateRow::default(),
                343,
                &[make_row(0, 1, 1, (0.0, 0.0, 0.0), 0.0, 0, 0)],
            )
            .slots
            .into_values()
            .next()
            .unwrap();
            250
        ];
        assert!(diff.should_use_live_packets_in_auto());

        diff.added = vec![diff.moved[0].clone()];
        assert!(!diff.should_use_live_packets_in_auto()); // total = 251 > 250

        diff.moved.clear();
        diff.added = vec![
            SyncedIndoorState::from_db(
                &HousingEstateRow::default(),
                343,
                &[make_row(0, 1, 1, (0.0, 0.0, 0.0), 0.0, 0, 0)],
            )
            .slots
            .into_values()
            .next()
            .unwrap();
            121
        ];
        assert!(!diff.should_use_live_packets_in_auto()); // structural = 121 > 120
    }
}
