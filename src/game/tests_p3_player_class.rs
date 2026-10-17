//! Phase 3 against Thistle Tea's class tests (`test/game/core/class`): the
//! real spells of each class, cast and checked on the map. These read the
//! real data (`WVS_DATA`), and pass without running where it isn't.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells::{self, Targets, checks::fail, mods};
use crate::game::map::{Map, MapKey, auras, derive};
use crate::game::store::BAG_PLAYER;

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

fn realm() -> Arc<Realm> {
    harness::realm_with(crate::game::data::load::load(DB.as_path()))
}

/// A creature of `entry` at `level`, `yards` in front of `g`, hardy and
/// without armor, facing them.
fn spawn(m: &mut Map, g: Guid, entry: u32, level: u8, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let template = data
        .creature_template(entry)
        .cloned()
        .expect("the creature");
    let p = &m.players[&g];
    let (pos, o) = (p.position(), p.movement.orientation);
    let at = Vector3d {
        x: pos.x + yards * o.cos(),
        y: pos.y + yards * o.sin(),
        z: pos.z,
    };
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let mut c = Creature::new(
        guid::unit(entry, counter | 0x80_0000),
        &template,
        level,
        at,
        o + std::f32::consts::PI,
    );
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
    c.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
    c.fields.set_u32(UNIT_FIELD_RESISTANCES, 0);
    let c = c.with_base();
    m.add_creature(c)
}

/// At `level`, knowing `learn`, with power to spare.
fn ready(m: &mut Map, g: Guid, level: u8, learn: &[u32]) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, level);
    for s in learn {
        spells::learn(m, g, *s);
    }
    refill(m, g);
}

fn refill(m: &mut Map, g: Guid) {
    let p = m.players.get_mut(&g).unwrap();
    for i in 0..5 {
        p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + i, 100_000);
        p.fields.set_u32(UNIT_FIELD_POWER1 + i, 100_000);
    }
    // No cooldown, global or other, in the way.
    p.caster.cooldowns.clear();
    p.caster.category_cooldowns.clear();
    p.caster.gcd_until = None;
}

/// Why `g` can't cast `spell` at `target` now, if it can't.
fn check(m: &mut Map, g: Guid, spell: u32, target: Guid) -> Option<u8> {
    refill(m, g);
    let data = m.realm.data.clone();
    let s = data.spells.get(spell).unwrap();
    let t = Targets {
        unit: Some(target),
        ..Default::default()
    };
    let mods = mods::for_spell(m, g, s);
    spells::checks::check(m, g, s, &t, None, false, &mods)
}

/// Cast `spell` at `target` with nothing in the way: whether it went.
fn cast(m: &mut Map, g: Guid, spell: u32, target: Guid) -> bool {
    refill(m, g);
    spells::cast(
        m,
        g,
        spell,
        Targets {
            unit: Some(target),
            ..Default::default()
        },
        None,
    )
}

fn holder(m: &Map, g: Guid, spell: u32) -> Option<auras::Holder> {
    auras::auras(m, g).and_then(|a| a.holders.iter().find(|h| h.spell == spell).cloned())
}

/// Equip a new item of `entry` in `slot`.
fn equip(m: &mut Map, g: Guid, entry: u32, slot: u8) {
    let data = m.realm.data.clone();
    let counter = m
        .realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = m.players.get_mut(&g).expect("the player");
    p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot == slot));
    p.items.push(crate::game::map::player::Item::new(
        &data,
        guid::item(counter),
        g,
        entry,
        BAG_PLAYER,
        slot,
        1,
    ));
    p.refresh_inventory(&data);
    crate::game::map::inventory::sync_gear(m, g);
    derive::unit(m, g);
}

fn unequip(m: &mut Map, g: Guid, slot: u8) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).expect("the player");
    p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot == slot));
    p.refresh_inventory(&data);
    crate::game::map::inventory::sync_gear(m, g);
    derive::unit(m, g);
}

fn form(m: &Map, g: Guid) -> u8 {
    m.players[&g].fields.get_byte(UNIT_FIELD_BYTES_1, 2)
}

const BATTLE_STANCE: u32 = 2457;
const DEFENSIVE_STANCE: u32 = 71;
const BERSERKER_STANCE: u32 = 2458;
const HAMSTRING: u32 = 1715;
const SHIELD_BASH: u32 = 72;
const PUMMEL: u32 = 6552;
const EXECUTE: u32 = 5308;
const BLOODRAGE: u32 = 2687;
const SUNDER_ARMOR: u32 = 7386;
const RECKLESSNESS: u32 = 1719;
const SHIELD_BLOCK: u32 = 2565;
const DISARM: u32 = 676;
const DEATH_WISH: u32 = 12328;
const DEMORALIZING_SHOUT: u32 = 1160;
const LAST_STAND: u32 = 12975;
/// A Worn Shortsword and a Worn Buckler.
const SWORD: u32 = 25;
const BUCKLER: u32 = 2129;
/// A Kobold Vermin.
const KOBOLD: u32 = 6;

