//! The open questions settled by mangos-classic (round 12): each test
//! pins what mangos-classic does where wvs did otherwise, the question's
//! number in docs/DECISIONS.md by its test.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::SpellEffect;
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::auras;
use crate::game::tests_p3_combat::{BRUTE, setup_with, spawn};
use crate::game::tests_p3_entity::{bob, on, spell};

// #12: the Cenarion Favor in the Gates of Ahn'Qiraj.

/// mangos-classic's `OutdoorPvPMgr::GetScriptOfAffectedZone` gives
/// Silithus's script the Gates of Ahn'Qiraj (zone 3478) with the Ruins and
/// the Temple, so the side that won the Cenarion Favor keeps it there, on
/// Kalimdor; Thistle Tea's zone list (and wvs's) left it out.
#[tokio::test]
async fn the_cenarion_favor_holds_in_the_gates_of_ahnqiraj() {
    use crate::game::tests_p3_world_pvp::{silithus, tick};
    let realm = harness::realm_with(silithus());
    let mut c = Client::connect(&realm, "Alice");
    let g = c.enter("Alice", Race::NightElf, Class::Warrior).await;
    let key = realm.registry.online(g).unwrap().map().key;
    assert_eq!(key.map, 1);
    let map = realm.maps.get(&realm, key);
    map.query(move |m| {
        m.outdoor
            .as_mut()
            .unwrap()
            .silithus
            .as_mut()
            .unwrap()
            .controller = Some(0);
        m.players.get_mut(&g).unwrap().zone = 3478;
    })
    .await;
    let mut has = false;
    for _ in 0..30 {
        tick(&map).await;
        has = map
            .query(move |m| m.players[&g].auras.has_spell(30754))
            .await
            .unwrap();
        if has {
            break;
        }
    }
    assert!(has, "the Alliance's favor at the Gates");
}

// #23: an untapped creature's soul shard.

const DRAIN: u32 = 912_300;
const SHARD: u32 = 6265;

fn drain_world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut s = spell(
        DRAIN,
        &[SpellEffect {
            effect: 6,
            aura: auras::AURA_CHANNEL_DEATH_ITEM,
            base_points: 0,
            base_dice: 1,
            target_a: 25,
            item_type: u64::from(SHARD),
            ..Default::default()
        }],
    );
    s.family_name = 5;
    d.spells.spells.insert(DRAIN, s);
    d.items.insert(
        SHARD,
        ItemTemplate {
            entry: SHARD,
            name: "Soul Shard".into(),
            display_id: 1000,
            stackable: 20,
            allowable_class: -1,
            allowable_race: -1,
            buy_count: 1,
            ..Default::default()
        },
    );
    d
}

/// mangos-classic's `Aura::HandleChannelDeathItem` gives a soul shard only
/// for a creature `IsTappedBy` the warlock (their tap or their group's):
/// an untapped one is the killing blow's, so a stranger's blow leaves the
/// warlock none. wvs counted no one's tap as everyone's.
#[tokio::test]
async fn an_untapped_creature_killed_by_a_stranger_leaves_the_warlock_no_shard() {
    use crate::game::map::spells::outcome::death_items;
    let (realm, _a, ga, map) = setup_with(drain_world(), Class::Warlock).await;
    let (_b, gb) = bob(&realm, ga).await;
    let got = on(&map, move |m| {
        let wolf = spawn(m, ga, BRUTE, 1, 5.0);
        let s = m.realm.data.spells.get(DRAIN).cloned().unwrap();
        auras::apply_spell(m, ga, wolf, &s, &[(0, 1)], Default::default()).unwrap();
        m.creature_mut(wolf).unwrap().tapped_by = None;
        let hp = m.creature(wolf).unwrap().fields.get_u32(UNIT_FIELD_HEALTH);
        let by_stranger = death_items(m, wolf, gb, hp);
        let by_her = death_items(m, wolf, ga, hp);
        (by_stranger, by_her)
    })
    .await;
    assert_eq!(got.0, Vec::<(Guid, u32, u32)>::new(), "Bob's blow taps it");
    assert_eq!(got.1, vec![(ga, SHARD, 1)], "her own blow, hers");
}

// #48: a druid's attack power by form.

