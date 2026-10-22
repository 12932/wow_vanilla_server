//! A spell that kills, as its caster's client is told of it: the go naming
//! the victim among its hits before the damage, the death (its health at
//! 0 in a values block) after both, and the corpse left where it fell, a
//! critter's as a beast's (mangos-classic's `Spell::cast`: `SendSpellGo`,
//! then `handle_immediate`'s `DoAllEffectOnTarget`, whose damage log and
//! `Unit::Kill` come before the map's object updates carry the death).
//!
//! The 1.12.1 client plays a spell's impact from the go alone: for a spell
//! of no speed, its `SMSG_SPELL_GO` handler (`0x6E7A70`) puts the impact
//! kit of the spell's visual on every unit the go lists as hit, then and
//! there (`0x6E8BF0`), whatever the unit's state and whatever the go's
//! cast flags. A kill's impact is the client's to show once the go names
//! the victim, alive and known to it, as these tests check it does.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_pets::{cast, learn};

/// A critter (a rat's like) and a beast, at level 1.
const CRITTER: u32 = 3790;
const BEAST: u32 = 3791;
/// A holy bolt of no speed, with a cast time: Smite's shape.
const BOLT: u32 = 95_790;
const BOLT_CAST_TIME: u32 = 90;

/// What a client was told of a kill, by the place of each packet among
/// those it got.
#[derive(Debug, Default)]
struct Told {
    /// The go of the spell, and whether it lists the victim as hit.
    go: Option<(usize, bool)>,
    damage: Option<usize>,
    /// The first values block with the victim's health at 0.
    death: Option<usize>,
    /// The victim forgotten or destroyed.
    gone: Option<usize>,
}

/// What `c` was told of `caster`'s `spell` at `victim`.
fn told(c: &mut Client, caster: Guid, spell: u32, victim: Guid) -> Told {
    let mut t = Told::default();
    let sent: Vec<ServerOpcodeMessage> = c
        .captured()
        .iter()
        .filter_map(|f| ServerOpcodeMessage::read_unencrypted(&f[..]).ok())
        .collect();
    for (i, m) in sent.iter().enumerate() {
        match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(g)
                if g.caster == caster && g.spell == spell && t.go.is_none() =>
            {
                t.go = Some((i, g.hits.contains(&victim)));
            }
            ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(d)
                if d.attacker == caster
                    && d.spell == spell
                    && d.target == victim
                    && t.damage.is_none() =>
            {
                t.damage = Some(i);
            }
            ServerOpcodeMessage::SMSG_DESTROY_OBJECT(d) if d.guid == victim => {
                t.gone.get_or_insert(i);
            }
            ServerOpcodeMessage::SMSG_UPDATE_OBJECT(_)
            | ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(_) => {
                if harness::forgotten(m).contains(&victim) {
                    t.gone.get_or_insert(i);
                }
                let dies = harness::values(m).iter().any(|(g, mask)| {
                    *g == victim
                        && matches!(mask, UpdateMask::Unit(u) if u.unit_health() == Some(0))
                });
                if dies {
                    t.death.get_or_insert(i);
                }
            }
            _ => {}
        }
    }
    t
}

/// The kill as it should be told: the go naming the victim as hit first,
/// then its damage, then its death, and nothing forgetting it.
fn assert_told_in_order(t: &Told, what: &str) {
    let Some((go, hit)) = t.go else {
        panic!("{what}: no go: {t:?}");
    };
    assert!(hit, "{what}: the go lists the victim as hit: {t:?}");
    let damage = t
        .damage
        .unwrap_or_else(|| panic!("{what}: no damage: {t:?}"));
    let death = t.death.unwrap_or_else(|| panic!("{what}: no death: {t:?}"));
    assert!(go < damage, "{what}: the go before the damage: {t:?}");
    assert!(damage < death, "{what}: the damage before the death: {t:?}");
    assert_eq!(t.gone, None, "{what}: the corpse is kept: {t:?}");
}

