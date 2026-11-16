-- Generic handler for Warps
-- Use this for most warps that are just a yes/no option

local played_default_talk = false
local is_residental_menu = false

function onTalk(target, player)
    -- TODO: Use WarpCondition to deterine which one to use!
    local target_event_id = GAME_DATA:get_warp_default_talk_success(EVENT_ID)
    if target_event_id ~= nil and target_event_id ~= 0 then
        -- For warps like the residential Ferry Skippers, they talk first (server-side) before displaying a menu
        player:start_event(target_event_id, EVENT_TYPE_NEST, 0)
        player:play_scene(0, HIDE_HOTBAR | NO_DEFAULT_CAMERA, {})

        is_residental_menu = true -- TODO: placeholder
    else
        player:play_scene(00000, HIDE_HOTBAR, {0})
    end
end

function onReturn(scene, results, player)
    -- HACK: 255 is not a real scene, this is a Kawari implementation detail to tell we returned from nesting
    -- TODO: with nested args now we really do need a dedicated API for this!
    if scene == 255 then
        if not played_default_talk then
            played_default_talk = true

            player:play_scene(00000, HIDE_HOTBAR, {})
            return
        end
        if results[1] == 1 then
            -- ExitRange told us to warp out of the residental district
            player:warp(EVENT_ID)
        end
    else
        if results[1] == 1 then
            if is_residental_menu then
                -- open the housing menu
                local target_territory = GAME_DATA:get_warp_territory_type(EVENT_ID)
                player:start_event(1310721, EVENT_TYPE_NEST, 0)
                player:play_scene(0, HIDE_HOTBAR | NO_DEFAULT_CAMERA, {target_territory})
                return
            else
                player:warp(EVENT_ID)
            end
        end
    end

    player:finish_event()
end
