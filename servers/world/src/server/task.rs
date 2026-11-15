use std::time::Instant;

use kawari::{
    common::{HandlerId, ObjectId},
    ipc::zone::{ActionRequest, ServerZoneIpcSegment},
};

use crate::ClientId;

#[derive(Debug, Clone)]
pub enum QueuedTaskData {
    CastAction {
        request: ActionRequest,
        /// Currently means if it has a cast bar.
        interruptible: bool,
    },
    LoseStatusEffect {
        effect_id: u16,
        effect_param: u16,
        effect_source_actor_id: ObjectId,
    },
    /// Fade out a dead actor.
    DeadFadeOut {
        actor_id: ObjectId,
        respawn_layout_id: Option<u32>,
    },
    /// Despawn a dead actor.
    DeadDespawn {
        actor_id: ObjectId,
        respawn_layout_id: Option<u32>,
    },
    /// Complete an EventAction
    CastEventAction { target: ObjectId },
    /// Make a fish bite.
    FishBite,
    /// Seal a boss wall.
    SealBossWall { id: u32, place_name: u32 },
    /// Generically send a packet segment, only used for `do_change_zone`. Don't abuse this as a generic task, you almost certainly want to create a new variant.
    PacketSegment { segment: ServerZoneIpcSegment },
    /// Used by directors since its tough to fit this into the director logic.
    WarpToPopRange { id: u32 },
    /// Reset a player's action combo status.
    ResetCombo,
    /// Respawn a new mob.
    RespawnMob { layout_id: u32 },
    /// Ends a FATE.
    EndFate { fate_id: u32 },
    /// Terminates a director.
    TerminateDirector { handler_id: HandlerId },
}

#[derive(Debug, Clone)]
pub struct QueuedTask {
    pub point: Instant,
    pub from_id: ClientId,
    pub from_actor_id: ObjectId,
    pub data: QueuedTaskData,
}

impl PartialEq for QueuedTask {
    fn eq(&self, other: &Self) -> bool {
        self.point == other.point
            && self.from_id == other.from_id
            && self.from_actor_id == other.from_actor_id
    }
}