/// Wait for `n` of the map's updates.
async fn updates(map: &MapHandle, n: u64) {
    let from = map.query(|m| m.stats.updates).await.unwrap();
    for _ in 0..2000 {
        if map.query(|m| m.stats.updates).await.unwrap() >= from + n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("the map ran no {n} updates");
}

/// Wait until `victim` is dead on the map.
async fn dead(map: &MapHandle, victim: Guid) {
    for _ in 0..2000 {
        let dead = map
            .query(move |m| m.creature(victim).is_some_and(|c| !c.alive()))
            .await
            .unwrap();
        if dead {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("{victim} never died");
}

/// Whether the map still has `victim` dead, known to `g` (its corpse).
async fn corpse_known(map: &MapHandle, g: Guid, victim: Guid) -> bool {
    map.query(move |m: &mut Map| {
        m.creature(victim).is_some_and(|c| !c.alive()) && m.players[&g].known.contains(&victim)
    })
    .await
    .unwrap()
}

fn world() -> GameData {
    use crate::game::tests_p3_combat::{data, template};
    let mut d = data();
    for (entry, kind) in [
        (CRITTER, crate::game::map::combat::CREATURE_TYPE_CRITTER),
        (BEAST, 1),
    ] {
        let mut t = template(entry, kind);
        t.level_min = 1;
        t.level_max = 1;
        d.creature_templates.insert(entry, t);
    }
    d.spells.cast_times.insert(BOLT_CAST_TIME, 200);
    let mut bolt = crate::game::tests_p3_entity::spell(
        BOLT,
        &[SpellEffect {
            effect: 2,
            base_points: 499,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        }],
    );
    bolt.school = 1;
    bolt.casting_time_index = BOLT_CAST_TIME;
    bolt.spell_visual = 128;
    d.spells.spells.insert(BOLT, bolt);
    d
}

/// A priest's bolt kills a level 1 creature of `entry` at a blow: what her
/// client is told, in order, with its corpse kept a while after.
async fn bolt_kills(entry: u32) {
    let (_realm, mut a, ga, map) =
        crate::game::tests_p3_combat::setup_with(world(), Class::Priest).await;
    learn(&map, ga, &[BOLT]);
    let victim = map
        .query(move |m| {
            let g = crate::game::tests_p3_combat::spawn(m, ga, entry, 1, 10.0);
            let c = m.creature_mut(g).unwrap();
            c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 8);
            c.fields.set_u32(UNIT_FIELD_HEALTH, 8);
            g
        })
        .await
        .unwrap();
    a.sees(victim).await;
    a.clear();
    cast(&a, BOLT, Some(victim)).await;
    dead(&map, victim).await;
    // Its corpse lies a while: some updates on, it is still known.
    updates(&map, 10).await;
    assert!(corpse_known(&map, ga, victim).await, "its corpse is kept");
    assert_told_in_order(&told(&mut a, ga, BOLT, victim), "the bolt");
}

#[tokio::test]
async fn a_bolt_that_kills_a_critter_is_told_hit_before_the_death_and_leaves_its_corpse() {
    bolt_kills(CRITTER).await;
}

#[tokio::test]
async fn a_bolt_that_kills_a_beast_is_told_hit_before_the_death_and_leaves_its_corpse() {
    bolt_kills(BEAST).await;
}

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

/// Smite, rank 8: a level 60 priest's.
const SMITE: u32 = 10934;
const RAT: u32 = 4075;
const YOUNG_WOLF: u32 = 299;

/// The user's case: a level 60 priest's Smite on a level 1 creature of
/// `entry`, in the real world, its packets run through the client check
/// (`tools/client_check`) too. A Smite resisted (one in a hundred at
/// least) is cast again at another.
async fn smite_kills(entry: u32, account: &str) {
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let (mut a, ga, map) =
        crate::game::tests_pets::enter(&realm, account, "Priest", Race::Human, Class::Priest).await;
    // Past the intro's camera, back in her own eyes.
    a.send(CMSG_COMPLETE_CINEMATIC {}).await;
    map.query(move |m: &mut Map| {
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&ga).unwrap();
        crate::game::map::stats::set_level(p, &data, 60);
    })
    .await
    .unwrap();
    learn(&map, ga, &[SMITE]);
    for attempt in 0..5 {
        let victim =
            crate::game::tests_pets::place(&map, ga, entry, 0x70_0901 + attempt, 10.0).await;
        a.sees_on(&map, victim).await;
        a.clear();
        cast(&a, SMITE, Some(victim)).await;
        let landed = a
            .wait_for("Smite's go", |m| match m {
                ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.caster == ga && g.spell == SMITE => {
                    Some(g.hits.contains(&victim))
                }
                _ => None,
            })
            .await;
        if !landed {
            continue;
        }
        dead(&map, victim).await;
        updates(&map, 20).await;
        assert!(corpse_known(&map, ga, victim).await, "its corpse is kept");
        assert_told_in_order(&told(&mut a, ga, SMITE, victim), "Smite");
        // And what the kill sent passes the client's own parsers.
        let (check_py, wow_exe) = crate::game::tests_client_emu::checker()
            .expect("the client-emulation check: see docs/TESTING.md");
        let (capture, _) = crate::game::tests_client_emu::capture(vec![&mut a]);
        crate::game::tests_client_emu::run_checker(&check_py, &wow_exe, &capture);
        return;
    }
    panic!("five Smites resisted");
}

#[tokio::test]
#[ignore = "extended"]
async fn a_smite_that_kills_a_rat_is_told_hit_before_the_death_and_leaves_its_corpse() {
    smite_kills(RAT, "KVRAT").await;
}

#[tokio::test]
#[ignore = "extended"]
async fn a_smite_that_kills_a_young_wolf_is_told_hit_before_the_death_and_leaves_its_corpse() {
    smite_kills(YOUNG_WOLF, "KVWOLF").await;
}