#[tokio::test]
async fn a_warriors_stances_set_the_form_one_at_a_time_and_lock_their_abilities_and_gear_to_them() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "W");
    let w = c.enter("Warrior", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let got = map
        .query(move |m| {
            ready(
                m,
                w,
                60,
                &[
                    BATTLE_STANCE,
                    DEFENSIVE_STANCE,
                    BERSERKER_STANCE,
                    HAMSTRING,
                    SHIELD_BASH,
                    PUMMEL,
                ],
            );
            let kobold = spawn(m, w, KOBOLD, 60, 2.0);
            equip(m, w, SWORD, 15);
            let mut out = Vec::new();
            // No stance: Hamstring wants battle or berserker.
            out.push(("none", form(m, w), check(m, w, HAMSTRING, kobold)));
            cast(m, w, BATTLE_STANCE, w);
            out.push(("battle", form(m, w), check(m, w, HAMSTRING, kobold)));
            // Defensive replaces battle; Hamstring is shut, Shield Bash
            // wants a shield, then has one.
            cast(m, w, DEFENSIVE_STANCE, w);
            let replaced = holder(m, w, BATTLE_STANCE).is_none();
            out.push(("defensive", form(m, w), check(m, w, HAMSTRING, kobold)));
            // The Worn Wooden Shield off.
            unequip(m, w, 16);
            out.push(("bash bare", form(m, w), check(m, w, SHIELD_BASH, kobold)));
            equip(m, w, BUCKLER, 16);
            out.push(("bash shield", form(m, w), check(m, w, SHIELD_BASH, kobold)));
            // Berserker: Hamstring again (either of two), Pummel (berserker
            // only), and no Shield Bash.
            cast(m, w, BERSERKER_STANCE, w);
            out.push(("berserker", form(m, w), check(m, w, HAMSTRING, kobold)));
            out.push(("pummel", form(m, w), check(m, w, PUMMEL, kobold)));
            out.push(("bash", form(m, w), check(m, w, SHIELD_BASH, kobold)));
            // A weapon spell with no weapon: refused.
            unequip(m, w, 15);
            out.push(("unarmed", form(m, w), check(m, w, HAMSTRING, kobold)));
            // The stance gone, the form byte with it.
            auras::remove_spell(m, w, BERSERKER_STANCE, None);
            out.push(("gone", form(m, w), None));
            (replaced, out)
        })
        .await
        .unwrap();
    let (replaced, steps) = got;
    assert!(replaced, "one stance at a time");
    let want: Vec<(&str, u8, Option<u8>)> = vec![
        ("none", 0, Some(fail::ONLY_SHAPESHIFT)),
        ("battle", 17, None),
        ("defensive", 18, Some(fail::ONLY_SHAPESHIFT)),
        ("bash bare", 18, Some(fail::EQUIPPED_ITEM_CLASS)),
        ("bash shield", 18, None),
        ("berserker", 19, None),
        ("pummel", 19, None),
        ("bash", 19, Some(fail::ONLY_SHAPESHIFT)),
        // Thistle Tea and mangos answer EQUIPPED_ITEM_CLASS; wvs's own.
        ("unarmed", 19, Some(fail::MAINHAND_EMPTY)),
        ("gone", 0, None),
    ];
    assert_eq!(steps, want);
}

