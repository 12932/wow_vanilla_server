//! Round 8's combat tests, whose damage a creature took (Thistle Tea's
//! `damage_origin_test.exs`): the players' side and the NPCs', counted
//! whole through death and before a floor, without what was absorbed or
//! kept off, a pet's and a DoT's as their master's, kept through a fight
//! and gone with it; a raid's corpse whoever fought it; the share deciding
//! a soul shard with its killing blow; and the experience it earns.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, AURA_SCHOOL_ABSORB, AURA_SCHOOL_IMMUNITY};
use crate::game::map::combat::{self, KillXp};
use crate::game::map::creature_combat;
use crate::game::map::spells::Targets;
use crate::game::map::{Map, ai, event_ai};
use crate::game::tests_p3_combat::{self as p3, BRUTE, HELPER};

/// A raid's corpse (`static_flags1` 0x1000).
const RAID_BOSS: u32 = 3770;
/// Twenty holy damage; ten a second of shadow; a death of one's own; Drain
/// Soul's kind (a soul shard if it dies under it).
const SMITE: u32 = 96_600;
const ROT: u32 = 96_601;
const END: u32 = 96_602;
const DRAIN: u32 = 96_603;
const SOUL_SHARD: u32 = 6265;
const LONG: u32 = 963;

fn spell(id: u32, school: u32, e: SpellEffect) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        school,
        dmg_class: 1,
        range_index: 1,
        equipped_item_class: -1,
        duration_index: LONG,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        base_dice: 1,
        target_a: 6,
        bonus_coefficient: -1.0,
        ..e
    };
    s
}