/// mangos-classic's `Player::UpdateAttackPowerAndDamage` (since its
/// `adaa953`, "reimplement predatory strikes"): every druid's melee attack
/// power is three a level, twice the strength, less 20, in any form, with
/// the agility too in a form flagged for it (Cat Form's
/// `SHAPESHIFT_FLAG_AGILITY_ATTACK_BONUS`); the forms' own share comes from
/// their passive auras (3025, 1178, 9635). Ranged, Cat and Bear forms have
/// none, the rest agility less 10. wvs gave a caster druid no level share,
/// a cat two a level and a moonkin one and a half on top of the passives.
#[tokio::test]
async fn a_druids_attack_power_is_three_a_level_in_every_form_and_agility_in_cat() {
    use crate::game::map::auras::{FORM_BEAR, FORM_CAT, FORM_DIREBEAR, FORM_MOONKIN};
    use crate::game::map::derive;
    use crate::game::tests_p4_core_stats::{MAGE_POOLS, MAGE_STATS, data, on_map, player};
    let mut d = data(&[]);
    d.player_level_stats.insert((4, 11, 60), MAGE_STATS);
    d.player_class_stats.insert((11, 60), MAGE_POOLS);
    let (realm, _a, g) = player(d, Race::NightElf, Class::Druid).await;
    let got = on_map(&realm, move |m| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        crate::game::map::stats::set_level(p, &data, 60);
        let mut out = Vec::new();
        for f in [0, FORM_CAT, FORM_BEAR, FORM_DIREBEAR, FORM_MOONKIN] {
            let p = m.players.get_mut(&g).unwrap();
            p.fields.set_byte(UNIT_FIELD_BYTES_1, 2, f);
            derive::unit(m, g);
            let f = &m.players[&g].fields;
            out.push((
                f.get_u32(UNIT_FIELD_ATTACK_POWER),
                f.get_u32(UNIT_FIELD_RANGED_ATTACK_POWER),
            ));
        }
        out
    })
    .await;
    // Strength 35, agility 41 at 60.
    let melee = 60 * 3 + 35 * 2 - 20;
    assert_eq!(
        got,
        vec![
            (melee, 41 - 10),
            (melee + 41, 0),
            (melee, 0),
            (melee, 0),
            (melee, 41 - 10),
        ],
        "caster, cat, bear, dire bear, moonkin"
    );
}

/// A cast under way for `g`: a plain one, or a channel.
fn casting(m: &mut crate::game::map::Map, g: Guid, spell: u32, channel: bool) {
    use std::time::Duration;
    let now = m.now;
    m.caster_mut(g).unwrap().casting = Some(crate::game::map::spells::Casting {
        spell,
        targets: Default::default(),
        ends: now + Duration::from_secs(3),
        item: None,
        pushbacks: 0,
        channel_until: channel.then_some(now + Duration::from_secs(3)),
        duration: 3_000,
    });
}

// #28: a charm's end.

/// mangos-classic's `Unit::Uncharm` lets a creature (not a pet) go with
/// `AttackStop(true, true)`, which interrupts its cast and its channel
/// (`InterruptNonMeleeSpells`), and its auto-repeat and melee spells: a
/// freed creature's cast doesn't carry on, as Thistle Tea has it too. wvs
/// stopped its swing alone.
#[tokio::test]
async fn a_creature_let_go_from_a_charm_stops_its_cast_and_its_channel() {
    use crate::game::map::pets;
    use crate::game::tests_p5_data_aura::{CHARM_ONE, charm_data};
    let (_realm, _a, ga, map) = setup_with(charm_data(), Class::Warrior).await;
    let got = on(&map, move |m| {
        let mut out = Vec::new();
        for channel in [false, true] {
            let brute = spawn(m, ga, BRUTE, 1, 5.0);
            pets::charmed(m, ga, brute, false);
            casting(m, brute, CHARM_ONE, channel);
            pets::uncharmed(m, ga, brute);
            out.push(m.caster(brute).unwrap().casting.is_some());
        }
        out
    })
    .await;
    assert_eq!(got, vec![false, false], "a cast, a channel");
}

// #19 and #44: a pet's Stay from the bar.

