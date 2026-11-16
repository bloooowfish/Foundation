-- Generic warp for housing entrances

-- Scenes
SCENE_GREETING = 00000 -- "Enter the estate hall?"

function onTalk(target, player)
    player:play_scene(SCENE_GREETING, HIDE_HOTBAR, {0})
end

function onReturn(scene, results, player)
    if results[1] == 1 then
        -- Enter the estate hall
        local house_id = 0 -- TODO: placeholder
        player:change_territory_house(house_id)
    end
    player:finish_event()
end
