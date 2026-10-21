//! Round 11's summons (11e), against Thistle Tea's `summon_lifecycle_test.exs`,
//! `player_group_reward_test.exs`, `creature_pet_owner_test.exs`,
//! `mini_pet_owner_test.exs` and `warlock_spells_test.exs`
//! (docs/parity/matrix.csv), with mangos-classic the rule where they
//! differ: a creature's pet, guardian or totem killed gives no XP but its
//! standing and quest credit, once, and leaves a corpse with no loot; a
//! creature's guardian goes at its time mid-fight; a creature hears of its
//! totems' and pets' coming, death and going; a pet's area aura reaches
//! its master within its radius, and leaves them out of it or with the
//! pet; a companion's passives (the Disgusting Oozeling's aura) are cast
//! as it comes.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::misc::{FactionDef, KillReputation, RANK_EXALTED};
use crate::game::data::scripts::*;
use crate::game::data::spells::{ATTR_PASSIVE, SpellDuration, SpellEffect};
use crate::game::fields::*;
use crate::game::map::loot::DYNFLAG_LOOTABLE;
use crate::game::map::pets::{self, Kind};
use crate::game::map::player::UNIT_FLAG_IN_COMBAT;
use crate::game::map::{Map, combat, event_ai, spells};
use crate::game::tests_p3_combat::{self as p3, BRUTE};
use crate::game::tests_p3_entity::take_quest;
use crate::game::tests_p3_spells_kit::{has, holder, pass};
use crate::game::tests_pets::{effect, spell};

const GUILD: u32 = 529;
const HUNT: u32 = 98_250;
/// A creature whose EventAI hears of its summons.
const SUMMONER: u32 = 3_790;
const TOTEM: u32 = 3_791;
const OOZE: u32 = 3_792;
const HELPER: u32 = 3_793;
/// A creature's guardian of a brute (effect 42), for five seconds.
const CALL_GUARDIAN: u32 = 990_960;
/// A totem (effect 87).
const CALL_TOTEM: u32 = 990_961;
/// A guardian of the old summon effect (28), a pet of mangos's.
const CALL_HELPER: u32 = 990_962;
/// A pet area aura (effect 119), a dummy, of 100 yards.
const LINK: u32 = 990_963;
/// A companion's passive: its area aura takes 20 from its owner's
/// resistances.
const OOZE_AURA: u32 = 990_964;
const CALL_OOZE: u32 = 990_965;

const FIVE_SECONDS: u32 = 950;
const HUNDRED_YARDS: u32 = 950;

fn step(command: u32, d1: u32) -> ScriptStep {
    ScriptStep {
        command,
        datalong: [d1, 0, 0, 0],
        ..Default::default()
    }
}

/// The summoner's event of `kind` for a summon of `entry`: its phase set to
/// `phase`.
fn heard(kind: u32, entry: u32, phase: u32) -> AiEvent {
    AiEvent {
        id: SUMMONER * 100 + phase,
        creature: SUMMONER,
        kind,
        chance: 100,
        params: [entry as i32, 0, 0, 0],
        actions: [380_100 + phase, 0, 0],
        ..Default::default()
    }
}

