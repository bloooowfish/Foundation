//! All things zone related, such as changing the weather or warping.

use crate::{
    InstanceQuery, ObsfucationData, TeleportReason, ToServer, ZoneConnection,
    inventory::BuyBackList, lua::LuaZone, zone_connection::TeleportQuery,
};
use kawari::{
    common::{
        FestivalId, HandlerId, HouseId, HouseUnit, HousingFlag, LandData, Position,
        TerritoryIntendedUse, WarpType, timestamp_secs,
    },
    constants::OBFUSCATION_ENABLED_MODE,
    ipc::zone::{
        ActorControlCategory, ApartmentBuildingState, Condition, DutyFinderSetting, FurnitureList,
        House, HouseExterior, HouseList, HouseStatus, HousingInteriorDetails, PlotSize,
        ServerZoneIpcData, ServerZoneIpcSegment, WardDivision, WardDivisionInfo, ZoneInit,
        ZoneInitFlags,
    },
    packet::{ConnectionState, PacketSegment, ScramblerKeyGenerator, SegmentData, SegmentType},
};

impl ZoneConnection {
    /// Request the global server state to change our zone.
    pub async fn change_zone(
        &mut self,
        new_zone_id: u16,
        new_position: Option<Position>,
        new_rotation: Option<f32>,
        warp_type: WarpType,
        loading_screen_vfx_id: u16,
    ) {
        self.teleport_reason = TeleportReason::NotSpecified;
        self.handle
            .send(ToServer::ChangeZone(
                self.id,
                self.player_data.character.actor_id,
                new_zone_id,
                new_position,
                new_rotation,
                warp_type,
                loading_screen_vfx_id,
            ))
            .await;
    }

