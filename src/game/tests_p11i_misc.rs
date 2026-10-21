//! Round 11's test-only rows, the rest of the player's (Thistle Tea's
//! `dev_commands_test`, `emotes_test`, `companion_visibility_test`,
//! `game_object_summons_test`, `player_test`, `containers_test` and
//! `open_lock_test`): a GM's quest added with no script, a script's emote
//! on a player, a charmed player's bar, a ritual's channel only for its
//! owner, a teleport ending the fight, a lockbox picked, and a vein's
//! refills.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::npc::Quest;
use crate::game::data::scripts::ScriptStep;
use crate::game::data::spells::{ATTR_EX_CHANNELED, SpellEffect};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey, objects};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

const ESCAPE: u32 = 96_950;
const ESCAPE_SCRIPT: u32 = 9_950;
const IMP: u32 = 3_955;

/// Thistle Tea's "starts no script without a selected creature": a GM's
/// `.addquest` puts the quest in the log and starts none of its scripts
/// (mangos-classic's `HandleQuestAddCommand`: `AddQuest(pQuest, nullptr)`,
/// no giver to start one from): the start script's summons never comes.
#[tokio::test]
async fn a_gms_added_quest_starts_no_script() {
    let mut d = crate::game::tests_p3_combat::data();
    d.npc.quests.insert(
        ESCAPE,
        Quest {
            id: ESCAPE,
            title: "Escape".into(),
            min_level: 1,
            level: 1,
            ..Default::default()
        },
    );
    let mut imp = d.creature_templates[&crate::game::tests_p3_combat::BRUTE].clone();
    imp.entry = IMP;
    d.creature_templates.insert(IMP, imp);
    d.scripts.quest_start_script.insert(ESCAPE, ESCAPE_SCRIPT);
    d.scripts.quest_start.insert(
        ESCAPE_SCRIPT,
        vec![ScriptStep {
            id: ESCAPE_SCRIPT,
            command: 10,
            datalong: [IMP, 60_000, 0, 0],
            x: crate::game::tests_p3_combat::START.0 + 3.0,
            y: crate::game::tests_p3_combat::START.1,
            z: crate::game::tests_p3_combat::START.2,
            ..Default::default()
        }],
    );
    let (_realm, mut a, g, map) = crate::game::tests_p3_combat::setup_with(d, Class::Warrior).await;
    a.say(&format!(".addquest {ESCAPE}")).await;
    a.system_message("Added Escape.").await;
    harness::an_update(&map).await;
    let (logged, imps) = on(&map, move |m| {
        (
            m.players[&g].journal.has(ESCAPE),
            m.creatures.iter().filter(|(_, c)| c.entry == IMP).count(),
        )
    })
    .await;
    assert!(logged, "in the log");
    assert_eq!(imps, 0, "no start script");
}

/// Thistle Tea's "uses the same cached persistent and one-shot animation
/// rules", with a player as the script's unit: a state emote
/// (`Emotes.dbc`'s, here 10, dance) is held in `UNIT_NPC_EMOTESTATE`, 0
/// ends it, a one-shot (3, wave) is sent as `SMSG_EMOTE` to those who see
/// the player, and an id that is no emote does nothing (mangos's
/// `Unit::HandleEmote`).
#[tokio::test]
async fn a_scripts_emote_on_a_player_holds_a_state_and_plays_a_one_shot() {
    let mut d = GameData::empty();
    d.emote_states.insert(10);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Dancer", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    b.enter("Watcher", Race::Human, Class::Warrior).await;
    b.sees(g).await;
    let map = realm.registry.online(g).unwrap().map();
    let state = move |m: &mut Map| m.players[&g].fields.get_u32(UNIT_NPC_EMOTESTATE);
    let dancing = on(&map, move |m| {
        crate::game::map::scripts::emote(m, g, 10);
        state(m)
    })
    .await;
    assert_eq!(dancing, 10, "a state held");
    let stopped = on(&map, move |m| {
        crate::game::map::scripts::emote(m, g, 0);
        state(m)
    })
    .await;
    assert_eq!(stopped, 0, "and ended");
    b.clear();
    on(&map, move |m| {
        crate::game::map::scripts::emote(m, g, 9_999);
        crate::game::map::scripts::emote(m, g, 3);
    })
    .await;
    let played = b
        .wait_for("SMSG_EMOTE", |m| match m {
            ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == g => Some(e.emote),
            _ => None,
        })
        .await;
    assert_eq!(
        played,
        Emote::OneshotWave,
        "a one-shot, the unknown one dropped"
    );
    assert_eq!(on(&map, state).await, 0, "no state from either");
}