fn data() -> GameData {
    let mut d = p3::data();
    let mut boss = p3::template(RAID_BOSS, 7);
    boss.static_flags1 = 0x1000;
    d.creature_templates.insert(RAID_BOSS, boss);
    d.spells.durations.insert(
        LONG,
        crate::game::data::spells::SpellDuration {
            base: 600_000,
            per_level: 0,
            max: 600_000,
        },
    );
    d.items.insert(
        SOUL_SHARD,
        crate::game::data::items::ItemTemplate {
            entry: SOUL_SHARD,
            name: "Soul Shard".into(),
            stackable: 20,
            ..Default::default()
        },
    );
    for s in [
        spell(
            SMITE,
            1,
            SpellEffect {
                effect: 2,
                base_points: 19,
                ..Default::default()
            },
        ),
        spell(
            ROT,
            5,
            SpellEffect {
                effect: 6,
                aura: auras::AURA_PERIODIC_DAMAGE,
                base_points: 9,
                amplitude: 1_000,
                ..Default::default()
            },
        ),
        spell(
            END,
            0,
            SpellEffect {
                effect: crate::game::map::spells::EFFECT_INSTAKILL,
                target_a: 1,
                ..Default::default()
            },
        ),
        {
            let mut s = spell(
                DRAIN,
                5,
                SpellEffect {
                    effect: 6,
                    aura: auras::AURA_CHANNEL_DEATH_ITEM,
                    item_type: u64::from(SOUL_SHARD),
                    base_points: 0,
                    ..Default::default()
                },
            );
            s.family_name = 5;
            s
        },
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn origin(m: &Map, g: Guid) -> (u64, u64) {
    m.creature(g).map_or((0, 0), |c| {
        (c.engagement.damage_by_players, c.engagement.damage_by_npcs)
    })
}

/// A creature of `entry` before Alice, of `health`, hers.
fn creature(m: &mut Map, ga: Guid, entry: u32, health: u32) -> Guid {
    let g = p3::spawn(m, ga, entry, 20, 3.0);
    let c = m.creature_mut(g).unwrap();
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, health);
    c.fields.set_u32(UNIT_FIELD_HEALTH, health);
    c.tapped_by = Some(ga);
    g
}

/// An NPC on Alice's side, to hurt the creatures she fights.
fn npc(m: &mut Map, ga: Guid) -> Guid {
    p3::spawn(m, ga, HELPER, 20, 6.0)
}

fn cast(m: &mut Map, caster: Guid, spell: u32, target: Guid) {
    crate::game::map::spells::cast_triggered_with(
        m,
        caster,
        spell,
        Targets {
            unit: Some(target),
            ..Default::default()
        },
    );
}

/// Thistle Tea's "raid corpses bypass contribution restrictions": a
/// creature whose corpse is the raid's is lootable and worth its whole
/// experience, though the NPCs did nearly all its harm.
#[tokio::test]
async fn a_raids_corpse_is_lootable_and_worth_its_all_whoever_fought_it() {
    let (_realm, _a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let boss = creature(m, ga, RAID_BOSS, 1_000);
            let brute = creature(m, ga, BRUTE, 1_000);
            let helper = npc(m, ga);
            let mut out = Vec::new();
            for g in [boss, brute] {
                creature_combat::damaged(m, g, helper, 900);
                creature_combat::damaged(m, g, ga, 10);
                out.push((
                    creature_combat::loot_allowed(m, g),
                    creature_combat::xp_share(m, g),
                ));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(r, vec![(true, 1.0), (false, 0.0)], "(the boss, a brute)");
}

/// Thistle Tea's "retains overkill and healing history through death",
/// "counts damage before a scripted health threshold" and "excludes
/// absorption and immunity before counting damage": what a creature took
/// is counted whole, healed or not, the killing blow's overkill too, and
/// nothing a corpse takes; a blow a floor of health holds back counts
/// whole; what a ward soaks and what an immunity keeps off don't count.
#[tokio::test]
async fn damage_counts_whole_through_death_and_floors_but_not_what_is_soaked_or_kept_off() {
    let (_realm, _a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let helper = npc(m, ga);
            // Forty of Alice's, healed; then a hundred of the NPC's kills
            // it; five hundred more on the corpse.
            let brute = creature(m, ga, BRUTE, 100);
            combat::deal_damage(m, ga, brute, 40);
            m.unit_fields_mut(brute)
                .unwrap()
                .set_u32(UNIT_FIELD_HEALTH, 100);
            combat::deal_damage(m, helper, brute, 100);
            let died = (origin(m, brute), m.unit_view(brute).unwrap().alive);
            combat::deal_damage(m, ga, brute, 500);
            let after = (origin(m, brute), creature_combat::loot_allowed(m, brute));
            // Two hundred on a floor of ten.
            let floored = creature(m, ga, BRUTE, 100);
            event_ai::scripted_mut(m, floored).unwrap().invincible_hp = Some(10);
            combat::deal_damage(m, ga, floored, 200);
            let floor = (
                m.unit_fields(floored).unwrap().get_u32(UNIT_FIELD_HEALTH),
                origin(m, floored),
            );
            // A holy ward of thirty: twenty soaked, twenty with ten through;
            // then immune to holy.
            let warded = creature(m, ga, BRUTE, 1_000);
            p3::aura_of(m, warded, 96_610, AURA_SCHOOL_ABSORB, 2, 30);
            cast(m, ga, SMITE, warded);
            let soaked = origin(m, warded);
            cast(m, ga, SMITE, warded);
            let through = origin(m, warded);
            p3::aura_of(m, warded, 96_611, AURA_SCHOOL_IMMUNITY, 2, 0);
            cast(m, ga, SMITE, warded);
            (died, after, floor, (soaked, through, origin(m, warded)))
        })
        .await
        .unwrap();
    assert_eq!(r.0, ((40, 100), false), "forty and the whole hundred");
    assert_eq!(
        r.1,
        ((40, 100), false),
        "nothing on the corpse; not lootable"
    );
    assert_eq!(r.2, (10, (200, 0)), "held at ten, the two hundred counted");
    assert_eq!(
        r.3,
        ((0, 0), (10, 0), (10, 0)),
        "(soaked, ten through, kept off)"
    );
}

/// Thistle Tea's "attributes controlled attackers and self damage" and
/// "periodic damage retains the caster's controlling owner": a player's
/// pet's blow and its DoT count as hers, a creature's harm to itself as
/// the players', an NPC's as the NPCs'.
#[tokio::test]
async fn a_pets_harm_and_its_dots_are_its_masters_and_a_creatures_own_the_players() {
    let (_realm, _a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = creature(m, ga, BRUTE, 10_000);
            let helper = npc(m, ga);
            // A creature of Alice's: her pet, as far as blame goes.
            let pet = p3::spawn(m, ga, HELPER, 20, 5.0);
            m.creature_mut(pet).unwrap().owner = Some(ga);
            combat::deal_damage(m, pet, brute, 10);
            combat::deal_damage(m, brute, brute, 10);
            combat::deal_damage(m, helper, brute, 10);
            let blows = origin(m, brute);
            // The pet's DoT, a tick of it.
            cast(m, pet, ROT, brute);
            crate::game::tests_p3_spells_kit::tick_now(m, brute, ROT);
            (blows, origin(m, brute))
        })
        .await
        .unwrap();
    assert_eq!(r.0, (20, 10), "the pet's and its own the players'");
    assert_eq!(r.1, (30, 10), "and the pet's tick");
}

/// Thistle Tea's "preserves contributions across victim changes and clears
/// them on reset" and "scripted self kills preserve player-side damage":
/// the count lasts while the fight does, the creature turning on another
/// or letting its victim go, and is gone when it evades (a respawn is a
/// creature made anew, `ai::respawn`); a creature that kills itself does so
/// on the players' side, its tap kept.
#[tokio::test]
async fn the_count_lasts_the_fight_and_a_creatures_own_death_is_the_players() {
    let (_realm, _a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            let brute = creature(m, ga, BRUTE, 10_000);
            let helper = npc(m, ga);
            ai::attacked(m, brute, ga);
            combat::deal_damage(m, ga, brute, 20);
            combat::deal_damage(m, helper, brute, 30);
            combat::stop_attack(m, brute);
            let stopped = origin(m, brute);
            ai::attacked(m, brute, helper);
            let turned = origin(m, brute);
            ai::enter_evade(m, brute);
            let evaded = origin(m, brute);
            // Its own end, Alice's tap on it.
            let doomed = creature(m, ga, BRUTE, 100);
            cast(m, doomed, END, doomed);
            let c = m.creature(doomed).unwrap();
            let ended = (
                c.alive(),
                origin(m, doomed),
                c.tapped_by,
                creature_combat::loot_allowed(m, doomed),
            );
            (stopped, turned, evaded, ended)
        })
        .await
        .unwrap();
    assert_eq!(r.0, (20, 30), "kept as it lets its victim go");
    assert_eq!(r.1, (20, 30), "and as it turns on another");
    assert_eq!(r.2, (0, 0), "gone as it evades");
    assert_eq!(
        r.3,
        (false, (100, 0), Some(ga), true),
        "its own death the players', Alice's tap kept"
    );
}