    /// Handle the zone change information from the server state.
    pub async fn handle_zone_change(
        &mut self,
        new_zone_id: u16,
        content_finder_condition_id: u16,
        weather_id: u8,
        exit_position: Position,
        exit_rotation: f32,
        lua_zone: &LuaZone,
        handler_id: Option<HandlerId>,
        query: InstanceQuery,
    ) {
        // Reset data
        self.spawned_in = false;
        self.fate_motivation_npcs.clear();
        self.content_handler_id = handler_id;
        self.player_data.buyback_list = BuyBackList::default();
        self.synced_level = None;

        let bound_by_duty = content_finder_condition_id != 0;

        // Commit back our zone id and other volatile info on zone change.
        {
            self.player_data.volatile.zone_id = new_zone_id as i32;
            self.player_data.volatile.position = exit_position;
            self.player_data.volatile.rotation = exit_rotation as f64;

            let mut db = self.database.lock();
            db.commit_volatile(&self.player_data);
        }

        // Generate obsfucation-related keys if needed.
        if self.config.enable_packet_obsfucation {
            let seed1 = fastrand::u8(..);
            let seed2 = fastrand::u8(..);
            let seed3 = fastrand::u32(..);

            let generator = ScramblerKeyGenerator::new();

            self.obsfucation_data = ObsfucationData {
                seed1,
                seed2,
                seed3,
            };

            let ConnectionState::Zone { scrambler_keys, .. } = &mut self.state else {
                panic!("Unexpected connection type!");
            };
            *scrambler_keys = Some(generator.generate(seed1, seed2, seed3));

            tracing::info!(
                "You enabled packet obsfucation in your World config, if things break please report it!",
            );
        }

        // they send the initialize packet again for some reason
        self.send_segment(PacketSegment {
            segment_type: SegmentType::Initialize,
            data: SegmentData::Initialize {
                actor_id: self.player_data.character.actor_id,
                timestamp: timestamp_secs(),
            },
            ..Default::default()
        })
        .await;

        // Send obsfucation init
        let ipc = if self.config.enable_packet_obsfucation {
            ServerZoneIpcSegment::new(ServerZoneIpcData::InitializeObfuscation {
                unk_before: [0; 6],
                obsfucation_mode: OBFUSCATION_ENABLED_MODE,
                seed1: !self.obsfucation_data.seed1,
                seed2: !self.obsfucation_data.seed2,
                seed3: !self.obsfucation_data.seed3,
            })
        } else {
            ServerZoneIpcSegment::new(ServerZoneIpcData::InitializeObfuscation {
                unk_before: [0; 6],
                obsfucation_mode: 0,
                seed1: 0,
                seed2: 0,
                seed3: 0,
            })
        };
        self.send_ipc_self(ipc).await;

        self.send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::InitResponse {
            actor_id: self.player_data.character.actor_id,
        }))
        .await;

        // Send owned housing list (unsure where this fits in before ZoneInit?!)
        {
            let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::OwnedHousing {
                unk1: Default::default(),
                unk2: Default::default(),
                unk3: Default::default(),
                unk4: Default::default(),
                unk5: Default::default(),
                apartment: LandData {
                    id: HouseId {
                        unit: HouseUnit {
                            apartment_division_plot_index: 0,
                            apartment_flag: true,
                        },
                        unk1: 0,
                        ward_index: 0,
                        room_number: 1,
                        territory_type_id: 340,
                        world_id: self.config.world_id,
                    },
                    flags: 19,
                    unk1: 0,
                },
            });
            self.send_ipc_self(ipc).await;
        }

        // Send the list of available items if we're in an inn, since its only accessible via their beds.
        if lua_zone.intended_use == TerritoryIntendedUse::Inn {
            let display_ids;
            {
                let mut game_data = self.game_data.lock();
                display_ids = game_data.get_latest_fittingshop_display_ids();
            }

            let ipc =
                ServerZoneIpcSegment::new(ServerZoneIpcData::UpdateFittingShop { display_ids });
            self.send_ipc_self(ipc).await;
        }

        let mut game_festival_ids = self.config.active_festivals.map(FestivalId);
        let mut game_festival_phases = [0; 8];

        // Set up ocean fishing
        if bound_by_duty {
            let mut game_data = self.game_data.lock();
            if let Some(spots) =
                game_data.lookup_ikd_route_spots_via_content(content_finder_condition_id as u32)
            {
                // Special festivals used for ocean fishing.
                game_festival_ids = [
                    FestivalId(101),
                    FestivalId(102),
                    FestivalId::default(),
                    FestivalId::default(),
                    FestivalId::default(),
                    FestivalId::default(),
                    FestivalId::default(),
                    FestivalId::default(),
                ];

                game_festival_phases = [
                    spots[0] as u16 + 1, // IKDSpot + 1
                    23,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                ];
            }
        }

        // Init Zone
        {
            let mut flags = if self.initial_login {
                ZoneInitFlags::INITIAL_LOGIN
            } else if bound_by_duty {
                ZoneInitFlags::INSTANCED_CONTENT | ZoneInitFlags::UNK5
            } else {
                ZoneInitFlags::default()
            };

            if !bound_by_duty {
                flags |= ZoneInitFlags::ENABLE_FLYING;
            } else {
                self.offered_teleport = None; // Discard any previously offered teleports once we're in a duty.

                let mut game_data = self.game_data.lock();
                if game_data.is_duty_recorder_allowed(content_finder_condition_id) {
                    flags |= ZoneInitFlags::ENABLE_RECORD_READY_CHECK;
                }
            }

            let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::ZoneInit(ZoneInit {
                territory_type: new_zone_id,
                weather_id,
                flags,
                content_finder_condition_id,
                game_festival_ids,
                game_festival_phases,
                ui_festival_ids: self.config.active_festivals.map(FestivalId),
                ..Default::default()
            }));
            self.send_ipc_self(ipc).await;
        }

        let level;
        {
            let mut game_data = self.game_data.lock();

            level = self
                .player_data
                .inventory
                .equipped
                .calculate_item_level(&mut game_data) as u32;
        }

        self.actor_control_self(ActorControlCategory::SetItemLevel { level })
            .await;

        if !bound_by_duty {
            self.send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::DailyQuests {
                unk1: [0; 56],
            }))
            .await;

            self.send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::DailyQuestRepeatFlags { unk1: [0; 8] },
            ))
            .await;
        }

        if self.initial_login {
            self.send_quest_information().await;
            self.send_crafting_gathering_information().await;
        }

        if lua_zone.intended_use == TerritoryIntendedUse::HousingOutdoor
            && let InstanceQuery::Ward { ward } = query
        {
            let mut houses = [House::default(); 30];

            // First, populate the houses in this ward. Note that for now, we treat every ward the same.
            // TODO: For now, we hardcode 3 prefab houses as demonstration units until we implement more of the system. One cottage, one house and one mansion, all set to be individually owned and locked. Plots 5, 6, and 12 were chosen due to their close proximity to each other.
            // Glade house (Wood)
            houses[4] = House {
                plot_size: PlotSize::Medium,
                status: HouseStatus::HouseBuilt,
                flags: HousingFlag::OPEN,
                exterior: HouseExterior {
                    roof_id: 1029,
                    walls_id: 3589,
                    windows_id: 2562,
                    door_id: 514,
                    ..Default::default()
                },
                ..Default::default()
            };

            // Hingan mansion (Mokuzo)
            houses[5] = House {
                plot_size: PlotSize::Large,
                status: HouseStatus::HouseBuilt,
                flags: HousingFlag::OPEN,
                exterior: HouseExterior {
                    roof_id: 1081,
                    walls_id: 3632,
                    windows_id: 2579,
                    door_id: 531,
                    ..Default::default()
                },
                ..Default::default()
            };

            // Highland cottage (Wood)
            houses[11] = House {
                plot_size: PlotSize::Small,
                status: HouseStatus::HouseBuilt,
                flags: HousingFlag::OPEN | HousingFlag::OWNED_BY_FC,
                exterior: HouseExterior {
                    roof_id: 1136,
                    walls_id: 3687,
                    windows_id: 2598,
                    door_id: 550,
                    ..Default::default()
                },
                ..Default::default()
            };

            self.send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::HouseList(
                HouseList {
                    land_id: 0,
                    ward,
                    territory_type_id: lua_zone.zone_id,
                    world_id: self.config.world_id,
                    division_info: WardDivisionInfo {
                        apartment_building_state: ApartmentBuildingState::IsOwn,
                        ward_division: WardDivision::MainDivision,
                    },
                    houses,
                },
            )))
            .await;

            // Finally, populate the exterior furniture.
            // TODO: Actually send some real furniture, once we can do that!
            for index in 0..15 {
                self.send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::FurnitureList(
                    FurnitureList {
                        count: 15,
                        index,
                        ..Default::default()
                    },
                )))
                .await;
            }
        }

        if lua_zone.intended_use == TerritoryIntendedUse::HousingIndoor {
            // Bare minimum stuff to make housing interiors load
            self.send_ipc_self(ServerZoneIpcSegment::new(
                ServerZoneIpcData::HousingInteriorDetails(HousingInteriorDetails::default()),
            ))
            .await;

            // TODO: Limit this by interior size once we have furniture persistence
            for index in 0..6 {
                // The LandId is currently set so that plugins like HousingPos/Buildingway can plop stuff down
                self.send_ipc_self(ServerZoneIpcSegment::new(ServerZoneIpcData::FurnitureList(
                    FurnitureList {
                        id: HouseId {
                            unit: HouseUnit {
                                apartment_division_plot_index: 0,
                                apartment_flag: true,
                            },
                            unk1: 0,
                            room_number: 1,
                            ward_index: 0,
                            territory_type_id: 340,
                            world_id: self.config.world_id,
                        },
                        count: 6,
                        index,
                        unk2: 100, // Indoors
                        ..Default::default()
                    },
                )))
                .await;
            }
        }

        self.conditions
            .toggle_condition(Condition::BoundByDuty, bound_by_duty);
        self.conditions
            .toggle_condition(Condition::BoundByDuty56, bound_by_duty);

        self.send_conditions().await;
        self.update_class_info().await;
        self.send_stats().await;

        // Inform the server we're ready for it to send director data
        self.handle
            .send(ToServer::ReadyDirectorData(
                self.player_data.character.actor_id,
            ))
            .await;

        self.initial_login = false;
    }

    pub async fn warp(&mut self, warp_id: u32) {
        self.teleport_reason = TeleportReason::NotSpecified;
        self.handle
            .send(ToServer::Warp(
                self.id,
                self.player_data.character.actor_id,
                warp_id,
            ))
            .await;
    }

    pub async fn warp_aetheryte(
        &mut self,
        aetheryte_id: u32,
        housing_aethernet: bool,
        taking_offered_teleport: bool,
    ) {
        self.teleport_reason = TeleportReason::Aetheryte;
        self.handle
            .send(ToServer::WarpAetheryte(
                self.id,
                self.player_data.character.actor_id,
                aetheryte_id,
                housing_aethernet,
            ))
            .await;

        if self.can_offer_teleport() {
            let teleport_info = TeleportQuery {
                aetheryte_id: aetheryte_id as u16,
            };

            self.handle
                .send(ToServer::OfferTeleportToParty(
                    Some(self.party_id),
                    self.player_data.character.actor_id,
                    self.player_data.volatile.zone_id as u16,
                    teleport_info,
                ))
                .await;
            // Reset it so we don't inadvertently share an invalid aetheryte (housing/city aethernet shard)
            self.can_share_teleport = false;
        }

        if taking_offered_teleport {
            self.offered_teleport = None;
        }
    }

    pub async fn discover_location(&mut self, map_id: u32, map_part_id: u32) {
        let ipc = ServerZoneIpcSegment::new(ServerZoneIpcData::LocationDiscovered {
            map_id,
            map_part_id,
        });
        self.send_ipc_self(ipc).await;
    }

    pub async fn join_content(&mut self, id: u16, settings: DutyFinderSetting) {
        // Store our old information, for when we leave the instance
        self.old_zone_id = self.player_data.volatile.zone_id as u16;
        self.old_position = self.player_data.volatile.position;
        self.old_rotation = self.player_data.volatile.rotation as f32;

        self.handle
            .send(ToServer::JoinContent(
                self.id,
                self.player_data.character.actor_id,
                id,
                settings,
            ))
            .await;
    }

    /// Changes the time of day in the current zone for our client.
    pub async fn set_eorzean_time(&mut self, offset: i64) {
        self.send_ipc_self(ServerZoneIpcSegment::new(
            ServerZoneIpcData::EorzeanTimeOffset { offset },
        ))
        .await;
    }
}