/// mangos-classic's `HandlePetAction`, `COMMAND_STAY`: the pet's
/// `AttackStop(true, true)` (unless a combat script holds it) stops its
/// attack, interrupts its cast and lets go of its target, as Follow and
/// Passive do; wvs stopped the attack alone, the cast going on.
#[tokio::test]
async fn stay_from_the_bar_stops_the_pets_cast_and_its_target_too() {
    use crate::game::map::pets::{self, ACT_COMMAND, COMMAND_STAY};
    use crate::game::tests_p5_ai_pets::tamed_with;
    let (_realm, _h, gh, map, pet, wolf) = tamed_with(crate::game::tests_pets::data()).await;
    let got = on(&map, move |m| {
        pets::attack_target(m, pet, wolf);
        casting(m, pet, 1, false);
        pets::handle(
            m,
            gh,
            wow_world_messages::vanilla::opcodes::ClientOpcodeMessage::CMSG_PET_ACTION(
                CMSG_PET_ACTION {
                    pet,
                    data: pets::button(u32::from(COMMAND_STAY), ACT_COMMAND),
                    target: Guid::zero(),
                },
            ),
        );
        (
            m.caster(pet).unwrap().casting.is_some(),
            m.unit_fields(pet).unwrap().get_guid(UNIT_FIELD_TARGET),
            m.combat(pet).is_some_and(|c| c.attacking),
            pets::state(m, pet).map(|s| s.command),
        )
    })
    .await;
    assert_eq!(got, (false, Guid::zero(), false, Some(COMMAND_STAY)));
}

// #9: a battleground's creatures and the leash.

/// mangos-classic's `CombatManager::Update` leashes everywhere but a
/// dungeon ("disabled in instances", `IsDungeon`: an instance or a raid):
/// a battleground is no dungeon, so its creatures leash as the open
/// world's do, 30 yards from where the fight began (no 60-yard home guard
/// of their own). wvs let every copy's creatures fight on, battlegrounds'
/// too.
#[tokio::test]
async fn a_battlegrounds_creature_leashes_as_the_open_worlds_and_a_dungeons_doesnt() {
    use std::time::Duration;
    use crate::game::guid;
    use crate::game::map::creature::Creature;
    use crate::game::map::creature_combat::should_evade;
    use crate::game::map::{MapKey, ai};
    let mut d = crate::game::tests_p3_combat::data();
    for (id, kind) in [(489u32, 3u32), (36, 1)] {
        d.maps.insert(
            id,
            MapInfo {
                id,
                internal_name: format!("Map{id}"),
                name: format!("Map {id}"),
                instance_type: kind,
                player_limit: 10,
            },
        );
    }
    let realm = harness::realm_with(d);
    let mut out = Vec::new();
    for map in [489u32, 36] {
        let handle = realm.maps.get(&realm, MapKey { map, instance: 7 });
        let evades = on(&handle, move |m| {
            let t = m.realm.data.creature_template(BRUTE).cloned().unwrap();
            let at = Vector3d {
                x: 100.0,
                y: 100.0,
                z: 0.0,
            };
            let add = |m: &mut crate::game::map::Map, n: u32| {
                let c = Creature::new(guid::unit(BRUTE, 0x70_0000 + n), &t, 10, at, 0.0);
                m.add_creature(c.with_base())
            };
            let (brute, foe) = (add(m, 1), add(m, 2));
            ai::attacked(m, brute, foe);
            // Both 80 yards from where it began, nothing done for 13
            // seconds.
            let far = Vector3d { x: 180.0, ..at };
            m.creature_mut(brute).unwrap().position = far;
            m.creature_mut(foe).unwrap().position = far;
            m.now += Duration::from_secs(13);
            should_evade(m, brute, foe)
        })
        .await;
        out.push(evades);
    }
    assert_eq!(out, vec![true, false], "Warsong Gulch, Deadmines");
}

// #16: an unreachable victim, for one that doesn't move in its fight.

/// mangos-classic's `Unit::SelectHostileTarget` starts the evade timer for
/// an unreachable victim only `if (AI()->IsCombatMovement())`: a creature
/// whose combat movement is off (Eranikus, whose `boss_eranikusAI` and
/// Thistle Tea's script set it so on aggro) fights on whatever its reach.
/// wvs sent it home after 24 seconds out of reach, which failed The
/// Nightmare Manifests.
#[tokio::test]
async fn one_that_doesnt_move_in_its_fight_doesnt_evade_for_an_unreachable_victim() {
    use std::time::Duration;
    use crate::game::map::creature_combat::should_evade;
    use crate::game::map::{ai, event_ai};
    let (_realm, _a, ga, map) = setup_with(crate::game::tests_p3_combat::data(), Class::Warrior).await;
    let got = on(&map, move |m| {
        let mut out = Vec::new();
        for still in [false, true] {
            let brute = spawn(m, ga, BRUTE, 1, 5.0);
            ai::attacked(m, brute, ga);
            if still {
                event_ai::scripted_mut(m, brute).unwrap().no_combat_movement = true;
            }
            let long_ago = m.now;
            m.creature_mut(brute).unwrap().engagement.unreachable_since = Some(long_ago);
            m.now += Duration::from_secs(25);
            m.creature_mut(brute).unwrap().engagement.last_hostile = Some(m.now);
            out.push(should_evade(m, brute, ga));
        }
        out
    })
    .await;
    assert_eq!(got, vec![true, false], "a chaser, one that stands");
}