#[tokio::test]
async fn a_warriors_execute_bloodrage_sunder_shouts_and_cooldowns_do_what_they_say() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "W");
    let w = c.enter("Warrior", Race::Human, Class::Warrior).await;
    let mut d = Client::connect(&realm, "V");
    let v = d.enter("Victim", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let got = map
        .query(move |m| {
            ready(
                m,
                w,
                60,
                &[
                    BATTLE_STANCE,
                    DEFENSIVE_STANCE,
                    BERSERKER_STANCE,
                    EXECUTE,
                    BLOODRAGE,
                    SUNDER_ARMOR,
                    RECKLESSNESS,
                    SHIELD_BLOCK,
                    DISARM,
                    DEATH_WISH,
                    DEMORALIZING_SHOUT,
                    LAST_STAND,
                    crate::game::map::derive::SPELL_BLOCK,
                ],
            );
            equip(m, w, SWORD, 15);
            let kobold = spawn(m, w, KOBOLD, 60, 2.0);
            cast(m, w, BATTLE_STANCE, w);
            // Execute: only under a fifth of its health.
            let half = check(m, w, EXECUTE, kobold);
            let k = m.creature_mut(kobold).unwrap();
            k.fields.set_u32(UNIT_FIELD_HEALTH, 100_000);
            let low = check(m, w, EXECUTE, kobold);
            // Bloodrage: a fight begins.
            let calm = m.players[&w].fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            );
            cast(m, w, BLOODRAGE, w);
            let fighting = m.players[&w].fields.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            );
            // Sunder Armor: five stacks at most, in a slot the client
            // shows, each more armor off.
            let other = spawn(m, w, KOBOLD, 60, 2.5);
            m.creature_mut(other)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_RESISTANCES, 3_000);
            if let Some(c) = m.creature_mut(other) {
                c.base.resistances[0] = 3_000;
            }
            derive::unit(m, other);
            let mut armor = Vec::new();
            for _ in 0..6 {
                cast(m, w, SUNDER_ARMOR, other);
                armor.push(
                    m.creature(other)
                        .unwrap()
                        .fields
                        .get_u32(UNIT_FIELD_RESISTANCES),
                );
            }
            let sunder = holder(m, other, SUNDER_ARMOR).map(|h| (h.stacks, h.slot.is_some()));
            // Demoralizing Shout, on those about: a slot of its own.
            cast(m, w, DEMORALIZING_SHOUT, w);
            let shout = holder(m, kobold, DEMORALIZING_SHOUT).map(|h| h.slot.is_some());
            // Berserker's Recklessness: a hundred more crit.
            cast(m, w, BERSERKER_STANCE, w);
            let crit = m.players[&w].fields.get_f32(PLAYER_CRIT_PERCENTAGE);
            cast(m, w, RECKLESSNESS, w);
            let reckless = m.players[&w].fields.get_f32(PLAYER_CRIT_PERCENTAGE) - crit;
            // Death Wish: a fifth more physical damage.
            let min = m.players[&w].fields.get_f32(UNIT_FIELD_MINDAMAGE);
            cast(m, w, DEATH_WISH, w);
            let wish = m.players[&w].fields.get_f32(UNIT_FIELD_MINDAMAGE) / min;
            // Defensive: Shield Block with a shield; Disarm on a warrior
            // with a sword leaves him unarmed and flagged.
            cast(m, w, DEFENSIVE_STANCE, w);
            equip(m, w, BUCKLER, 16);
            let block = m.players[&w].fields.get_f32(PLAYER_BLOCK_PERCENTAGE);
            cast(m, w, SHIELD_BLOCK, w);
            let blocking = m.players[&w].fields.get_f32(PLAYER_BLOCK_PERCENTAGE) - block;
            equip(m, v, SWORD, 15);
            let armed = m.players[&v].fields.get_f32(UNIT_FIELD_MAXDAMAGE);
            let pos = m.players[&w].position();
            m.players.get_mut(&v).unwrap().movement.position = Vector3d {
                x: pos.x + 1.0,
                ..pos
            };
            // On a creature, cast; on a warrior (a friend, so put on him),
            // his sword's damage goes.
            let disarmed = cast(m, w, DISARM, kobold);
            let flagged = m
                .creature(kobold)
                .unwrap()
                .fields
                .has_flag(UNIT_FIELD_FLAGS, auras::UNIT_FLAG_DISARMED);
            let s = m.realm.data.spells.get(DISARM).unwrap().clone();
            auras::apply_spell(m, w, v, &s, &[(0, 0)], Default::default()).unwrap();
            derive::unit(m, v);
            let unarmed = m.players[&v].fields.get_f32(UNIT_FIELD_MAXDAMAGE);
            // Last Stand: three tenths more health, and that much healed.
            let (max, health) = {
                let f = &m.players[&w].fields;
                (
                    f.get_u32(UNIT_FIELD_MAXHEALTH),
                    f.get_u32(UNIT_FIELD_HEALTH),
                )
            };
            cast(m, w, LAST_STAND, w);
            let (max2, health2) = {
                let f = &m.players[&w].fields;
                (
                    f.get_u32(UNIT_FIELD_MAXHEALTH),
                    f.get_u32(UNIT_FIELD_HEALTH),
                )
            };
            (
                (half, low),
                (calm, fighting),
                (armor, sunder, shout),
                (reckless, wish, blocking),
                (disarmed, flagged, armed, unarmed),
                (max, health, max2, health2),
            )
        })
        .await
        .unwrap();
    let (
        (half, low),
        (calm, fighting),
        (armor, sunder, shout),
        (reckless, wish, blocking),
        disarm,
        last,
    ) = got;
    assert_eq!((half, low), (Some(fail::TARGET_AURASTATE), None));
    assert!(!calm && fighting, "Bloodrage starts a fight");
    assert!(
        armor.windows(2).take(4).all(|w| w[1] < w[0]) && armor[4] == armor[5],
        "{armor:?}"
    );
    assert_eq!(sunder, Some((5, true)));
    assert_eq!(shout, Some(true));
    assert!((reckless - 100.0).abs() < 0.01, "{reckless}");
    assert!((wish - 1.2).abs() < 0.01, "{wish}");
    assert!((blocking - 75.0).abs() < 0.01, "{blocking}");
    let (disarmed, flagged, armed, unarmed) = disarm;
    assert!(disarmed && flagged && unarmed < armed, "{disarm:?}");
    let (max, health, max2, health2) = last;
    assert_eq!(max2, max + max * 3 / 10);
    assert_eq!(health2, health + max * 3 / 10);
}

