#[cfg(test)]
use diesel::Connection;
use diesel::prelude::*;
#[cfg(test)]
use diesel_migrations::MigrationHarness;

#[cfg(test)]
use crate::database::MIGRATIONS;
use crate::{
    PlayerData, WorldDatabase,
    database::{models, schema},
};

impl WorldDatabase {
    /// Returns a mutable handle to the underlying SQLite connection for isolated domain modules.
    pub fn connection_mut(&mut self) -> &mut SqliteConnection {
        &mut self.connection
    }

    /// Opens an in-memory SQLite database with all embedded migrations applied for unit tests.
    #[cfg(test)]
    pub fn new_in_memory() -> Self {
        let mut connection = diesel::SqliteConnection::establish(":memory:")
            .expect("Failed to open in-memory database!");
        connection.run_pending_migrations(MIGRATIONS).unwrap();
        crate::housing_sim::db::ensure_schema(&mut connection);
        Self { connection }
    }

    /// Lists character rows matching `filter_content_id` (or all characters when `None`).
    pub fn find_character_rows(
        &mut self,
        filter_content_id: Option<u64>,
    ) -> Result<Vec<models::Character>, diesel::result::Error> {
        use models::Character;
        use schema::character::dsl::*;

        if let Some(cid) = filter_content_id {
            character
                .filter(content_id.eq(cid as i64))
                .select(Character::as_select())
                .load(&mut self.connection)
        } else {
            character
                .select(Character::as_select())
                .load(&mut self.connection)
        }
    }

    /// Loads raw persistent `PlayerData` for `found_character` without requiring `GameData` item preparation.
    pub fn load_player_data_raw(
        &mut self,
        found_character: models::Character,
    ) -> Result<PlayerData, diesel::result::Error> {
        use models::*;

        let volatile = Volatile::belonging_to(&found_character)
            .select(Volatile::as_select())
            .first(&mut self.connection)?;
        let inventory = Inventory::belonging_to(&found_character)
            .select(Inventory::as_select())
            .first(&mut self.connection)?;
        let classjob = ClassJob::belonging_to(&found_character)
            .select(ClassJob::as_select())
            .first(&mut self.connection)?;
        let unlock = Unlock::belonging_to(&found_character)
            .select(Unlock::as_select())
            .first(&mut self.connection)?;
        let customize = Customize::belonging_to(&found_character)
            .select(Customize::as_select())
            .first(&mut self.connection)?;
        let quest = Quest::belonging_to(&found_character)
            .select(Quest::as_select())
            .first(&mut self.connection)?;
        let content = Content::belonging_to(&found_character)
            .select(Content::as_select())
            .first(&mut self.connection)?;
        let companion = Companion::belonging_to(&found_character)
            .select(Companion::as_select())
            .first(&mut self.connection)?;
        let aether_current = AetherCurrent::belonging_to(&found_character)
            .select(AetherCurrent::as_select())
            .first(&mut self.connection)?;
        let aetheryte = Aetheryte::belonging_to(&found_character)
            .select(Aetheryte::as_select())
            .first(&mut self.connection)?;
        let mentor = Mentor::belonging_to(&found_character)
            .select(Mentor::as_select())
            .first(&mut self.connection)?;
        let search_info = SearchInfo::belonging_to(&found_character)
            .select(SearchInfo::as_select())
            .first(&mut self.connection)?;
        let grand_company = GrandCompany::belonging_to(&found_character)
            .select(GrandCompany::as_select())
            .first(&mut self.connection)?;
        let buddy = Buddy::belonging_to(&found_character)
            .select(Buddy::as_select())
            .first(&mut self.connection)?;

        let glasses: [u16; 2] = inventory
            .equipped_glasses_ids
            .0
            .try_into()
            .unwrap_or([0; 2]);

        Ok(PlayerData {
            character: found_character,
            classjob,
            volatile,
            inventory: inventory.contents,
            unlock,
            content,
            companion,
            aether_current,
            aetheryte,
            city_state: customize.city_state as u8,
            quest,
            mentor,
            search_info,
            grand_company,
            buddy,
            equipped_glasses_ids: glasses,
            customize,
            ..Default::default()
        })
    }
}
