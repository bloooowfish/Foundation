//! Status effect list handling.

use crate::{ToServer, ZoneConnection};
use kawari::common::ObjectId;

impl ZoneConnection {
    /// Request to gain a status effect.
    pub async fn gain_effect(&mut self, effect_id: u16, effect_param: u16, effect_duration: f32) {
        self.handle
            .send(ToServer::GainEffect(
                self.id,
                self.player_data.character.actor_id,
                effect_id,
                effect_param,
                effect_duration,
                self.player_data.character.actor_id,
            ))
            .await;
    }

    /// Request to lose a status effect.
    pub async fn lose_effect(
        &mut self,
        effect_id: u16,
        effect_param: u16,
        effect_source_actor_id: ObjectId,
    ) {
        self.handle
            .send(ToServer::LoseEffect(
                self.id,
                self.player_data.character.actor_id,
                effect_id,
                effect_param,
                effect_source_actor_id,
            ))
            .await;
    }
}
