use std::collections::HashMap;

use mlua::{UserData, UserDataFields};

use kawari::{
    common::{ObjectId, Position, TerritoryIntendedUse},
    ipc::zone::ServerZoneIpcSegment,
    packet::PacketSegment,
};

use crate::InstanceQuery;

use super::QueueSegments;

/// A strange concentration of Instance and Zone data, this is given by the server state.
#[derive(Debug, Default, Clone)]
pub struct LuaZone {
    pub zone_id: u16,
    pub query: InstanceQuery,
    // TODO: this is not updated when changed by the server state!
    pub weather_id: u8,
    pub internal_name: String,
    pub region_name: String,
    pub place_name: String,
    pub intended_use: TerritoryIntendedUse,
    pub map_id: u16,
    pub queued_segments: Vec<PacketSegment<ServerZoneIpcSegment>>,
    // NOTE: These are here to be accessed in Lua via the injected BASE_ID
    pub cached_npc_base_ids: HashMap<ObjectId, u32>,
    pub cached_eobj_base_ids: HashMap<u32, u32>,
    pub cached_pop_ranges: HashMap<u32, (Position, f32)>,
}

impl UserData for LuaZone {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, this| Ok(this.zone_id));
        fields.add_field_method_get("weather_id", |_, this| Ok(this.weather_id));
        fields.add_field_method_get("internal_name", |_, this| Ok(this.internal_name.clone()));
        fields.add_field_method_get("region_name", |_, this| Ok(this.region_name.clone()));
        fields.add_field_method_get("place_name", |_, this| Ok(this.place_name.clone()));
        fields.add_field_method_get("intended_use", |_, this| Ok(this.intended_use));
        fields.add_field_method_get("ward", |_, this| {
            if let InstanceQuery::Ward { ward } = this.query {
                Ok(mlua::Value::Integer(ward as i64))
            } else {
                Ok(mlua::Value::Nil)
            }
        });
    }
}

impl QueueSegments for LuaZone {
    fn queue_segment(&mut self, segment: PacketSegment<ServerZoneIpcSegment>) {
        self.queued_segments.push(segment);
    }
}