/// Thistle Tea's "charmed players receive a command bar without creature
/// control requests": a player possessed by a player gives their
/// possessor a bar of ten buttons, attack and nine passives
/// (`pets::possess::bar`), with no pet spells and no creature pet's state
/// behind it.
#[tokio::test]
async fn a_possessed_players_bar_is_ten_buttons_and_no_pet() {
    use crate::game::tests_pets::{cast, enter, learn};
    use crate::game::tests_pets2::{MIND_CONTROL, bring, flag_pvp, more};
    let realm = harness::realm_with(more());
    let (mut a, ga, map) = enter(&realm, "A", "Priest", Race::Human, Class::Priest).await;
    let (_b, gb, _) = enter(&realm, "B", "Undying", Race::Undead, Class::Warrior).await;
    learn(&map, ga, &[MIND_CONTROL]);
    bring(&map, gb, ga, 8.0).await;
    a.sees(gb).await;
    flag_pvp(&map, vec![ga, gb]);
    a.clear();
    cast(&a, MIND_CONTROL, Some(gb)).await;
    let bar = a
        .wait_for("SMSG_PET_SPELLS", |m| match m {
            ServerOpcodeMessage::SMSG_PET_SPELLS(s) if s.pet == gb => Some(s.action_bars.clone()),
            _ => None,
        })
        .await
        .expect("a bar");
    assert_eq!(bar.action_bars.len(), 10);
    let expected = on(&map, move |m| crate::game::map::pets::possess::bar(m, gb)).await;
    assert_eq!(bar.action_bars, expected, "attack and nine passives");
    assert!(bar.spells.is_empty(), "no pet spells: {:?}", bar.spells);
    let creature_pet = on(&map, move |m| {
        crate::game::map::pets::state(m, gb).is_some()
    })
    .await;
    assert!(!creature_pet, "no creature pet's state");
}

/// A summoning ritual's object, and two channels raising it: one the
/// caster's own (`SPELL_EFFECT_TRANS_DOOR`), one wild
/// (`SPELL_EFFECT_SUMMON_OBJECT_WILD`).
const RITUAL: u32 = 98_960;
const OWNED_RITUAL: u32 = 997_801;
const WILD_RITUAL: u32 = 997_802;

fn rituals(d: &mut GameData) {
    use crate::game::tests_p3_cast_kit::*;
    d.gameobject_templates.insert(
        RITUAL,
        crate::game::tests_p3_entity_objects::object(
            RITUAL,
            objects::GO_RITUAL,
            "Ritual",
            &[(0, 3)],
        ),
    );
    d.spells.radii.insert(7, 3.0);
    let mut all = Vec::new();
    for (id, effect) in [
        (OWNED_RITUAL, objects::EFFECT_TRANS_DOOR),
        (WILD_RITUAL, objects::EFFECT_SUMMON_OBJECT_WILD),
    ] {
        let mut s = bare(id, 0);
        s.dmg_class = 0;
        s.attributes[1] |= ATTR_EX_CHANNELED;
        s.duration_index = LASTS_5S;
        s.effects[0] = SpellEffect {
            effect,
            misc_value: RITUAL as i32,
            target_a: 32,
            radius_index: 7,
            ..Default::default()
        };
        all.push(s);
    }
    insert(d, all);
}

