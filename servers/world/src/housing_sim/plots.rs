#![allow(clippy::approx_constant)]
use glam::{Quat, Vec3, Vec3A};
use kawari::common::{Position, internal_housing_row};
use kawari::ipc::zone::PlotSize;

/// Represents the origin, orientation, size, and entrance style of a residential plot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlotLayoutSpec {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub rotation: f32,
    pub size: PlotSize,
    pub entrance_layout: &'static str,
}

impl PlotLayoutSpec {
    const fn new(
        x: f32,
        y: f32,
        z: f32,
        rotation: f32,
        size: PlotSize,
        entrance_layout: &'static str,
    ) -> Self {
        Self {
            x,
            y,
            z,
            rotation,
            size,
            entrance_layout,
        }
    }

    /// Returns the world origin vector of this plot.
    pub fn origin(&self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    /// Converts plot-local coordinates and rotation (ReMakePlace outdoor space) into FFXIV zone world space.
    pub fn local_to_world(&self, local_pos: Position, local_rotation: f32) -> (Position, f32) {
        let rotated = Quat::from_rotation_y(-self.rotation) * Vec3::from(local_pos.0);
        let world_pos = Position(Vec3A::from(rotated + self.origin()));
        let world_rot = wrap_angle_radians(local_rotation - self.rotation);
        (world_pos, world_rot)
    }

    /// Converts FFXIV zone world coordinates and rotation into plot-local coordinates (ReMakePlace outdoor space).
    pub fn world_to_local(&self, world_pos: Position, world_rotation: f32) -> (Position, f32) {
        let delta = Vec3::from(world_pos.0) - self.origin();
        let rotated = Quat::from_rotation_y(self.rotation) * delta;
        let local_rot = wrap_angle_radians(world_rotation + self.rotation);
        (Position(Vec3A::from(rotated)), local_rot)
    }
}

/// Normalizes an angle in radians into `[-PI, PI]`.
pub fn wrap_angle_radians(angle: f32) -> f32 {
    if !angle.is_finite() {
        return 0.0;
    }
    let wrapped =
        (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    if wrapped.is_finite() { wrapped } else { 0.0 }
}

/// Returns the plot layout metadata for `(territory_type_id, division, plot_index)`.
/// `division` is `0` (main) or `1` (subdivision), `plot_index` is `0..30`.
pub fn plot_spec(
    territory_type_id: u16,
    division: u8,
    plot_index: u8,
) -> Option<&'static PlotLayoutSpec> {
    let raw_idx = (division as usize) * 30 + (plot_index as usize);
    plot_spec_by_raw_index(territory_type_id, raw_idx as u8)
}

/// Returns the plot layout metadata for a raw 0-based `0..60` plot index in a housing ward.
pub fn plot_spec_by_raw_index(
    territory_type_id: u16,
    raw_plot_index: u8,
) -> Option<&'static PlotLayoutSpec> {
    let table = match territory_type_id {
        339 => &MIST_PLOTS,          // s1h1
        340 => &LAVENDER_BEDS_PLOTS, // f1h1
        341 => &GOBLET_PLOTS,        // w1h1
        641 => &SHIROGANE_PLOTS,     // e1h1
        979 => &EMPYREUM_PLOTS,      // r1h1
        _ => return None,
    };
    table.get(raw_plot_index as usize)
}

/// Finds the nearest `(division, plot_index)` in `territory_type_id` to a world position.
pub fn nearest_plot_to_position(territory_type_id: u16, world_pos: Position) -> Option<(u8, u8)> {
    let idx = (0u8..60).min_by(|&a, &b| {
        let da = plot_spec_by_raw_index(territory_type_id, a)
            .map(|p| Vec3::from(world_pos.0).distance_squared(p.origin()))
            .unwrap_or(f32::MAX);
        let db = plot_spec_by_raw_index(territory_type_id, b)
            .map(|p| Vec3::from(world_pos.0).distance_squared(p.origin()))
            .unwrap_or(f32::MAX);
        da.total_cmp(&db)
    })?;
    Some((idx / 30, idx % 30))
}

/// Nudges a front-door entrance position `distance` yalms outward along the entrance facing direction.
pub fn nudge_plot_entrance_exit(
    entrance_position: Position,
    entrance_rotation: f32,
    distance: f32,
) -> (Position, f32) {
    let angle = entrance_rotation + std::f32::consts::FRAC_PI_2;
    let nudged = Position(Vec3A::new(
        entrance_position.0.x - distance * angle.cos(),
        entrance_position.0.y,
        entrance_position.0.z + distance * angle.sin(),
    ));
    (nudged, wrap_angle_radians(entrance_rotation))
}

/// Returns the indoor entry `(position, rotation_radians)` right inside the front door,
/// facing inward (`PI` rad = `-Z`) for houses or `(0, 0, 0)` for apartments.
pub fn indoor_entry_transform(territory_type_id: u16) -> (Position, f32) {
    if is_apartment_territory(territory_type_id) {
        (Position(Vec3A::ZERO), std::f32::consts::PI)
    } else {
        (
            Position(Vec3A::new(-0.25, -0.39, 5.0)),
            std::f32::consts::PI,
        )
    }
}

/// Returns the city-state aetheryte ID corresponding to a housing district or indoor territory.
pub fn city_fallback_aetheryte_for_territory(zone_id: u16) -> u32 {
    match zone_id {
        339 | 282..=284 | 384 | 573 | 608 => 8,   // Limsa Lominsa
        340 | 342..=344 | 385 | 574 | 609 => 2,   // New Gridania
        341 | 345..=347 | 386 | 575 | 610 => 9,   // Ul'dah
        641 | 649..=651 | 652 | 654 | 655 => 111, // Kugane
        979 | 980..=982 | 983 | 985 | 999 => 70,  // Foundation
        _ => 2,
    }
}

