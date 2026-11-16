-- Used for the housing menu available through Ferry Skippers and Aetherytes.

function onReturn(scene, results, player)
    player:finish_event()
end

function onYield(scene, id, results, player)
    local choice = results[1]
    local ward = results[2]
    local territory_id = results[3]
    local pop_range_id

    -- To handle switching wards while already in a housing zone
    if player.zone.intended_use == TERRITORY_INTENDED_USE_HOUSING_OUTDOOR then
        territory_id = player.zone.id
    end

    -- There's no link between these, so we have to maintain our own.
    if territory_id == TERRITORY_S1H1 then
        pop_range_id = HOUSING_POP_RANGE_S1H1
    elseif territory_id == TERRITORY_F1H1 then
        pop_range_id = HOUSING_POP_RANGE_F1H1
    elseif territory_id == TERRITORY_W1H1 then
        pop_range_id = HOUSING_POP_RANGE_W1H1
    elseif territory_id == TERRITORY_E1H1 then
        pop_range_id = HOUSING_POP_RANGE_E1H1
    elseif territory_id == TERRITORY_R1H1 then
        pop_range_id = HOUSING_POP_RANGE_R1H1
    else
        print("Unknown housing territory id: "..territory_id)
    end

    if choice == 1 then
        if ward == -1 then
            -- exit residential district
            player:finish_event({1}) -- (passed to the parent Warp)
            return
        elseif pop_range_id ~= nil then
            -- go to ward
            player:change_territory_housing_ward(territory_id, pop_range_id, ward)
        end
    end
    player:finish_event()
end
