-- Generic warp for housing entrances

-- Scenes
SCENE_GREETING = 00000 -- "Enter the estate hall?"

function onTalk(target, player)
    player:play_scene(SCENE_GREETING, HIDE_HOTBAR, {0})
end

function onReturn(scene, results, player)
    if results[1] == 1 then
        -- Finish event before triggering housing territory change
        player:finish_event()
        local house_id = 0 -- TODO: placeholder
        player:change_territory_house(house_id)
        return
    end
    player:finish_event()
end