/// Returns the outdoor residential district territory ID for an apartment/indoor territory when known.
pub fn default_district_for_zone(zone_id: u16) -> u16 {
    if internal_housing_row(zone_id).is_some() {
        return zone_id;
    }
    match zone_id {
        282..=284 | 384 | 573 | 608 => 339,
        342..=344 | 385 | 574 | 609 => 340,
        345..=347 | 386 | 575 | 610 => 341,
        649..=651 | 652 | 654 | 655 => 641,
        980..=982 | 983 | 985 | 999 => 979,
        _ => 340,
    }
}

/// Canonical outdoor residential ward territory IDs in sheet order.
pub const HOUSING_DISTRICT_TERRITORIES: [u16; 5] = [339, 340, 341, 641, 979];

/// Parses a housing district name, alias, or numeric territory ID into a canonical outdoor ward territory ID.
pub fn parse_district_territory(token: &str) -> Option<u16> {
    match token.trim().to_ascii_lowercase().as_str() {
        "mist" | "limsa" | "s1h1" | "339" => Some(339),
        "lavender" | "lavenderbeds" | "lavender_beds" | "lavender-beds" | "lb" | "gridania"
        | "f1h1" | "340" => Some(340),
        "goblet" | "uldah" | "ul'dah" | "w1h1" | "341" => Some(341),
        "shirogane" | "shiro" | "kugane" | "e1h1" | "641" => Some(641),
        "empyreum" | "ishgard" | "firmament" | "foundation" | "r1h1" | "979" => Some(979),
        other => match other {
            "안개빛" | "안개" | "림사" => Some(339),
            "라벤더" | "안식처" | "그리다니아" => Some(340),
            "하늘잔" | "울다하" => Some(341),
            "시로가네" | "쿠가네" => Some(641),
            "엠피레움" | "이슈가르드" | "창천" => Some(979),
            _ => None,
        },
    }
}

/// Returns a human-readable display name for an outdoor housing district territory ID.
pub const fn district_display_name(territory_type_id: u16) -> &'static str {
    match territory_type_id {
        339 => "Mist",
        340 => "Lavender Beds",
        341 => "The Goblet",
        641 => "Shirogane",
        979 => "Empyreum",
        _ => "Housing Ward",
    }
}

/// Returns the canonical 0-based `plot_index` (`0..30`) in `(territory_type_id, division)` whose native physical lot matches `size`.
pub fn canonical_plot_for_district_and_size(
    territory_type_id: u16,
    division: u8,
    size: PlotSize,
) -> Option<u8> {
    (0u8..30)
        .find(|&plot| plot_spec(territory_type_id, division, plot).is_some_and(|s| s.size == size))
}

/// Canonical indoor apartment territory ID for each housing district.
pub fn apartment_territory_for_district(district_territory_id: u16) -> Option<u16> {
    match district_territory_id {
        339 => Some(608), // Mist
        340 => Some(609), // Lavender Beds
        341 => Some(610), // Goblet
        641 => Some(655), // Shirogane
        979 => Some(999), // Empyreum
        _ => None,
    }
}

/// Canonical indoor apartment lobby territory ID for each housing district.
pub const fn apartment_lobby_territory_for_district(district_territory_id: u16) -> Option<u16> {
    match district_territory_id {
        339 => Some(573), // Topmast Lobby
        340 => Some(574), // Lily Hills Lobby
        341 => Some(575), // Sultana's Breath Lobby
        641 => Some(654), // Kobai Goten Lobby
        979 => Some(985), // Ingleside Lobby
        _ => None,
    }
}

/// Returns whether `territory_type_id` is an apartment lobby territory.
pub const fn is_apartment_lobby_territory(territory_type_id: u16) -> bool {
    matches!(territory_type_id, 573 | 574 | 575 | 654 | 985)
}

/// Returns whether `territory_type_id` is an apartment building room territory.
pub const fn is_apartment_territory(territory_type_id: u16) -> bool {
    matches!(territory_type_id, 608..=610 | 655 | 999)
}

/// Returns whether `territory_type_id` is a house interior territory (excluding apartments).
pub const fn is_house_interior_territory(territory_type_id: u16) -> bool {
    matches!(
        territory_type_id,
        282..=284 | 342..=347 | 649..=651 | 980..=982 | 1249..=1251 | 1374..=1376
    )
}

const S: PlotSize = PlotSize::Small;
const M: PlotSize = PlotSize::Medium;
const L: PlotSize = PlotSize::Large;

