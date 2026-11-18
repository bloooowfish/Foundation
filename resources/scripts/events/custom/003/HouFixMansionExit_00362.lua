-- Apartment Building Exit and Apartment Exit, the front door in every apartment lobby and every apartment proper

-- TODO
-- Ensure the player returns to a housing ward safely. For now we are unable to do this, so we send them back to the corresponding city-state's aetheryte plaza. See below for futher details.
-- Why do the Ishgard and Ul'dah warp_aetheryte calls spawn us away from the plaza and underground respectively?
-- Hide the enter lobby menu option via however that's done, as it makes no sense to show an enter lobby option within a lobby.

-- Scenes
SCENE_SHOW_MENU = 00000 -- Shows the main menu, which offers to display the list of apartments, or exit the building.

function onTalk(target, player)
    player:play_scene(SCENE_SHOW_MENU, HIDE_HOTBAR, {})
end

function onReturn(scene, results, player)
    local EXIT_BUILDING <const> = 1
    local ENTER_LOBBY <const> = 2 -- Retail never shows this. We're currently not setting something properly to hide it.

    if results[1] == EXIT_BUILDING then
        player:finish_event()
        player:housing_exit_house()
        return
    elseif results[1] == ENTER_LOBBY then -- TODO: remove this once we can hide this menu option
        player:send_message("This option shouldn't be here and is a bug in Kawari, please select a different option.")
    end
    
    player:finish_event()
end

function onYield(scene, yield_id, results, player)
    if results[1] == 1 then
        player:resume_event(SCENE_SHOW_MENU, yield_id, {90, 1})
        return
    end
    player:finish_event()
end
