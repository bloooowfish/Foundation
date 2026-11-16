-- Generic handler for Warp NPCs

local is_residental_menu = false

function onTalk(target, player)
    -- TODO: Use WarpCondition to deterine which one to use!
    local target_event_id = GAME_DATA:get_warp_default_talk_success(EVENT_ID)
    if target_event_id ~= nil and target_event_id ~= 0 then
        -- For warps like the residential Ferry Skippers, they talk first (server-side) before displaying a menu
        player:start_event(target_event_id, EVENT_TYPE_NEST, 0)
        player:play_scene(0, HIDE_HOTBAR | NO_DEFAULT_CAMERA, {})

        is_residental_menu = true -- TODO: we need a way to detect the residental ferry skippers
    else
        player:play_scene(00000, HIDE_HOTBAR, {0})
    end
end

function onReturn(scene, results, player)
    if results[1] == 1 then
        if is_residental_menu then
            -- open the housing menu
            local target_territory = GAME_DATA:get_warp_territory_type(EVENT_ID)
            player:start_event(HOUSING_EXIT_RANGE, EVENT_TYPE_NEST, 0)
            player:play_scene(0, HIDE_HOTBAR | NO_DEFAULT_CAMERA, {target_territory})
            return
        else
            player:warp(EVENT_ID)
        end
    end

    player:finish_event()
end

function onNestedEventFinished(handler, results, player)
    if handler == HANDLER_TYPE_DEFAULT_TALK then
        player:play_scene(00000, HIDE_HOTBAR, {})
    end
    if handler == HANDLER_TYPE_EXIT_RANGE and results[1] == 1 then
        -- ExitRange told us to warp out of the residental district
        player:warp(EVENT_ID)
        player:finish_event()
    end
end