/// Thistle Tea's "captures the lethal contribution before shard
/// eligibility": a creature under a warlock's drain, the NPCs' 65 before
/// her killing 35, is mostly theirs, and leaves her no soul shard and no
/// loot; 64 and 36, hers, and a shard.
#[tokio::test]
async fn the_killing_blows_share_decides_a_soul_shard() {
    let realm = crate::game::harness::realm_with(data());
    let mut w = crate::game::harness::Client::connect(&realm, "W");
    let gw = w.enter("Wanda", Race::Human, Class::Warlock).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let r = map
        .query(move |m| {
            p3::level(m, gw, 20);
            let mut out = Vec::new();
            for (npcs, hers) in [(65, 35), (64, 36)] {
                let brute = creature(m, gw, BRUTE, 100);
                let helper = npc(m, gw);
                combat::deal_damage(m, helper, brute, npcs);
                let s = m.realm.data.spells.get(DRAIN).cloned().unwrap();
                auras::apply_spell(m, gw, brute, &s, &[(0, 1)], Default::default()).unwrap();
                let shards = |m: &Map| {
                    m.players[&gw]
                        .items
                        .iter()
                        .filter(|i| i.entry == SOUL_SHARD)
                        .map(|i| i.count())
                        .sum::<u32>()
                };
                let before = shards(m);
                let d = crate::game::map::spells::outcome::Damage {
                    amount: hers,
                    resisted: 0,
                    blocked: 0,
                };
                crate::game::map::spells::outcome::deal(
                    m,
                    gw,
                    brute,
                    &s,
                    d,
                    &crate::game::map::spells::outcome::Landing::default(),
                );
                out.push((
                    m.unit_view(brute).unwrap().alive,
                    shards(m) - before,
                    creature_combat::loot_allowed(m, brute),
                ));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        vec![(false, 0, false), (false, 1, true)],
        "(alive, shards, lootable): 35 of 100, then 36"
    );
}

/// Thistle Tea's "rounds assisted rewards to the nearest even integer": a
/// kill's experience by the players' share, a half rounded to the even.
#[test]
fn a_kills_experience_by_the_players_share_rounds_half_to_even() {
    let xp = |level: u32, share: f32| {
        combat::kill_xp_with(
            level,
            level,
            level,
            &KillXp {
                damage_share: share,
                ..Default::default()
            },
        )
    };
    assert_eq!(xp(10, 0.5), 48, "47.5");
    assert_eq!(xp(12, 0.5), 52, "52.5");
    assert_eq!(xp(10, 0.36), 34);
    assert_eq!(xp(10, 0.0), 0);
}