// Ported verbatim from ReMakePlacePlugin/Util/Plots.cs (all 5 residential districts x 60 plots).
const MIST_PLOTS: [PlotLayoutSpec; 60] = [
    PlotLayoutSpec::new(-72.0, 40.0, -108.0, -1.570451, M, "Center"),
    PlotLayoutSpec::new(-144.0, 36.0, -100.0, -0.7853982, L, "Center"),
    PlotLayoutSpec::new(-96.0, 32.0, -60.0, 3.141593, S, "Center"),
    PlotLayoutSpec::new(-92.0, 18.0, -12.0, -1.570451, M, "Right"),
    PlotLayoutSpec::new(-148.0, 20.0, 28.0, -2.094395, L, "Center"),
    PlotLayoutSpec::new(-36.0, 14.0, -4.0, 0.7853982, M, "Center"),
    PlotLayoutSpec::new(-40.0, 24.0, -56.0, 0.0, M, "Center"),
    PlotLayoutSpec::new(-12.0, 36.0, -92.0, 1.570451, S, "Right"),
    PlotLayoutSpec::new(40.0, 38.0, -100.0, 0.0, S, "Right Side"),
    PlotLayoutSpec::new(64.0, 42.0, -124.0, 0.0, S, "Right"),
    PlotLayoutSpec::new(4.0, 25.0, -56.0, 0.0, S, "Right Side"),
    PlotLayoutSpec::new(48.0, 26.0, -60.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(64.0, 18.0, -32.0, 0.0, S, "Right"),
    PlotLayoutSpec::new(83.628, 29.0, -63.0, -0.7853982, M, "Far Right"),
    PlotLayoutSpec::new(148.0, 46.0, -104.0, 0.7853982, L, "Center"),
    PlotLayoutSpec::new(136.0, 28.0, -34.0, 1.570451, S, "Left"),
    PlotLayoutSpec::new(112.0, 22.0, -20.0, 1.570451, S, "Left"),
    PlotLayoutSpec::new(72.0, 17.0, 4.0, 0.0, S, "Far Right Side"),
    PlotLayoutSpec::new(42.0, 8.0, 24.0, 0.3490659, S, "Center"),
    PlotLayoutSpec::new(66.0, 6.0, 36.0, 0.5235988, S, "Center"),
    PlotLayoutSpec::new(88.0, 5.0, 52.0, 0.6981317, S, "Center"),
    PlotLayoutSpec::new(120.0, 18.0, 56.0, 0.0, S, "Right Side"),
    PlotLayoutSpec::new(112.0, 18.0, 28.0, 0.0, S, "Right Side"),
    PlotLayoutSpec::new(150.0, 22.0, -6.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(162.0, 32.0, 24.0, 0.0, S, "Right Side"),
    PlotLayoutSpec::new(168.0, 28.0, 56.0, 1.570451, S, "Center"),
    PlotLayoutSpec::new(186.0, 36.0, -6.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(196.0, 40.0, 24.0, 0.0, S, "Right"),
    PlotLayoutSpec::new(196.0, 38.0, 64.0, 1.570451, M, "Far Right"),
    PlotLayoutSpec::new(140.0, 20.0, 100.0, 2.617994, M, "Center"),
    PlotLayoutSpec::new(-596.0, 40.0, -776.0, 0.0003454686, M, "Center"),
    PlotLayoutSpec::new(-604.0, 36.0, -848.0, 0.7853982, L, "Center"),
    PlotLayoutSpec::new(-644.0, 32.0, -800.0, -1.570796, S, "Center"),
    PlotLayoutSpec::new(-692.0, 18.0, -796.0, 0.0003454686, M, "Right"),
    PlotLayoutSpec::new(-732.0, 20.0, -852.0, -0.523599, L, "Center"),
    PlotLayoutSpec::new(-700.0, 14.0, -740.0, 2.356195, M, "Center"),
    PlotLayoutSpec::new(-648.0, 24.0, -744.0, 1.570796, M, "Center"),
    PlotLayoutSpec::new(-612.0, 36.0, -716.0, 3.141247, S, "Right"),
    PlotLayoutSpec::new(-604.0, 38.0, -664.0, 1.570796, S, "Right Side"),
    PlotLayoutSpec::new(-580.0, 42.0, -640.0, 1.570796, S, "Right"),
    PlotLayoutSpec::new(-648.0, 25.0, -700.0, 1.570796, S, "Right Side"),
    PlotLayoutSpec::new(-644.0, 26.0, -656.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-672.0, 18.0, -640.0, 1.570796, S, "Right"),
    PlotLayoutSpec::new(-641.0, 29.0, -620.372, 0.7853982, M, "Far Right"),
    PlotLayoutSpec::new(-600.0, 46.0, -556.0, 2.356195, L, "Center"),
    PlotLayoutSpec::new(-670.0, 28.0, -568.0, 3.141247, S, "Left"),
    PlotLayoutSpec::new(-684.0, 22.0, -592.0, 3.141247, S, "Left"),
    PlotLayoutSpec::new(-708.0, 17.0, -632.0, 1.570796, S, "Far Right Side"),
    PlotLayoutSpec::new(-728.0, 8.0, -662.0, 1.919862, S, "Center"),
    PlotLayoutSpec::new(-740.0, 6.0, -638.0, 2.094395, S, "Center"),
    PlotLayoutSpec::new(-756.0, 5.0, -616.0, 2.268928, S, "Center"),
    PlotLayoutSpec::new(-760.0, 18.0, -584.0, 1.570796, S, "Right Side"),
    PlotLayoutSpec::new(-732.0, 18.0, -592.0, 1.570796, S, "Right Side"),
    PlotLayoutSpec::new(-698.0, 22.0, -554.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-728.0, 32.0, -542.0, 1.570796, S, "Right Side"),
    PlotLayoutSpec::new(-760.0, 28.0, -536.0, 3.141247, S, "Center"),
    PlotLayoutSpec::new(-698.0, 36.0, -518.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-728.0, 40.0, -508.0, 1.570796, S, "Right"),
    PlotLayoutSpec::new(-768.0, 38.0, -508.0, 3.141247, M, "Far Right"),
    PlotLayoutSpec::new(-804.0, 20.0, -564.0, -2.094395, M, "Center"),
];

const LAVENDER_BEDS_PLOTS: [PlotLayoutSpec; 60] = [
    PlotLayoutSpec::new(144.0, 46.0, -78.375, 1.570451, M, "Center"),
    PlotLayoutSpec::new(78.5, 51.0, -100.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(130.0, 58.5, -163.0, 2.443461, L, "Far Right Side"),
    PlotLayoutSpec::new(39.5, 54.5, -81.0, -1.308997, S, "Center"),
    PlotLayoutSpec::new(71.5, 34.5, -30.5, 0.0, M, "Center"),
    PlotLayoutSpec::new(143.0, 23.5, 2.0, 2.007128, L, "Far Right Side"),
    PlotLayoutSpec::new(116.5, 9.625, 52.0, 0.08726646, S, "Center"),
    PlotLayoutSpec::new(85.0, 8.75, 63.5, -0.7853982, S, "Center"),
    PlotLayoutSpec::new(125.0, 8.0, 95.0, 1.570451, S, "Center"),
    PlotLayoutSpec::new(74.5, 7.75, 93.5, -1.221731, S, "Center"),
    PlotLayoutSpec::new(52.0, 5.0, 132.0, 0.0, M, "Center"),
    PlotLayoutSpec::new(64.0, 25.0, 22.5, -1.570451, S, "Center"),
    PlotLayoutSpec::new(27.74659, 34.0, 13.0, -1.832596, S, "Center"),
    PlotLayoutSpec::new(32.0, 19.5, 43.5, -1.570451, S, "Center"),
    PlotLayoutSpec::new(12.0, 9.0, 81.5, -1.570451, S, "Center"),
    PlotLayoutSpec::new(-92.0, 7.75, 98.0, -0.6981317, M, "Center"),
    PlotLayoutSpec::new(-30.0, 10.0, 78.5, 0.7853978, S, "Center"),
    PlotLayoutSpec::new(-31.0, 21.0, 44.5, -1.570451, S, "Center"),
    PlotLayoutSpec::new(-82.0, 21.5, 45.5, -1.221731, S, "Center"),
    PlotLayoutSpec::new(-8.5, 33.0, 1.0, 1.570451, S, "Center"),
    PlotLayoutSpec::new(-58.0, 30.5, -3.0, 0.0, M, "Center"),
    PlotLayoutSpec::new(-37.0, 41.5, -36.5, -1.570451, S, "Center"),
    PlotLayoutSpec::new(-65.5, 32.0, -35.5, 1.570451, S, "Center"),
    PlotLayoutSpec::new(-45.5, 47.0, -65.5, -1.570451, S, "Center"),
    PlotLayoutSpec::new(-4.0, 46.5, -73.0, 1.570451, S, "Center"),
    PlotLayoutSpec::new(-29.0, 56.5, -106.0, 2.96706, S, "Center"),
    PlotLayoutSpec::new(-114.0, 31.5, -40.0, -1.308997, M, "Center"),
    PlotLayoutSpec::new(-135.75, 27.5, -108.5, -2.6529, L, "Left Side"),
    PlotLayoutSpec::new(-105.0, 34.5, -154.5, -0.7330383, S, "Center"),
    PlotLayoutSpec::new(-50.5, 56.0, -150.0, 0.0, M, "Center"),
    PlotLayoutSpec::new(-625.625, 46.0, -560.0, 3.141247, M, "Center"),
    PlotLayoutSpec::new(-604.0, 51.0, -625.5, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-541.0, 58.5, -574.0, -2.268928, L, "Far Right Side"),
    PlotLayoutSpec::new(-623.0, 54.5, -664.5, 0.2617994, S, "Center"),
    PlotLayoutSpec::new(-673.5, 34.5, -632.5, 1.570796, M, "Center"),
    PlotLayoutSpec::new(-706.0, 23.5, -561.0, -2.70526, L, "Far Right Side"),
    PlotLayoutSpec::new(-756.0, 9.625, -587.5, 1.658063, S, "Center"),
    PlotLayoutSpec::new(-767.5, 8.75, -619.0, 0.7853982, S, "Center"),
    PlotLayoutSpec::new(-799.0, 8.0, -579.0, 3.141247, S, "Center"),
    PlotLayoutSpec::new(-797.5, 7.75, -629.5, 0.3490657, S, "Center"),
    PlotLayoutSpec::new(-836.0, 5.0, -652.0, 1.570796, M, "Center"),
    PlotLayoutSpec::new(-726.5, 25.0, -640.0, 0.0003454686, S, "Center"),
    PlotLayoutSpec::new(-717.0, 34.0, -676.2534, -0.2617998, S, "Center"),
    PlotLayoutSpec::new(-747.5, 19.5, -672.0, 0.0003454686, S, "Center"),
    PlotLayoutSpec::new(-785.5, 9.0, -692.0, 0.0003454686, S, "Center"),
    PlotLayoutSpec::new(-802.0, 7.75, -796.0, 0.8726646, M, "Center"),
    PlotLayoutSpec::new(-782.5, 10.0, -734.0, 2.356194, S, "Center"),
    PlotLayoutSpec::new(-748.5, 21.0, -735.0, 0.0003454686, S, "Center"),
    PlotLayoutSpec::new(-749.5, 21.5, -786.0, 0.3490657, S, "Center"),
    PlotLayoutSpec::new(-705.0, 33.0, -712.5, 3.141247, S, "Center"),
    PlotLayoutSpec::new(-701.0, 30.5, -762.0, 1.570796, M, "Center"),
    PlotLayoutSpec::new(-667.5, 41.5, -741.0, 0.0003454686, S, "Center"),
    PlotLayoutSpec::new(-668.5, 32.0, -769.5, 3.141247, S, "Center"),
    PlotLayoutSpec::new(-638.5, 47.0, -749.5, 0.0003454686, S, "Center"),
    PlotLayoutSpec::new(-631.0, 46.5, -708.0, 3.141247, S, "Center"),
    PlotLayoutSpec::new(-598.0, 56.5, -733.0, -1.745329, S, "Center"),
    PlotLayoutSpec::new(-664.0, 31.5, -818.0, 0.2617989, M, "Center"),
    PlotLayoutSpec::new(-595.5, 27.5, -839.75, -1.082104, L, "Left Side"),
    PlotLayoutSpec::new(-549.5, 34.5, -809.0, 0.8377581, S, "Center"),
    PlotLayoutSpec::new(-554.0, 56.0, -754.5, 1.570796, M, "Center"),
];

const GOBLET_PLOTS: [PlotLayoutSpec; 60] = [
    PlotLayoutSpec::new(-38.00001, -4.0, -110.0, 2.86234, S, "Right"),
    PlotLayoutSpec::new(-64.00002, 4.0, -98.00002, -0.6108653, S, "Right"),
    PlotLayoutSpec::new(-85.64, 6.0, -80.02, -0.8834857, S, "Right"),
    PlotLayoutSpec::new(-82.203, -4.0, -121.07, 2.548181, M, "Left"),
    PlotLayoutSpec::new(-118.665, 12.0, -92.44, 0.5962044, L, "Right Side"),
    PlotLayoutSpec::new(-39.73, 0.0, -64.44, 0.7853982, M, "Right"),
    PlotLayoutSpec::new(-72.6, 8.0, -40.315, 2.059489, S, "Right"),
    PlotLayoutSpec::new(-117.81, 10.0, -23.37, -1.343904, M, "Center"),
    PlotLayoutSpec::new(-140.246, 12.0, -51.004, 2.007129, S, "Right"),
    PlotLayoutSpec::new(-147.905, 15.0, -24.543, 0.1745329, S, "Right"),
    PlotLayoutSpec::new(-158.78, 15.0, 18.875, -1.833119, M, "Right"),
    PlotLayoutSpec::new(-116.0, 15.0, 23.99996, 1.308997, M, "Right"),
    PlotLayoutSpec::new(-72.0, 11.618, 12.0, -0.2391101, L, "Far Left Side"),
    PlotLayoutSpec::new(-32.00002, -20.0, 17.99998, 3.141593, S, "Left"),
    PlotLayoutSpec::new(-14.018, -16.02, -36.035, -0.7853982, S, "Right"),
    PlotLayoutSpec::new(68.24998, -16.0, -39.56697, -0.5235988, S, "Right"),
    PlotLayoutSpec::new(67.0, -20.0, 0.2499695, 3.141593, S, "Right"),
    PlotLayoutSpec::new(73.188, -24.0, 28.851, 2.094395, S, "Right"),
    PlotLayoutSpec::new(44.0, -24.0, 53.81, 2.094395, M, "Right"),
    PlotLayoutSpec::new(0.0, -24.0, 79.99998, 3.141593, S, "Right"),
    PlotLayoutSpec::new(93.99998, -26.0, -70.00002, 2.356194, S, "Right"),
    PlotLayoutSpec::new(106.0, -14.0, -36.00003, -0.3490659, S, "Left"),
    PlotLayoutSpec::new(130.465, -18.0, -56.628, 1.221731, S, "Right"),
    PlotLayoutSpec::new(144.5, -26.0, -32.00003, 0.0, S, "Right"),
    PlotLayoutSpec::new(123.75, -28.0, 4.0, -1.570796, M, "Right"),
    PlotLayoutSpec::new(120.0, -28.0, 39.99997, -1.570796, S, "Right"),
    PlotLayoutSpec::new(130.0, -36.0, 71.99997, 3.141593, S, "Right"),
    PlotLayoutSpec::new(95.545, -33.68, 59.08, 0.5235988, S, "Right"),
    PlotLayoutSpec::new(184.0, -44.0, 23.49998, 1.570796, S, "Right"),
    PlotLayoutSpec::new(188.0, -40.0, 60.0, 3.141593, L, "Far Right Side"),
    PlotLayoutSpec::new(-594.0, -4.0, -742.0, -1.850049, S, "Right"),
    PlotLayoutSpec::new(-606.0, 4.0, -768.0, 0.9599311, S, "Right"),
    PlotLayoutSpec::new(-623.98, 6.0, -789.64, 0.6873107, S, "Right"),
    PlotLayoutSpec::new(-582.93, -4.0, -786.203, -2.164208, M, "Left"),
    PlotLayoutSpec::new(-611.56, 12.0, -822.665, 2.167001, L, "Right Side"),
    PlotLayoutSpec::new(-639.56, 0.0, -743.73, 2.356195, M, "Right"),
    PlotLayoutSpec::new(-663.685, 8.0, -776.6, -2.6529, S, "Right"),
    PlotLayoutSpec::new(-680.63, 10.0, -821.81, 0.2268923, M, "Center"),
    PlotLayoutSpec::new(-652.996, 12.0, -844.246, -2.70526, S, "Right"),
    PlotLayoutSpec::new(-679.457, 15.0, -851.905, 1.745329, S, "Right"),
    PlotLayoutSpec::new(-722.875, 15.0, -862.78, -0.262323, M, "Right"),
    PlotLayoutSpec::new(-727.9999, 15.0, -820.0, 2.879793, M, "Right"),
    PlotLayoutSpec::new(-716.0, 11.618, -776.0, 1.331686, L, "Far Left Side"),
    PlotLayoutSpec::new(-722.0, -20.0, -736.0, -1.570796, S, "Left"),
    PlotLayoutSpec::new(-667.965, -16.02, -718.018, 0.7853982, S, "Right"),
    PlotLayoutSpec::new(-664.433, -16.0, -635.75, 1.047198, S, "Right"),
    PlotLayoutSpec::new(-704.25, -20.0, -637.0, -1.570796, S, "Right"),
    PlotLayoutSpec::new(-732.851, -24.0, -630.812, -2.617994, S, "Right"),
    PlotLayoutSpec::new(-757.81, -24.0, -660.0, -2.617994, M, "Right"),
    PlotLayoutSpec::new(-784.0, -24.0, -704.0, -1.570796, S, "Right"),
    PlotLayoutSpec::new(-634.0, -26.0, -610.0, -2.356195, S, "Right"),
    PlotLayoutSpec::new(-668.0, -14.0, -598.0, 1.22173, S, "Left"),
    PlotLayoutSpec::new(-647.372, -18.0, -573.535, 2.792527, S, "Right"),
    PlotLayoutSpec::new(-672.0, -26.0, -559.5, 1.570796, S, "Right"),
    PlotLayoutSpec::new(-708.0, -28.0, -580.25, 0.0, M, "Right"),
    PlotLayoutSpec::new(-744.0, -28.0, -584.0, 0.0, S, "Right"),
    PlotLayoutSpec::new(-776.0, -36.0, -574.0, -1.570796, S, "Right"),
    PlotLayoutSpec::new(-763.08, -33.68, -608.455, 2.094395, S, "Right"),
    PlotLayoutSpec::new(-727.5, -44.0, -520.0, -3.141593, S, "Right"),
    PlotLayoutSpec::new(-764.0, -40.0, -516.0, -1.570796, L, "Far Right Side"),
];

const SHIROGANE_PLOTS: [PlotLayoutSpec; 60] = [
    PlotLayoutSpec::new(-34.0, 2.02, 116.0, -1.308997, M, "Center"),
    PlotLayoutSpec::new(-27.0, 14.52, 75.0, -1.308997, S, "Center"),
    PlotLayoutSpec::new(5.0, 6.02, 125.0, -1.134464, S, "Center"),
    PlotLayoutSpec::new(7.0, 20.02, 79.0, -2.617994, S, "Center"),
    PlotLayoutSpec::new(40.0, 20.02, 63.0, 0.5235988, S, "Center"),
    PlotLayoutSpec::new(45.0, 10.02, 99.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(50.0, 10.02, 144.0, 3.141593, L, "Center"),
    PlotLayoutSpec::new(115.0, 11.6, 130.0, 2.356194, M, "Center"),
    PlotLayoutSpec::new(90.0, 16.02, 93.0, -2.181662, S, "Center"),
    PlotLayoutSpec::new(118.0, 22.02, 70.0, 2.356194, S, "Center"),
    PlotLayoutSpec::new(89.0, 22.02, 47.0, 1.047198, S, "Center"),
    PlotLayoutSpec::new(58.0, 26.02, 22.0, 0.4363323, S, "Center"),
    PlotLayoutSpec::new(68.0, 31.59, -18.0, 0.4886922, M, "Center"),
    PlotLayoutSpec::new(105.0, 30.02, 6.0, 0.7853982, S, "Center"),
    PlotLayoutSpec::new(131.0, 34.02, 32.0, 2.356194, M, "Center"),
    PlotLayoutSpec::new(136.0, 40.02, -30.0, 0.7853982, L, "Center"),
    PlotLayoutSpec::new(-105.0, 2.02, 74.0, 2.094395, S, "Center"),
    PlotLayoutSpec::new(-88.0, 14.52, 39.0, 2.356194, S, "Center"),
    PlotLayoutSpec::new(-135.0, 2.02, 50.0, -0.7853982, M, "Center"),
    PlotLayoutSpec::new(-107.0, 10.02, 12.0, 0.9599311, S, "Center"),
    PlotLayoutSpec::new(-164.0, 6.02, 22.0, 2.879793, S, "Center"),
    PlotLayoutSpec::new(-167.0, 6.02, -11.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(-151.0, 16.02, -50.0, -2.356194, S, "Center"),
    PlotLayoutSpec::new(-123.0, 17.51, -26.0, -2.356194, M, "Center"),
    PlotLayoutSpec::new(-80.0, 14.02, -14.0, 2.094395, S, "Center"),
    PlotLayoutSpec::new(-97.0, 16.02, -56.0, 0.7853982, S, "Center"),
    PlotLayoutSpec::new(-59.0, 20.02, -55.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(-75.0, 25.02, -117.0, 0.0, M, "Center"),
    PlotLayoutSpec::new(-116.0, 26.62, -93.0, 1.134464, S, "Center"),
    PlotLayoutSpec::new(-159.0, 30.02, -122.0, -0.7853982, L, "Center"),
    PlotLayoutSpec::new(-820.0, 2.02, -738.0, 0.2617994, M, "Center"),
    PlotLayoutSpec::new(-779.0, 14.52, -731.0, 0.2617994, S, "Center"),
    PlotLayoutSpec::new(-829.0, 6.02, -699.0, 0.4363323, S, "Center"),
    PlotLayoutSpec::new(-783.0, 20.02, -697.0, -1.047197, S, "Center"),
    PlotLayoutSpec::new(-767.0, 20.02, -664.0, 2.094395, S, "Center"),
    PlotLayoutSpec::new(-803.0, 10.02, -659.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-848.0, 10.02, -654.0, -1.570796, L, "Center"),
    PlotLayoutSpec::new(-834.0, 11.6, -589.0, -2.356195, M, "Center"),
    PlotLayoutSpec::new(-797.0, 16.02, -614.0, -0.6108653, S, "Center"),
    PlotLayoutSpec::new(-774.0, 22.02, -586.0, -2.356195, S, "Center"),
    PlotLayoutSpec::new(-751.0, 22.02, -615.0, 2.617994, S, "Center"),
    PlotLayoutSpec::new(-726.0, 26.02, -646.0, 2.007128, S, "Center"),
    PlotLayoutSpec::new(-686.0, 31.59, -636.0, 2.059489, M, "Center"),
    PlotLayoutSpec::new(-710.0, 30.02, -599.0, 2.356195, S, "Center"),
    PlotLayoutSpec::new(-736.0, 34.02, -573.0, -2.356195, M, "Center"),
    PlotLayoutSpec::new(-674.0, 40.02, -568.0, 2.356195, L, "Center"),
    PlotLayoutSpec::new(-778.0, 2.02, -809.0, -2.617994, S, "Center"),
    PlotLayoutSpec::new(-743.0, 14.52, -792.0, -2.356195, S, "Center"),
    PlotLayoutSpec::new(-754.0, 2.02, -839.0, 0.7853982, M, "Center"),
    PlotLayoutSpec::new(-716.0, 10.02, -811.0, 2.530728, S, "Center"),
    PlotLayoutSpec::new(-726.0, 6.02, -868.0, -1.832596, S, "Center"),
    PlotLayoutSpec::new(-693.0, 6.02, -871.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-654.0, 16.02, -855.0, -0.7853979, S, "Center"),
    PlotLayoutSpec::new(-678.0, 17.51, -827.0, -0.7853979, M, "Center"),
    PlotLayoutSpec::new(-690.0, 14.02, -784.0, -2.617994, S, "Center"),
    PlotLayoutSpec::new(-648.0, 16.02, -801.0, 2.356195, S, "Center"),
    PlotLayoutSpec::new(-649.0, 20.02, -763.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-587.0, 25.02, -779.0, 1.570796, M, "Center"),
    PlotLayoutSpec::new(-611.0, 26.62, -820.0, 2.70526, S, "Center"),
    PlotLayoutSpec::new(-582.0, 30.02, -863.0, 0.7853982, L, "Center"),
];

const EMPYREUM_PLOTS: [PlotLayoutSpec; 60] = [
    PlotLayoutSpec::new(90.0, -18.5, 177.0, 3.141593, S, "Center"),
    PlotLayoutSpec::new(144.0, -38.5, 129.0, 0.0, M, "Center"),
    PlotLayoutSpec::new(79.0, -28.5, 109.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(101.0, -28.5, 78.0, -1.570796, S, "Center"),
    PlotLayoutSpec::new(179.0, -48.5, 85.0, 3.141593, S, "Center"),
    PlotLayoutSpec::new(184.0, -48.5, 44.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(117.0, -28.5, 42.0, -1.570796, M, "Center"),
    PlotLayoutSpec::new(-36.0, -18.5, 86.0, -0.9599311, M, "Center"),
    PlotLayoutSpec::new(4.0, -18.5, 59.0, 0.6108653, S, "Center"),
    PlotLayoutSpec::new(-23.0, -18.5, 36.0, 3.141593, S, "Center"),
    PlotLayoutSpec::new(-71.0, -8.5, 135.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(-114.0, 1.5, 78.0, -0.9599311, L, "Center"),
    PlotLayoutSpec::new(-81.0, 1.5, 42.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(-23.0, -18.5, -36.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(-9.0, -18.5, -80.0, -0.6108653, S, "Center"),
    PlotLayoutSpec::new(-41.0, -8.5, -94.0, -2.181662, S, "Center"),
    PlotLayoutSpec::new(-86.0, 1.5, -43.0, -1.570796, M, "Center"),
    PlotLayoutSpec::new(-116.0, 11.5, -85.0, -2.181662, M, "Center"),
    PlotLayoutSpec::new(-46.0, 1.5, -146.0, -1.570796, S, "Center"),
    PlotLayoutSpec::new(-77.0, 1.5, -147.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(-72.0, 11.5, -190.0, 0.0, M, "Center"),
    PlotLayoutSpec::new(-140.0, 21.5, -136.0, -1.570796, L, "Center"),
    PlotLayoutSpec::new(114.0, -18.5, -37.0, 0.9599311, S, "Center"),
    PlotLayoutSpec::new(131.0, -18.5, -61.0, 2.530727, S, "Center"),
    PlotLayoutSpec::new(88.0, -18.5, -88.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(185.0, -38.5, -54.0, 0.0, M, "Center"),
    PlotLayoutSpec::new(151.0, -28.5, -100.0, 0.9599311, S, "Center"),
    PlotLayoutSpec::new(44.0, 1.5, -135.0, -1.570796, S, "Center"),
    PlotLayoutSpec::new(52.0, 1.5, -176.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(93.0, 1.5, -178.0, 0.0, L, "Center"),
    PlotLayoutSpec::new(-881.0, -18.5, -614.0, -1.570796, S, "Center"),
    PlotLayoutSpec::new(-833.0, -38.5, -560.0, 1.570796, M, "Center"),
    PlotLayoutSpec::new(-813.0, -28.5, -625.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-782.0, -28.5, -603.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(-789.0, -48.5, -525.0, -1.570796, S, "Center"),
    PlotLayoutSpec::new(-748.0, -48.5, -520.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-746.0, -28.5, -587.0, 0.0, M, "Center"),
    PlotLayoutSpec::new(-790.0, -18.5, -740.0, 0.6108653, M, "Center"),
    PlotLayoutSpec::new(-763.0, -18.5, -700.0, 2.181662, S, "Center"),
    PlotLayoutSpec::new(-740.0, -18.5, -727.0, -1.570796, S, "Center"),
    PlotLayoutSpec::new(-839.0, -8.5, -775.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-782.0, 1.5, -818.0, 0.6108653, L, "Center"),
    PlotLayoutSpec::new(-746.0, 1.5, -785.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-668.0, -18.5, -727.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-624.0, -18.5, -713.0, 0.9599311, S, "Center"),
    PlotLayoutSpec::new(-610.0, -8.5, -745.0, -0.6108651, S, "Center"),
    PlotLayoutSpec::new(-661.0, 1.5, -790.0, 0.0, M, "Center"),
    PlotLayoutSpec::new(-619.0, 11.5, -820.0, -0.6108651, M, "Center"),
    PlotLayoutSpec::new(-558.0, 1.5, -750.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(-557.0, 1.5, -781.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-514.0, 11.5, -776.0, 1.570796, M, "Center"),
    PlotLayoutSpec::new(-568.0, 21.5, -844.0, 0.0, L, "Center"),
    PlotLayoutSpec::new(-667.0, -18.5, -590.0, 2.530728, S, "Center"),
    PlotLayoutSpec::new(-643.0, -18.5, -573.0, -2.181662, S, "Center"),
    PlotLayoutSpec::new(-616.0, -18.5, -616.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-650.0, -38.5, -519.0, 1.570796, M, "Center"),
    PlotLayoutSpec::new(-604.0, -28.5, -553.0, 2.530728, S, "Center"),
    PlotLayoutSpec::new(-569.0, 1.5, -660.0, 0.0, S, "Center"),
    PlotLayoutSpec::new(-528.0, 1.5, -652.0, 1.570796, S, "Center"),
    PlotLayoutSpec::new(-526.0, 1.5, -611.0, 1.570796, L, "Center"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outdoor_plot_local_world_roundtrip_all_districts() {
        let local_pos = Position(Vec3A::new(3.5, 1.25, -4.75));
        let local_rot = 1.23_f32;
        for territory_id in HOUSING_DISTRICT_TERRITORIES {
            for division in [0u8, 1] {
                for plot_index in [0u8, 7, 15, 29] {
                    let spec = plot_spec(territory_id, division, plot_index)
                        .expect("plot spec must exist");
                    let (world_pos, world_rot) = spec.local_to_world(local_pos, local_rot);
                    let (back_pos, back_rot) = spec.world_to_local(world_pos, world_rot);
                    assert!(
                        (Vec3::from(back_pos.0) - Vec3::from(local_pos.0)).length() < 1e-4,
                        "mismatch for zone {territory_id} div {division} plot {plot_index}"
                    );
                    assert!(
                        wrap_angle_radians(back_rot - local_rot).abs() < 1e-4,
                        "rot mismatch for zone {territory_id} div {division} plot {plot_index}"
                    );
                }
            }
        }
    }

    #[test]
    fn parse_district_territory_supports_all_aliases() {
        assert_eq!(parse_district_territory("mist"), Some(339));
        assert_eq!(parse_district_territory("안개빛"), Some(339));
        assert_eq!(parse_district_territory("339"), Some(339));
        assert_eq!(parse_district_territory("lavender"), Some(340));
        assert_eq!(parse_district_territory("라벤더"), Some(340));
        assert_eq!(parse_district_territory("goblet"), Some(341));
        assert_eq!(parse_district_territory("하늘잔"), Some(341));
        assert_eq!(parse_district_territory("shirogane"), Some(641));
        assert_eq!(parse_district_territory("시로가네"), Some(641));
        assert_eq!(parse_district_territory("empyreum"), Some(979));
        assert_eq!(parse_district_territory("엠피레움"), Some(979));
        assert_eq!(parse_district_territory("foundation"), Some(979));
        assert_eq!(parse_district_territory("979"), Some(979));
    }

    #[test]
    fn canonical_plot_matches_requested_size_for_all_districts_and_divisions() {
        for territory_id in HOUSING_DISTRICT_TERRITORIES {
            for division in [0u8, 1] {
                for size in [PlotSize::Small, PlotSize::Medium, PlotSize::Large] {
                    let plot = canonical_plot_for_district_and_size(territory_id, division, size)
                        .expect("canonical plot must exist");
                    let spec = plot_spec(territory_id, division, plot).expect("spec must exist");
                    assert_eq!(
                        spec.size, size,
                        "zone {territory_id} div {division} plot {plot} mismatch"
                    );
                }
            }
        }
    }

    #[test]
    fn indoor_entry_transform_uses_radians_within_quantization_range() {
        let (house_pos, house_rot) = indoor_entry_transform(284);
        assert_eq!(house_pos, Position(Vec3A::new(-0.25, -0.39, 5.0)));
        assert!((-std::f32::consts::PI..=std::f32::consts::PI).contains(&house_rot));

        let (apt_pos, apt_rot) = indoor_entry_transform(999);
        assert_eq!(apt_pos, Position(Vec3A::ZERO));
        assert!((-std::f32::consts::PI..=std::f32::consts::PI).contains(&apt_rot));
    }

    #[test]
    fn wrap_angle_radians_handles_non_finite_and_huge_values() {
        assert_eq!(wrap_angle_radians(f32::NAN), 0.0);
        assert_eq!(wrap_angle_radians(f32::INFINITY), 0.0);
        assert_eq!(wrap_angle_radians(f32::NEG_INFINITY), 0.0);
        let huge = wrap_angle_radians(1.0e12);
        assert!((-std::f32::consts::PI..=std::f32::consts::PI).contains(&huge));
    }
}