// #32: a scripted follower's evade.

/// mangos-classic's `FollowerAI::EnterEvadeMode` walks a follower whose
/// follow is under way back to where its fight began
/// (`GetCombatStartPosition`), from where it follows on; a creature
/// following by `MoveFollow` (not its main movement) loses the follow to
/// `MoveTargetedHome`, which takes it to the same place. wvs left it where
/// it stood, as Thistle Tea does.
#[tokio::test]
async fn a_scripted_follower_goes_back_to_where_its_fight_began_and_follows_on() {
    use crate::game::map::ai;
    use crate::game::map::creature::Motion;
    let (_realm, _a, ga, map) = setup_with(crate::game::tests_p3_combat::data(), Class::Warrior).await;
    let got = on(&map, move |m| {
        let leader = spawn(m, ga, BRUTE, 1, 5.0);
        let w = spawn(m, ga, BRUTE, 1, 8.0);
        m.creature_mut(w)
            .unwrap()
            .scripted
            .get_or_insert_with(Default::default)
            .follow = Some((leader, 3.0, std::f32::consts::PI));
        let began = m.creature(w).unwrap().position;
        ai::attacked(m, w, ga);
        // Drawn 20 yards off in its fight.
        let drawn = Vector3d {
            x: began.x + 20.0,
            ..began
        };
        m.creature_mut(w).unwrap().position = drawn;
        ai::enter_evade(m, w);
        let c = m.creature(w).unwrap();
        let bound = match &c.motion {
            Motion::Moving(walk) => Some(walk.destination()),
            _ => None,
        };
        (
            c.evading,
            bound.map(|b| crate::game::map::combat::dist(b, began) < 0.5),
            c.scripted.as_deref().and_then(|s| s.follow).map(|f| f.0) == Some(leader),
        )
    })
    .await;
    assert_eq!(got, (true, Some(true), true), "home to where it began, following");
}

// #45: a dead pet's corpse.

/// mangos-classic's hunter pet takes the corpse time of its template, as
/// any creature (`Creature::InitEntry`: by rank, `Corpse.Decay.NORMAL`'s
/// 300 seconds for a normal beast), and `Pet::Update` unsummons it once
/// that has run out; only a summoned pet's is five seconds
/// (`SetCorpseDelay(5)`). wvs gave a hunter's pet a minute.
#[tokio::test]
async fn a_dead_hunters_pet_lies_as_long_as_a_creature_of_its_rank() {
    use std::time::Duration;
    use crate::game::map::{combat, pets};
    use crate::game::tests_p5_ai_pets::tamed_with;
    let (_realm, _h, _gh, map, pet, wolf) = tamed_with(crate::game::tests_pets::data()).await;
    let got = on(&map, move |m| {
        let decay = m.cfg.creatures.corpse_decay_normal;
        combat::kill(m, wolf, pet);
        let start = m.now;
        let mut lying = Vec::new();
        for secs in [61, decay - 1, decay + 1] {
            m.now = start + Duration::from_secs(secs);
            pets::update(m);
            lying.push(m.creature(pet).is_some());
        }
        (decay, lying)
    })
    .await;
    assert_eq!(got, (300, vec![true, true, false]));
}

// #1: death's wear.

/// The worn chest's durability (of 50), in slot 4.
fn chest(m: &crate::game::map::Map, g: Guid) -> u32 {
    let p = &m.players[&g];
    let i = p.item_at(crate::game::store::BAG_PLAYER, 4).unwrap();
    p.items[i].fields.get_u32(ITEM_FIELD_DURABILITY)
}

/// Alice in a chest of 50, hardy.
fn chested(m: &mut crate::game::map::Map, g: Guid) {
    use crate::game::tests_p5_player_misc::CHEST;
    crate::game::tests_p3_combat::equip(m, g, CHEST, 4);
    let f = &mut m.players.get_mut(&g).unwrap().fields;
    f.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
    f.set_u32(UNIT_FIELD_HEALTH, 100_000);
}

