AETHERYTE_S1H1_FC = 56
AETHERYTE_S1H1_PRIVATE_APARTMENT = 59
AETHERYTE_F1H1_FC = 57
AETHERYTE_F1H1_PRIVATE_APARTMENT = 60
AETHERYTE_W1H1_FC = 58
AETHERYTE_W1H1_PRIVATE_APARTMENT = 61
AETHERYTE_E1H1_FC = 96
AETHERYTE_E1H1_PRIVATE_APARTMENT = 97
AETHERYTE_R1H1_FC = 164
AETHERYTE_R1H1_PRIVATE_APARTMENT = 165

AETHERYTE_FC_IDS = {
    AETHERYTE_S1H1_FC,
    AETHERYTE_F1H1_FC,
    AETHERYTE_W1H1_FC,
    AETHERYTE_E1H1_FC,
    AETHERYTE_R1H1_FC
}

AETHERYTE_PRIVATE_APARTMENT_IDS = {
    AETHERYTE_S1H1_PRIVATE_APARTMENT,
    AETHERYTE_F1H1_PRIVATE_APARTMENT,
    AETHERYTE_W1H1_PRIVATE_APARTMENT,
    AETHERYTE_E1H1_PRIVATE_APARTMENT,
    AETHERYTE_R1H1_PRIVATE_APARTMENT
}

function getApartmentIds(id, division_index)
    local is_main_division = nil

    if division_index == 0 then
        is_main_division = true
    elseif division_index == 1 then
        is_main_division = false
    end

    if is_main_division == nil then
        printf(player, "Invalid division_index (%s) for id %s", division_index, id)
    end

    local territory_id = nil
    local pop_range_id = nil

    if id == AETHERYTE_S1H1_PRIVATE_APARTMENT then
        territory_id = TERRITORY_S1H1

        if is_main_division then
            pop_range_id = 6486032
        else
            pop_range_id = 6486040
        end
    elseif id == AETHERYTE_F1H1_PRIVATE_APARTMENT then
        territory_id = TERRITORY_F1H1

        if is_main_division then
            pop_range_id = 6486042
        else
            pop_range_id = 6486043
        end
    elseif id == AETHERYTE_W1H1_PRIVATE_APARTMENT then
        territory_id = TERRITORY_W1H1

        if is_main_division then
            pop_range_id = 6486045
        else
            pop_range_id = 6486053
        end
    elseif id == AETHERYTE_E1H1_PRIVATE_APARTMENT then
        territory_id = TERRITORY_E1H1

        if is_main_division then
            pop_range_id = 6793801
        else
            pop_range_id = 6793819
        end
    elseif id == AETHERYTE_R1H1_PRIVATE_APARTMENT then
        territory_id = TERRITORY_R1H1

        if is_main_division then
            pop_range_id = 8710380
        else
            pop_range_id = 8710385
        end
    else
        printf(player, "Invalid id (%s) for Apartment PopRange lookup", id)
    end

    return territory_id, pop_range_id
end

function getEstateIds(id, plot_index)
    local is_main_division = nil

    if plot_index >= 0 and plot_index <= 29 then
        is_main_division = true
    elseif plot_index >= 30 and plot_index <= 59 then
        is_main_division = false
        plot_index = plot_index - 30
    end

    if is_main_division == nil then
        printf(player, "Invalid plot_index (%s) for id %s", plot_index, id)
    end

    local territory_id = nil
    local pop_range_id = nil

    if id == AETHERYTE_S1H1_FC or id == AETHERYTE_S1H1_PRIVATE_APARTMENT then
        territory_id = TERRITORY_S1H1

        if is_main_division then
            pop_range_id = 4387629 + plot_index
        else
            pop_range_id = 5347391 + plot_index
        end
    elseif id == AETHERYTE_F1H1_FC or id == AETHERYTE_F1H1_PRIVATE_APARTMENT then
        territory_id = TERRITORY_F1H1

        if is_main_division then
            pop_range_id = 4508875 + plot_index
        else
            pop_range_id = 5364207 + plot_index
        end
    elseif id == AETHERYTE_W1H1_FC or id == AETHERYTE_W1H1_PRIVATE_APARTMENT then
        territory_id = TERRITORY_W1H1

        if is_main_division then
            pop_range_id = 4508781 + plot_index
        else
            pop_range_id = 5362502 + plot_index
        end
    elseif id == AETHERYTE_E1H1_FC or id == AETHERYTE_E1H1_PRIVATE_APARTMENT then
        territory_id = TERRITORY_E1H1

        if is_main_division then
            pop_range_id = 6791358 + plot_index
        else
            pop_range_id = 6791423 + plot_index
        end
    elseif id == AETHERYTE_R1H1_FC or id == AETHERYTE_R1H1_PRIVATE_APARTMENT then
        territory_id = TERRITORY_R1H1

        if is_main_division then
            pop_range_id = 8698095 + plot_index
        else
            pop_range_id = 8698125 + plot_index
        end
    else
        printf(player, "Invalid id (%s) for Estate PopRange lookup", id)
    end

    return territory_id, pop_range_id
end

function doAction(player, in_combo)
    -- get the aetheryte they requested
    local id = player.teleport_query.aetheryte_id
    local is_fc_id = has_value(AETHERYTE_FC_IDS, id)
    local is_private_apartment_id = has_value(AETHERYTE_PRIVATE_APARTMENT_IDS, id)

    if is_fc_id or is_private_apartment_id then
        local unk_flags = player.teleport_query.unk_flags
        local is_apartment = is_private_apartment_id and unk_flags == 128

        -- TODO: for now it matches with the hardcoded value in handle_zone_change's OwnedHousing
        local ward = 0
        local division_plot_index = 0

        if is_apartment then
            territory_id, pop_range_id = getApartmentIds(id, division_plot_index)
        else
            territory_id, pop_range_id = getEstateIds(id, division_plot_index)
        end

        player:change_territory_housing_ward(territory_id, pop_range_id, ward)
    else
        -- warp there
        player:warp_aetheryte(id)
    end

    return EffectsBuilder()
end