/// Thistle Tea's "only an owned ritual attaches to its creator's
/// channel": a channel that raises its caster's own ritual is on it
/// (`UNIT_FIELD_CHANNEL_OBJECT`, as the warlock's on the summoning
/// portal); one that raises a wild one, no one's, isn't.
#[tokio::test]
async fn only_an_owned_ritual_takes_its_casters_channel() {
    use crate::game::tests_p3_spells_kit as kit;
    let realm = crate::game::tests_p3_cast_kit::fast_realm(rituals);
    let (_c, g, map) =
        crate::game::tests_p3_cast_kit::human(&realm, "P11M1", "Ritualist", Class::Warlock).await;
    let out = on(&map, move |m| {
        kit::ready(m, g, 60, &[OWNED_RITUAL, WILD_RITUAL]);
        let mut out = Vec::new();
        for id in [OWNED_RITUAL, WILD_RITUAL] {
            let before: Vec<Guid> = m.gameobjects.iter().map(|(_, o)| o.guid).collect();
            assert!(kit::cast_fresh(m, g, id, Default::default()), "{id} begins");
            let raised = m
                .gameobjects
                .iter()
                .find(|(_, o)| o.entry == RITUAL && !before.contains(&o.guid))
                .map(|(_, o)| o.guid);
            let channel = m.players[&g].fields.get_guid(UNIT_FIELD_CHANNEL_OBJECT);
            out.push((id, raised, channel, kit::casting(m, g)));
            crate::game::map::spells::interrupt(m, g);
        }
        out
    })
    .await;
    let (_, owned, channel, casting) = out[0];
    let owned = owned.expect("the owned ritual raised");
    assert_eq!(
        (channel, casting),
        (owned, Some(OWNED_RITUAL)),
        "on its own ritual"
    );
    let (_, wild, channel, casting) = out[1];
    let wild = wild.expect("the wild one raised");
    assert_eq!(casting, Some(WILD_RITUAL), "its channel goes on");
    assert_ne!(channel, wild, "not on a wild one");
}