/// mangos-classic's `Unit::Kill` wears a dead player's gear a tenth only
/// if no player was behind the blow (`responsiblePlayer`): their own hand
/// is a player's, so a death by it wears nothing; and the world's harm
/// (`EnvironmentalDamage`, dealt as their own) wears only for a fall
/// ("DealDamage not apply item durability loss at self damage"). wvs wore
/// gear at every death by one's own hand.
#[tokio::test]
async fn a_death_by_ones_own_hand_wears_nothing_but_a_falls() {
    use crate::game::map::{combat, death, movement};
    use crate::game::tests_p5_player_misc::wear_world;
    let (_realm, _a, ga, map) = setup_with(wear_world(), Class::Warrior).await;
    let worn = on(&map, move |m| {
        chested(m, ga);
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        let mut out = Vec::new();
        combat::kill_by(m, ga, ga, None);
        out.push(chest(m, ga));
        death::resurrect(m, ga, 1.0);
        movement::fall_damage(m, ga, 200.0);
        out.push((m.players[&ga].death != death::Death::Alive) as u32 * chest(m, ga));
        death::resurrect(m, ga, 1.0);
        combat::kill(m, brute, ga);
        out.push(chest(m, ga));
        out
    })
    .await;
    assert_eq!(worn, vec![50, 45, 40], "her own hand, a fall, a creature");
}

/// mangos-classic's `Unit::DealDamage` rolls a blow's one-in-200 wear in
/// `HandleDamageDealt`, which a killing blow's `Kill` takes the place of:
/// the blow that kills wears no point off either side, the victim's
/// death's tenth aside. wvs rolled every blow.
#[tokio::test]
async fn a_killing_blow_rolls_no_wear_of_its_own() {
    use crate::game::map::combat;
    use crate::game::map::vendor::FORCED_WEAR;
    use crate::game::tests_p5_player_misc::wear_world;
    let (_realm, _a, ga, map) = setup_with(wear_world(), Class::Warrior).await;
    let worn = on(&map, move |m| {
        chested(m, ga);
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        FORCED_WEAR.with(|f| f.set(Some((0.1, 4))));
        let mut out = Vec::new();
        // Her blow, then her killing blow.
        combat::deal_damage(m, ga, brute, 10);
        out.push(chest(m, ga));
        combat::deal_damage(m, ga, brute, 1_000_000);
        out.push(chest(m, ga));
        // A creature's killing blow on her: the death's five alone.
        let other = spawn(m, ga, BRUTE, 20, 5.0);
        combat::deal_damage(m, other, ga, 1_000_000);
        out.push(chest(m, ga));
        FORCED_WEAR.with(|f| f.set(None));
        out
    })
    .await;
    assert_eq!(worn, vec![49, 49, 44], "a blow; the kill; her death");
}

/// mangos-classic's `Unit::Kill` wears a priest's gear at the fatal blow
/// that a Spirit of Redemption saves her from (its durability branch isn't
/// gated by the spirit), if no player struck it; the spirit's end, its
/// Suicide, is her own hand and wears nothing. wvs wore it at the end.
#[tokio::test]
async fn a_spirit_of_redemption_wears_at_the_fatal_blow_and_not_at_its_end() {
    use crate::game::map::spells::redemption;
    use crate::game::map::{combat, death};
    use crate::game::tests_p5_player_misc::wear_world;
    let mut d = wear_world();
    for id in std::iter::once(redemption::FORM).chain(redemption::STATE) {
        let mut s = spell(
            id,
            &[SpellEffect {
                effect: 6,
                aura: auras::AURA_DUMMY,
                target_a: 1,
                ..Default::default()
            }],
        );
        s.name = "Spirit of Redemption".into();
        d.spells.spells.insert(id, s);
    }
    let (_realm, _a, ga, map) = setup_with(d, Class::Priest).await;
    let worn = on(&map, move |m| {
        chested(m, ga);
        m.players
            .get_mut(&ga)
            .unwrap()
            .spells
            .push(redemption::TALENT);
        let brute = spawn(m, ga, BRUTE, 20, 5.0);
        combat::deal_damage(m, brute, ga, 1_000_000);
        let saved = (redemption::spirit(m, ga), chest(m, ga));
        redemption::aura_gone(m, ga, redemption::STATE[1], true);
        let dead = (m.players[&ga].death != death::Death::Alive, chest(m, ga));
        (saved, dead)
    })
    .await;
    assert_eq!(worn, ((true, 45), (true, 45)), "at the blow; not again");
}