const POWER_WORD_SHIELD: u32 = 17;
const FEAR_WARD: u32 = 6346;
const FEAR: u32 = 5782;
const SHACKLE_UNDEAD: u32 = 9484;
const MANA_BURN: u32 = 8129;
const LEVITATE: u32 = 1706;
const SHADOW_PROTECTION: u32 = 976;
const DISPEL_MAGIC: u32 = 527;
const MIND_SOOTHE: u32 = 453;
const SHADOW_WORD_PAIN: u32 = 589;
const FORTITUDE: u32 = 1243;
const ZOMBIE: u32 = 1501;

fn feared(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, crate::game::map::auras::UNIT_FLAG_FLEEING))
}

#[tokio::test]
async fn a_priests_shield_wards_shackle_mana_burn_levitate_protection_and_dispels() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "P");
    let p = c.enter("Priest", Race::Human, Class::Priest).await;
    let mut d = Client::connect(&realm, "F");
    let friend = d.enter("Friend", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let got = map
        .query(move |m| {
            ready(
                m,
                p,
                60,
                &[
                    POWER_WORD_SHIELD,
                    FEAR_WARD,
                    SHACKLE_UNDEAD,
                    MANA_BURN,
                    LEVITATE,
                    SHADOW_PROTECTION,
                    DISPEL_MAGIC,
                    MIND_SOOTHE,
                    SHADOW_WORD_PAIN,
                    FORTITUDE,
                ],
            );
            let kobold = spawn(m, p, KOBOLD, 60, 3.0);
            // The shield takes any school's damage.
            spells::cast_triggered(m, p, POWER_WORD_SHIELD, Some(p));
            let shield: Vec<(u32, u32)> = [1, 4, 32]
                .into_iter()
                .map(|school| auras::absorb(m, p, 10, school))
                .collect();
            // Fear Ward: a fear blocked (wvs keeps the ward on: its charge
            // isn't spent); a fear already on stays when a ward comes.
            spells::cast_triggered(m, p, FEAR_WARD, Some(p));
            spells::cast_triggered(m, kobold, FEAR, Some(p));
            let warded = feared(m, p);
            let pos = m.players[&p].position();
            m.players.get_mut(&friend).unwrap().movement.position = Vector3d {
                x: pos.x + 1.0,
                ..pos
            };
            spells::cast_triggered(m, kobold, FEAR, Some(friend));
            let then_feared = feared(m, friend);
            spells::cast_triggered(m, p, FEAR_WARD, Some(friend));
            let still = feared(m, friend);
            auras::remove_spell(m, friend, FEAR, None);
            auras::remove_spell(m, friend, FEAR_WARD, None);
            // Shackle: stunned and rooted, and free of both after.
            let zombie = spawn(m, p, ZOMBIE, 60, 4.0);
            spells::cast_triggered(m, p, SHACKLE_UNDEAD, Some(zombie));
            let stun_flag = |m: &Map| {
                m.unit_fields(zombie).is_some_and(|f| {
                    f.has_flag(
                        UNIT_FIELD_FLAGS,
                        crate::game::map::player::UNIT_FLAG_STUNNED,
                    )
                })
            };
            let shackled = stun_flag(m);
            auras::remove_spell(m, zombie, SHACKLE_UNDEAD, None);
            let freed = !stun_flag(m);
            // Mana Burn: half the mana burnt, as damage; only what is
            // there; nothing from one without.
            let burn = |m: &mut Map, mana: u32| {
                let target = spawn(m, p, KOBOLD, 60, 3.0);
                let f = &mut m.creature_mut(target).unwrap().fields;
                f.set_u32(UNIT_FIELD_MAXPOWER1, 5_000);
                f.set_u32(UNIT_FIELD_POWER1, mana);
                spells::cast_triggered(m, p, MANA_BURN, Some(target));
                let f = &m.creature(target).unwrap().fields;
                (
                    mana - f.get_u32(UNIT_FIELD_POWER1),
                    1_000_000 - f.get_u32(UNIT_FIELD_HEALTH),
                )
            };
            let burns = [burn(m, 5_000), burn(m, 50), burn(m, 0)];
            // Shadow Protection: shadow resistance.
            let shadow = m.players[&p].fields.get_u32(UNIT_FIELD_RESISTANCES + 5);
            spells::cast_triggered(m, p, SHADOW_PROTECTION, Some(p));
            let protected = m.players[&p].fields.get_u32(UNIT_FIELD_RESISTANCES + 5) - shadow;
            // Dispel Magic on a foe takes its buff, not the caster's own
            // debuff; on a friend, the debuff, not the buff.
            spells::cast_triggered(m, kobold, FORTITUDE, Some(kobold));
            spells::cast_triggered(m, p, SHADOW_WORD_PAIN, Some(kobold));
            spells::cast_triggered(m, p, DISPEL_MAGIC, Some(kobold));
            let offensive = (
                holder(m, kobold, FORTITUDE).is_none(),
                holder(m, kobold, SHADOW_WORD_PAIN).is_some(),
            );
            spells::cast_triggered(m, p, FORTITUDE, Some(friend));
            spells::cast_triggered(m, kobold, SHADOW_WORD_PAIN, Some(friend));
            spells::cast_triggered(m, p, DISPEL_MAGIC, Some(friend));
            let defensive = (
                holder(m, friend, FORTITUDE).is_some(),
                holder(m, friend, SHADOW_WORD_PAIN).is_none(),
            );
            // Mind Soothe: a harmful aura shortening the detection range.
            spells::cast_triggered(m, p, MIND_SOOTHE, Some(kobold));
            let soothe = holder(m, kobold, MIND_SOOTHE).map(|h| {
                (
                    h.positive,
                    h.effects
                        .iter()
                        .any(|e| e.kind == auras::AURA_MOD_DETECT_RANGE && e.amount < 0),
                )
            });
            (
                shield,
                (warded, then_feared, still),
                (shackled, freed),
                burns,
                protected,
                (offensive, defensive),
                soothe,
            )
        })
        .await
        .unwrap();
    let (shield, fear, shackle, burns, protected, dispels, soothe) = got;
    assert_eq!(
        shield,
        vec![(0, 10), (0, 10), (0, 10)],
        "every school absorbed"
    );
    assert_eq!(fear, (false, true, true));
    assert_eq!(shackle, (true, true));
    let (all, some, none) = (burns[0], burns[1], burns[2]);
    assert!(all.0 > 0 && all.1 == all.0 / 2, "{all:?}");
    assert_eq!(some, (50, 25));
    assert_eq!(none, (0, 0));
    assert!(protected > 0);
    assert_eq!(dispels, ((true, true), (true, true)));
    assert_eq!(soothe, Some((false, true)));
}

#[tokio::test]
async fn levitate_floats_its_target_and_breaks_on_damage() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "P");
    let p = c.enter("Priest", Race::Human, Class::Priest).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    map.query(move |m| {
        ready(m, p, 60, &[LEVITATE]);
        spells::cast_triggered(m, p, LEVITATE, Some(p));
    })
    .await
    .unwrap();
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage as S;
    c.wait_for("SMSG_MOVE_FEATHER_FALL", |m| {
        matches!(m, S::SMSG_MOVE_FEATHER_FALL(_)).then_some(())
    })
    .await;
    c.wait_for("SMSG_MOVE_SET_HOVER", |m| {
        matches!(m, S::SMSG_MOVE_SET_HOVER(_)).then_some(())
    })
    .await;
    c.wait_for("SMSG_MOVE_WATER_WALK", |m| {
        matches!(m, S::SMSG_MOVE_WATER_WALK(_)).then_some(())
    })
    .await;
    let gone = map
        .query(move |m| {
            let kobold = spawn(m, p, KOBOLD, 60, 3.0);
            crate::game::map::combat::deal_damage(m, kobold, p, 5);
            holder(m, p, LEVITATE).is_none()
        })
        .await
        .unwrap();
    assert!(gone, "damage breaks it");
}