/// Thistle Tea's "waits for the near teleport acknowledgement before
/// restoring a suspended pet", its fight: a teleport within the map that
/// no spell made ends its player's fight, as mangos's `TeleportTo` does
/// (`CombatStop`): out of combat, its swings stopped, its target kept
/// (mangos's `CombatStop` keeps it, where Thistle Tea clears it, and
/// leaves the creature's threat and any extra attacks as they are, which
/// Thistle Tea also clears); its pet, left behind, comes back only at the
/// teleport's acknowledgement
/// (`tests_p7_movement_acks::a_pet_left_by_a_teleport_comes_back_at_its_acknowledgement`).
/// A spell's short one keeps the fight.
#[tokio::test]
async fn a_teleport_no_spell_made_ends_the_fight_and_a_spells_short_one_keeps_it() {
    use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
    use crate::game::tests_pets::{BOAR, WOLF, data};
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "H");
    let h = a.enter("Hunter", Race::Dwarf, Class::Hunter).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let fight = move |m: &mut Map, boar: Guid| {
        crate::game::map::combat::engage(m, h, boar);
        crate::game::map::ai::attacked(m, boar, h);
        if let Some(c) = m.combat_mut(h) {
            c.victim = boar;
            c.attacking = true;
        }
        if let Some(f) = m.unit_fields_mut(h) {
            f.set_guid(UNIT_FIELD_TARGET, boar);
        }
    };
    let state = move |m: &mut Map, boar: Guid| {
        let p = &m.players[&h];
        (
            p.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT),
            m.combat(h).is_some_and(|c| c.attacking),
            p.fields.get_guid(UNIT_FIELD_TARGET) == boar,
        )
    };
    let (pet, boar, start) = on(&map, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&h).unwrap();
        crate::game::map::stats::set_level(p, &data, 20);
        let at = p.position();
        let pet = crate::game::map::pets::testing::spawn(
            m,
            h,
            WOLF,
            crate::game::map::pets::Kind::Hunter,
            20,
            at,
            0.0,
        );
        m.players.get_mut(&h).unwrap().owned.pet = Some(pet);
        crate::game::map::pets::sync_record(m, h);
        let boar = crate::game::tests_pets::place_in(m, h, BOAR, 0x71_2001, 4.0);
        fight(m, boar);
        (pet, boar, at)
    })
    .await;
    assert_eq!(
        on(&map, move |m| state(m, boar)).await,
        (true, true, true),
        "fighting"
    );
    // A spell's: Blink's ten yards keep it.
    let short = Vector3d {
        x: start.x + 10.0,
        ..start
    };
    let kept = on(&map, move |m| {
        m.near_teleport(h, short, 0.0);
        state(m, boar)
    })
    .await;
    assert_eq!(kept, (true, true, true), "a spell's short one keeps it");
    a.wait_for("the teleport", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: h,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    // No spell's: it ends.
    let far = Vector3d {
        x: start.x + 100.0,
        ..start
    };
    let ended = on(&map, move |m| {
        m.teleport(h, m.key, far, 0.0);
        (
            state(m, boar),
            m.players[&h].owned.pet.filter(|p| m.creature(*p).is_some()),
        )
    })
    .await;
    assert_eq!(
        ended.0,
        (false, false, true),
        "the fight over, the target kept"
    );
    assert_eq!(ended.1, None, "the pet left behind");
    a.wait_for("the teleport", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    a.send(MSG_MOVE_TELEPORT_ACK_Client {
        guid: h,
        movement_counter: a.teleport_order(),
        time: Default::default(),
    })
    .await;
    let deadline = tokio::time::Instant::now() + harness::WAIT;
    while on(&map, move |m| {
        m.players[&h].owned.pet.filter(|p| m.creature(*p).is_some())
    })
    .await
    .is_none()
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the pet back at the acknowledgement"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let _ = pet;
}

/// Thistle Tea's "typed completion unlocks, awards one point and opens
/// private loot": a lockbox's pick, done, unlocks it
/// (`ITEM_FLAG_UNLOCKED`), raises Lockpicking a point (mangos's
/// `UpdateGatherSkill`, at its lock's own level always) and opens its loot
/// to the picker; opened again by hand, it gives no second point.
#[tokio::test]
async fn a_picked_lockbox_is_unlocked_opened_and_teaches_once() {
    use crate::game::tests_p9_player_locks::*;
    let mut d = data();
    // This box wants Pick Lock at 50.
    d.items.get_mut(&BOX).unwrap().lock_id = 49;
    let realm = harness::realm_with(d);
    let (mut a, g, map) = setup(&realm, "A", "Alice").await;
    let skill = move |m: &mut Map| {
        crate::game::map::skills::value(&m.players[&g], LOCKPICKING).unwrap_or(0)
    };
    let box_item = on(&map, move |m| {
        learn_skill(m, g, LOCKPICKING, 50);
        give(m, g, BOX)
    })
    .await;
    a.clear();
    // The pick's attempt taken (its roll from 25 below to 37 above the
    // skill is `tests_p9_player_locks`'), done at once.
    on(&map, move |m| {
        use crate::game::map::professions::FORCED_ATTEMPT;
        FORCED_ATTEMPT.with(|f| f.set(Some(true)));
        cast(m, g, PICK_LOCK, item_target(box_item), None);
        hurry(m, g);
        crate::game::map::spells::update(m);
        FORCED_ATTEMPT.with(|f| f.set(None));
    })
    .await;
    let looted = a
        .wait_frame("SMSG_LOOT_RESPONSE", SMSG_LOOT_RESPONSE, |b| b.len() >= 9)
        .await;
    assert_eq!(
        u64::from_le_bytes(looted[0..8].try_into().unwrap()),
        box_item.guid()
    );
    let (unlocked, picked, looting) = on(&map, move |m| {
        let p = &m.players[&g];
        let i = p.item_by_guid(box_item).unwrap();
        (
            p.items[i].fields.get_u32(ITEM_FIELD_FLAGS) & 0x4 != 0,
            skill(m),
            m.players[&g].looting,
        )
    })
    .await;
    assert!(unlocked, "unlocked");
    assert_eq!(picked, 51, "a point");
    assert_eq!(looting, Some(box_item), "its loot open to the picker");
    a.send(CMSG_LOOT_RELEASE { guid: box_item }).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    let (bag, slot) = on(&map, move |m| {
        let p = &m.players[&g];
        let i = p.item_by_guid(box_item).unwrap();
        (p.items[i].bag, p.items[i].slot)
    })
    .await;
    a.send(CMSG_OPEN_ITEM {
        bag_index: bag,
        slot,
    })
    .await;
    a.wait_frame("the box opened by hand", SMSG_LOOT_RESPONSE, |b| {
        b.len() >= 9
    })
    .await;
    assert_eq!(on(&map, skill).await, 51, "no second point");
}

/// Mine `vein` until it opens (a low skill sometimes fails): whether it did.
async fn mine(map: &MapHandle, g: Guid, vein: Guid) -> bool {
    use crate::game::tests_loot::MINING;
    for _ in 0..60 {
        let opened = on(map, move |m| {
            if let Some(p) = m.players.get_mut(&g) {
                p.caster.gcd_until = None;
                p.caster.casting = None;
            }
            crate::game::map::spells::cast(
                m,
                g,
                MINING,
                crate::game::map::spells::Targets {
                    object: Some(vein),
                    ..Default::default()
                },
                None,
            );
            m.players[&g].looting == Some(vein)
        })
        .await;
        if opened {
            return true;
        }
    }
    false
}

/// Take the ore and let the vein go.
async fn empty(a: &mut Client, vein: Guid) {
    use crate::game::tests_loot::ORE;
    a.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    a.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == ORE => Some(()),
        _ => None,
    })
    .await;
    a.send(CMSG_LOOT_RELEASE { guid: vein }).await;
    crate::game::tests_p3_player2_ammo::handled(a).await;
}