fn world() -> GameData {
    let mut d = crate::game::tests_p7_combat::data();
    for entry in [SUMMONER, TOTEM, OOZE, HELPER] {
        let mut t = p3::template(entry, 7);
        t.level_min = 30;
        t.level_max = 30;
        d.creature_templates.insert(entry, t);
    }
    d.misc.factions.insert(
        GUILD,
        FactionDef {
            id: GUILD,
            index: 13,
            name: "The Guild".into(),
            variants: [(0x1, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)],
            ..Default::default()
        },
    );
    d.misc.faction_by_index.insert(13, GUILD);
    d.misc.kill_reputation.insert(
        BRUTE,
        vec![KillReputation {
            faction: GUILD,
            value: 10,
            max_rank: RANK_EXALTED,
            team: 0,
            team_award: false,
        }],
    );
    d.npc.quests.insert(
        HUNT,
        crate::game::data::npc::Quest {
            id: HUNT,
            required_kills: [(BRUTE as i32, 20), (0, 0), (0, 0), (0, 0)],
            ..Default::default()
        },
    );
    d.spells.durations.insert(
        FIVE_SECONDS,
        SpellDuration {
            base: 5_000,
            per_level: 0,
            max: 5_000,
        },
    );
    d.spells.radii.insert(HUNDRED_YARDS, 100.0);
    let summon = |id: u32, kind: u32, entry: u32| {
        let mut s = spell(
            id,
            "Call",
            [
                SpellEffect {
                    misc_value: entry as i32,
                    ..effect(kind, 0, 1)
                },
                SpellEffect::default(),
                SpellEffect::default(),
            ],
        );
        s.duration_index = FIVE_SECONDS;
        s
    };
    for s in [
        summon(CALL_GUARDIAN, pets::EFFECT_SUMMON_GUARDIAN, BRUTE),
        summon(CALL_TOTEM, pets::EFFECT_SUMMON_TOTEM_SLOT1, TOTEM),
        summon(CALL_HELPER, pets::EFFECT_SUMMON, HELPER),
        spell(
            CALL_OOZE,
            "Call Ooze",
            [
                SpellEffect {
                    misc_value: OOZE as i32,
                    ..effect(pets::EFFECT_SUMMON_CRITTER, 0, 1)
                },
                SpellEffect::default(),
                SpellEffect::default(),
            ],
        ),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    let area = |aura: u32, misc: i32, amount: i32| SpellEffect {
        aura,
        misc_value: misc,
        radius_index: HUNDRED_YARDS,
        ..effect(pets::EFFECT_APPLY_AREA_AURA_PET, amount - 1, 1)
    };
    d.spells.spells.insert(
        LINK,
        spell(
            LINK,
            "Link",
            [
                area(4, 0, 1),
                SpellEffect::default(),
                SpellEffect::default(),
            ],
        ),
    );
    let mut ooze = spell(
        OOZE_AURA,
        "Disgusting",
        [
            area(22, 126, -20),
            SpellEffect::default(),
            SpellEffect::default(),
        ],
    );
    ooze.attributes[0] |= ATTR_PASSIVE;
    d.spells.spells.insert(OOZE_AURA, ooze);
    d.pets.create_spells.insert(OOZE, vec![OOZE_AURA]);
    d.scripts.ai_events.insert(
        SUMMONER,
        vec![
            heard(EVENT_SUMMONED_UNIT, TOTEM, 1),
            heard(EVENT_SUMMONED_JUST_DIED, TOTEM, 2),
            heard(EVENT_SUMMONED_JUST_DESPAWN, TOTEM, 3),
            heard(EVENT_SUMMONED_UNIT, HELPER, 4),
            heard(EVENT_SUMMONED_JUST_DIED, HELPER, 5),
            heard(EVENT_SUMMONED_JUST_DESPAWN, HELPER, 6),
        ],
    );
    for phase in 1..=6 {
        d.scripts
            .ai_scripts
            .insert(380_100 + phase, vec![step(44, phase)]);
    }
    d
}

/// `g`'s experience, standing with the guild and the hunt's kills.
fn earned(m: &Map, g: Guid) -> (u32, i32, u32) {
    let data = m.realm.data.clone();
    let p = &m.players[&g];
    let kills = p
        .journal
        .log
        .iter()
        .find(|l| l.quest == HUNT)
        .map_or(0, |l| l.kills[0]);
    (
        p.fields.get_u32(PLAYER_XP),
        crate::game::map::reputation::standing(p, &data, GUILD),
        kills,
    )
}

/// What `owner` summoned last.
fn last_summoned(m: &Map, owner: Guid) -> Guid {
    *event_ai::scripted(m, owner)
        .and_then(|s| s.summoned.last())
        .expect("a summon")
}

/// A second on: the queued spawned events, the summons' times and every
/// pet's step.
fn update(m: &mut Map) {
    m.now += Duration::from_secs(1);
    event_ai::update_despawns(m);
    pets::update(m);
}

fn lootable(m: &Map, g: Guid) -> bool {
    m.creature(g)
        .is_some_and(|c| c.fields.get_u32(UNIT_DYNAMIC_FLAGS) & DYNFLAG_LOOTABLE != 0)
}

/// Thistle Tea's "NPC pets award reduced XP alongside quest and reputation
/// credit" and "NPC pets grant one reward and leave an unlootable corpse
/// until removal", as mangos-classic has it (`XP::Gain` gives nothing for
/// `IsPet()`; `RewardSinglePlayerAtKill` the standing and credit of a
/// victim not `UNIT_FLAG_PLAYER_CONTROLLED`): a creature's pet, its
/// guardian (one of its own, of effect 42, and one of the old effect 28)
/// and its totem each give their killer no XP but their standing and quest
/// credit, once, and lie unlootable; a brute of no one's gives its XP.
#[tokio::test]
async fn a_creatures_pets_guardians_and_totems_give_standing_and_credit_once_but_no_xp() {
    let (_realm, _a, ga, map) =
        crate::game::tests_p7_combat::setup_with(world(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            p3::level(m, ga, 30);
            m.players.get_mut(&ga).unwrap().row.state.rest_bonus = 0.0;
            take_quest(m, ga, HUNT);
            let npc = p3::spawn(m, ga, SUMMONER, 30, 20.0);
            let at = m.creature(npc).unwrap().position;
            let mut victims = vec![
                pets::testing::spawn(m, npc, BRUTE, Kind::Demon, 30, at, 0.0),
                pets::testing::spawn(m, npc, BRUTE, Kind::Guardian, 30, at, 0.0),
                pets::testing::spawn(m, npc, BRUTE, Kind::Totem, 30, at, 0.0),
            ];
            spells::cast_triggered(m, npc, CALL_GUARDIAN, None);
            victims.push(last_summoned(m, npc));
            let mut out = Vec::new();
            for v in victims {
                let before = earned(m, ga);
                combat::deal_damage(m, ga, v, 1_000_000);
                let once = earned(m, ga);
                combat::deal_damage(m, ga, v, 1_000_000);
                let twice = earned(m, ga);
                let dead = m.creature(v).is_some_and(|c| !c.alive());
                out.push((
                    (once.0 - before.0, once.1 - before.1, once.2 - before.2),
                    twice == once,
                    dead,
                    lootable(m, v),
                ));
            }
            let free = p3::spawn(m, ga, BRUTE, 30, 3.0);
            let before = earned(m, ga);
            combat::deal_damage(m, ga, free, 1_000_000);
            let after = earned(m, ga);
            (
                out,
                (after.0 > before.0, after.1 - before.1, after.2 - before.2),
            )
        })
        .await
        .unwrap();
    let (victims, free) = r;
    for (i, v) in victims.iter().enumerate() {
        assert_eq!(
            *v,
            ((0, 10, 1), true, true, false),
            "victim {i} (a pet, a guardian, a totem, a creature's own guardian): \
             no XP, standing and credit once, a corpse with no loot"
        );
    }
    assert_eq!(free, (true, 10, 1), "a free brute's XP too");
}

/// Thistle Tea's "expires during combat and releases the owner's slot", as
/// mangos-classic's `Pet::Update` has it: a creature's guardian goes at its
/// time though it fights, one of its own (effect 42) and one of the old
/// summon effect (28) alike.
#[tokio::test]
async fn a_creatures_guardian_goes_at_its_time_mid_fight() {
    let (_realm, _a, ga, map) =
        crate::game::tests_p7_combat::setup_with(world(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let npc = p3::spawn(m, ga, SUMMONER, 30, 20.0);
            let mut out = Vec::new();
            for call in [CALL_GUARDIAN, CALL_HELPER] {
                spells::cast_triggered(m, npc, call, None);
                let g = last_summoned(m, npc);
                combat::start_attack(m, g, ga);
                combat::engage(m, g, ga);
                if let Some(c) = m.creature_mut(g) {
                    c.fields
                        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, true);
                }
                let mut fighting = true;
                for _ in 0..6 {
                    fighting &= m
                        .creature(g)
                        .is_none_or(|c| c.fields.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT));
                    update(m);
                }
                out.push((fighting, m.creature(g).is_none()));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(r, vec![(true, true); 2], "fighting to its time, and gone");
}

/// Thistle Tea's "totems and pets use the same death edge before
/// shutdown", as mangos-classic tells a creature of its totem
/// (`Totem::Summon`'s `JustSummoned`, `Unit::Kill`'s
/// `SummonedCreatureJustDied` through `GetMaster`, `Totem::UnSummon`'s
/// `SummonedCreatureDespawn`): a creature's EventAI hears of its totem's
/// and its guardian's coming, death and going, in turn.
#[tokio::test]
async fn a_creature_hears_of_its_totems_and_guardians_coming_death_and_going() {
    let (_realm, _a, ga, map) =
        crate::game::tests_p7_combat::setup_with(world(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let npc = p3::spawn(m, ga, SUMMONER, 30, 20.0);
            let mut out = Vec::new();
            for call in [CALL_TOTEM, CALL_HELPER] {
                spells::cast_triggered(m, npc, call, None);
                let g = last_summoned(m, npc);
                update(m);
                let came = event_ai::phase(m, npc);
                combat::deal_damage(m, ga, g, 1_000_000);
                let died = event_ai::phase(m, npc);
                for _ in 0..10 {
                    update(m);
                }
                out.push((came, died, event_ai::phase(m, npc), m.creature(g).is_none()));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(r[0], (1, 2, 3, true), "the totem's coming, death and going");
    assert_eq!(r[1], (4, 5, 6, true), "the guardian's");
}

/// Thistle Tea's "pets arm the area refresh on their self-cast link aura",
/// as mangos-classic's `AreaAura::Update` for `AREA_AURA_PET`: a pet's
/// area aura of its own is on its master within its radius, off them out
/// of it, on again back in it, and off them once the pet has lost it.
#[tokio::test]
async fn a_pets_area_aura_follows_its_master_in_and_out_of_its_radius() {
    let (_realm, _a, ga, map) =
        crate::game::tests_p7_combat::setup_with(world(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let at = m.players[&ga].position();
            let pet = pets::testing::spawn(m, ga, HELPER, Kind::Demon, 30, at, 0.0);
            spells::cast_triggered(m, pet, LINK, Some(pet));
            let from = holder(m, ga, LINK).map(|h| h.caster);
            let mut on = vec![(has(m, pet, LINK), has(m, ga, LINK))];
            let moved = |m: &mut Map, x: f32| {
                m.creature_mut(pet).unwrap().position.x += x;
                pass(m, 1_000);
                (has(m, pet, LINK), has(m, ga, LINK))
            };
            on.push(moved(m, 150.0));
            on.push(moved(m, -150.0));
            crate::game::map::auras::remove_spell(m, pet, LINK, Some(pet));
            pass(m, 1_000);
            on.push((has(m, pet, LINK), has(m, ga, LINK)));
            (from, on, pet)
        })
        .await
        .unwrap();
    let (from, on, pet) = r;
    assert_eq!(from, Some(pet), "the master's from the pet");
    assert_eq!(
        on,
        vec![(true, true), (true, false), (true, true), (false, false)],
        "near, far, back, and the pet's gone"
    );
}

/// Thistle Tea's "creation passives refresh the owner and expire after
/// dismissal", as mangos-classic's `EffectSummonCritter` (its
/// `InitPetCreateSpells`, whose passives the companion casts): the
/// Oozeling's aura reaches its owner as it comes, draws them into no
/// fight, and leaves them once it is sent away.
#[tokio::test]
async fn a_companions_passive_reaches_its_owner_and_leaves_with_it() {
    let (_realm, _a, ga, map) =
        crate::game::tests_p7_combat::setup_with(world(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            spells::cast_triggered(m, ga, CALL_OOZE, None);
            let ooze = m.players[&ga].owned.critter.expect("the ooze");
            let from = holder(m, ga, OOZE_AURA).map(|h| h.caster);
            let fighting = m.players[&ga]
                .fields
                .has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT);
            // Sent away: the same companion called again.
            spells::cast_triggered(m, ga, CALL_OOZE, None);
            let gone = m.creature(ooze).is_none();
            pass(m, 1_000);
            (ooze, from, fighting, gone, has(m, ga, OOZE_AURA))
        })
        .await
        .unwrap();
    let (ooze, from, fighting, gone, after) = r;
    assert_eq!(from, Some(ooze), "its aura on its owner, from it");
    assert!(!fighting, "no fight for it");
    assert!(gone, "sent away");
    assert!(!after, "and its aura with it");
}

/// The same on the real data, Soul Link's own (25228 from the demon, its
/// 100 yards): the warlock has it beside the voidwalker, not 150 yards
/// off it, and again back by it.
#[tokio::test]
#[ignore = "extended"]
async fn soul_link_leaves_a_warlock_a_hundred_yards_from_the_demon_and_comes_back() {
    use crate::game::tests_p3_spells_kit as kit;
    const SOUL_LINK: u32 = 19028;
    const SOUL_LINK_AURA: u32 = 25228;
    const SUMMON_VOIDWALKER: u32 = 697;
    if !kit::have_data() {
        return;
    }
    let realm = kit::realm();
    let (_a, w, map) = kit::player(&realm, "P11EA", "Linker", Race::Human, Class::Warlock).await;
    let on = map
        .query(move |m| {
            kit::ready(m, w, 60, &[SOUL_LINK]);
            spells::cast_triggered(m, w, SUMMON_VOIDWALKER, None);
            let pet = pets::pet_of(m, w).expect("the voidwalker");
            kit::fresh(m, w);
            spells::cast_triggered(m, w, SOUL_LINK, Some(pet));
            pass(m, 1_000);
            let mut on = vec![has(m, w, SOUL_LINK_AURA)];
            for x in [150.0, -150.0] {
                m.creature_mut(pet).unwrap().position.x += x;
                pass(m, 1_000);
                on.push(has(m, w, SOUL_LINK_AURA));
            }
            on
        })
        .await
        .unwrap();
    assert_eq!(on, vec![true, false, true], "by it, far off, back");
}