/// Thistle Tea's "replenishes a vein, requires another cast, and clears
/// gain history on respawn": a vein of at least two uses fills again when
/// emptied the first time, its loot gone so it takes another cast to open
/// (`professions::object_looted`); its miner learns from it once a spawn
/// (`LootState::skilled`), not again at its second opening; mined out, it
/// goes, and come back it teaches again.
#[tokio::test]
async fn a_refilled_vein_takes_another_cast_and_teaches_once_a_spawn() {
    use crate::game::tests_loot::{MINING, MINING_PICK, VEIN, learn, skill_of};
    let mut d = crate::game::tests_loot::data();
    let t = d.gameobject_templates.get_mut(&VEIN).unwrap();
    (t.data[4], t.data[5]) = (2, 3);
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "Alice");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let vein = guid::game_object(VEIN, 50);
    a.sees(vein).await;
    on(&map, move |m| {
        learn(m, g, MINING, 186);
        let _ = crate::game::map::inventory::add_item(m, g, MINING_PICK, 1, false);
    })
    .await;
    assert!(mine(&map, g, vein).await, "it opens");
    assert_eq!(
        on(&map, move |m| skill_of(m, g, 186)).await,
        2,
        "a first vein teaches"
    );
    empty(&mut a, vein).await;
    let refilled = on(&map, move |m| {
        (
            m.gameobject(vein).map(|o| o.loot.is_none()),
            m.loot_state
                .skilled
                .get(&vein)
                .is_some_and(|s| s.contains(&g)),
            m.players[&g].looting,
        )
    })
    .await;
    assert_eq!(
        refilled,
        (Some(true), true, None),
        "filled again, shut, her gain kept"
    );
    // Another cast opens it; she learns nothing more of it.
    assert!(mine(&map, g, vein).await, "opens again");
    assert_eq!(
        on(&map, move |m| skill_of(m, g, 186)).await,
        2,
        "once a spawn"
    );
    empty(&mut a, vein).await;
    // Mined out, at the third if not the second.
    if on(&map, move |m| m.gameobject(vein).is_some()).await {
        assert!(mine(&map, g, vein).await);
        empty(&mut a, vein).await;
    }
    a.forgets(vein).await;
    assert!(
        on(&map, move |m| !m.loot_state.skilled.contains_key(&vein)).await,
        "its gains forgotten as it goes"
    );
    on(&map, |m| {
        let now = m.now;
        for r in m.loot_state.respawns.iter_mut() {
            r.0 = now;
        }
    })
    .await;
    a.sees(vein).await;
    assert!(mine(&map, g, vein).await, "back, it opens");
    assert_eq!(
        on(&map, move |m| skill_of(m, g, 186)).await,
        3,
        "and teaches again"
    );
}
