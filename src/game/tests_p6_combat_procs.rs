//! Round 6's combat tests, a weapon's own procs (Thistle Tea's
//! `weapon_procs_test.exs` and `weapon_procs_dbc_test.exs`): an item's
//! chance-on-hit spells, their chance, the blows that set them off, the
//! hand and the target they go to, and what stops them.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::{ItemSpell, ItemTemplate};
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::map::auras::{self, *};
use crate::game::map::enchant;
use crate::game::map::inventory::{SLOT_MAIN_HAND, SLOT_OFF_HAND};
use crate::game::map::procs::{self, Event};
use crate::game::map::{Map, combat};
use crate::game::tests_p3_combat::{self as base, *};

/// A point of damage to the target; another; a buff of ten strength.
const HURT: u32 = 91_100;
const HURT_OFF: u32 = 91_101;
const MIGHT: u32 = 91_102;
/// Never rolled: on use, and on equip.
const ON_USE: u32 = 91_103;
const ON_EQUIP: u32 = 91_104;
/// An extra attack; a point of damage under the spells' global cooldown.
const EXTRA: u32 = 91_105;
const COOLED: u32 = 91_106;
/// A main-hand weapon with every kind of slot; an off-hand one; one of
/// extra attacks; one under a global cooldown.
const BLADE: u32 = 63_610;
const OFF_BLADE: u32 = 63_611;
const EXTRA_BLADE: u32 = 63_612;
const COOLED_BLADE: u32 = 63_613;

fn damage(id: u32) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Hurt {id}"),
        range_index: 1,
        equipped_item_class: -1,
        proc_chance: 50,
        ..Default::default()
    };
    s.effects[0] = SpellEffect {
        effect: 2,
        base_points: 0,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    s
}

fn item_spell(id: u32, trigger: u32) -> ItemSpell {
    ItemSpell {
        id,
        trigger,
        ..Default::default()
    }
}

fn weapon(entry: u32, spells: &[ItemSpell]) -> ItemTemplate {
    let mut t = ItemTemplate {
        entry,
        class: 2,
        subclass: 7,
        name: format!("Weapon {entry}"),
        display_id: 1000,
        inventory_type: 13,
        delay: 2000,
        stackable: 1,
        ..Default::default()
    };
    for (slot, s) in t.spells.iter_mut().zip(spells) {
        *slot = *s;
    }
    t
}

fn data() -> GameData {
    let mut d = base::data();
    let mut might = SpellTemplate {
        id: MIGHT,
        name: "Might".into(),
        equipped_item_class: -1,
        proc_chance: 100,
        duration_index: 1,
        ..Default::default()
    };
    might.effects[0] = SpellEffect {
        effect: 6,
        aura: AURA_MOD_STAT,
        misc_value: 0,
        base_points: 9,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    let mut extra = damage(EXTRA);
    extra.effects[0] = SpellEffect {
        effect: 19,
        base_points: 0,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    let mut cooled = damage(COOLED);
    cooled.start_recovery_category = crate::game::map::spells::GCD_SPELLS;
    cooled.start_recovery_time = 1500;
    for s in [
        damage(HURT),
        damage(HURT_OFF),
        might,
        damage(ON_USE),
        damage(ON_EQUIP),
        extra,
        cooled,
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d.spells.durations.insert(
        1,
        crate::game::data::spells::SpellDuration {
            base: 10_000,
            per_level: 0,
            max: 10_000,
        },
    );
    for t in [
        weapon(
            BLADE,
            &[
                item_spell(HURT, 2),
                item_spell(ON_USE, 0),
                item_spell(ON_EQUIP, 1),
                item_spell(MIGHT, 2),
            ],
        ),
        weapon(OFF_BLADE, &[item_spell(HURT_OFF, 2)]),
        weapon(EXTRA_BLADE, &[item_spell(EXTRA, 2), item_spell(HURT, 2)]),
        weapon(COOLED_BLADE, &[item_spell(COOLED, 2), item_spell(HURT, 2)]),
    ] {
        d.items.insert(t.entry, t);
    }
    d
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.holders.iter().any(|h| h.spell == spell))
}

/// A melee swing of `g`'s on `victim`, of `ex`.
fn swing(g: Guid, victim: Guid, ex: u32) -> Event {
    Event {
        actor: g,
        target: victim,
        actor_flags: procs::PROC_DONE_MELEE,
        target_flags: procs::PROC_TAKEN_MELEE,
        ex,
        damage: 10,
        school: 1,
        ..Default::default()
    }
}

/// What `e` takes off `victim`'s health, under a roll of `roll`.
fn dealt(m: &mut Map, victim: Guid, e: Event, roll: f32) -> u32 {
    let before = health(m, victim);
    enchant::FORCED_PROC_ROLL.with(|f| f.set(Some(roll)));
    procs::trigger(m, e);
    enchant::FORCED_PROC_ROLL.with(|f| f.set(None));
    before.saturating_sub(health(m, victim))
}

/// Thistle Tea's "item PPM overrides DBC chance and uses base weapon
/// speed", as mangos's `CastItemCombatSpell` has it: an item's procs a
/// minute at the weapon's own speed over the spell's chance; a chance over
/// 100 is one a minute; no rate and no chance, or no speed, none.
#[test]
fn an_items_spell_goes_off_by_its_procs_a_minute_at_its_own_speed_or_its_spells_chance() {
    let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
    assert!(close(
        enchant::item_spell_chance(8.0, 100, 1900),
        25.333_334
    ));
    assert!(close(enchant::item_spell_chance(2.0, 5, 3000), 10.0));
    assert!(close(enchant::item_spell_chance(0.0, 101, 3000), 5.0));
    assert!(close(enchant::item_spell_chance(0.0, 20, 3000), 20.0));
    assert!(close(enchant::item_spell_chance(0.0, 0, 3000), 0.0));
    assert!(close(enchant::item_spell_chance(0.0, 101, 0), 0.0));
}

/// Thistle Tea's "accepts damaging swings and blocks but rejects misses,
/// full absorption and dead targets" and "only on-hit slots roll and
/// triggers preserve the weapon, hand and implicit targeting", as mangos's
/// `ProcDamageAndSpell` and `CastItemCombatSpell` have them: a blow that
/// hit (a crit, a glancing or crushing one, one partly blocked) sets off
/// the weapon's chance-on-hit spells, its on-use and on-equip ones never;
/// a miss, a dodge, a parry, an evade, an immunity or a whole block none;
/// nor a blow on the dead or on oneself. The off hand's blow sets off the
/// off hand's weapon. A harmful spell goes to the victim, a helpful one to
/// the wielder.
#[tokio::test]
async fn a_weapons_chance_on_hit_goes_off_on_a_blow_that_hit_with_its_hand() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .push(crate::game::map::derive::SPELL_DUAL_WIELD);
            equip(m, ga, BLADE, SLOT_MAIN_HAND);
            equip(m, ga, OFF_BLADE, SLOT_OFF_HAND);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            // Every slot of the main hand: one point of HURT, MIGHT on the
            // wielder, and none of the others.
            let hit = dealt(m, brute, swing(ga, brute, procs::EX_NORMAL_HIT), 0.0);
            let might = (has(m, ga, MIGHT), has(m, brute, MIGHT));
            let landed: Vec<u32> = [
                procs::EX_CRITICAL_HIT,
                procs::EX_NORMAL_HIT | procs::EX_BLOCK,
                procs::EX_NORMAL_HIT | procs::EX_ABSORB,
                combat::outcome_ex(combat::Outcome::Glancing),
                combat::outcome_ex(combat::Outcome::Crushing),
            ]
            .into_iter()
            .map(|ex| dealt(m, brute, swing(ga, brute, ex), 0.0))
            .collect();
            let avoided: Vec<u32> = [
                procs::EX_MISS,
                procs::EX_DODGE,
                procs::EX_PARRY,
                procs::EX_EVADE,
                procs::EX_IMMUNE,
                procs::EX_BLOCK,
            ]
            .into_iter()
            .map(|ex| dealt(m, brute, swing(ga, brute, ex), 0.0))
            .collect();
            // The off hand's blow: its weapon's one point, not the main
            // hand's (whose MIGHT would be refreshed, not new).
            auras::remove_spell(m, ga, MIGHT, None);
            let off = dealt(
                m,
                brute,
                Event {
                    offhand: true,
                    ..swing(ga, brute, procs::EX_NORMAL_HIT)
                },
                0.0,
            );
            let off_might = has(m, ga, MIGHT);
            // A roll over the chance of fifty: nothing.
            let unlucky = dealt(m, brute, swing(ga, brute, procs::EX_NORMAL_HIT), 60.0);
            // Oneself; the dead.
            let before = health(m, ga);
            enchant::FORCED_PROC_ROLL.with(|f| f.set(Some(0.0)));
            procs::trigger(m, swing(ga, ga, procs::EX_NORMAL_HIT));
            enchant::FORCED_PROC_ROLL.with(|f| f.set(None));
            let own = before - health(m, ga);
            let corpse = spawn(m, ga, BRUTE, 60, 4.0);
            combat::kill(m, ga, corpse);
            auras::remove_spell(m, ga, MIGHT, None);
            let _ = dealt(m, corpse, swing(ga, corpse, procs::EX_NORMAL_HIT), 0.0);
            let dead = has(m, ga, MIGHT);
            (
                (hit, might),
                landed,
                avoided,
                (off, off_might),
                unlucky,
                (own, dead),
            )
        })
        .await
        .unwrap();
    let (hit, landed, avoided, off, unlucky, nothing) = r;
    assert_eq!(
        hit,
        (1, (true, false)),
        "HURT on the victim, MIGHT on the wielder"
    );
    assert_eq!(
        landed, [1; 5],
        "a crit, partly blocked, absorbed, glancing, crushing"
    );
    assert_eq!(
        avoided, [0; 6],
        "missed, dodged, parried, evaded, immune, wholly blocked"
    );
    assert_eq!(off, (1, false), "the off hand's weapon alone");
    assert_eq!(unlucky, 0);
    assert_eq!(nothing, (0, false), "nothing on oneself or the dead");
}

/// Thistle Tea's "GCD and the actual chance gate individual spells" and
/// "an extra attack proc stops recursive batches and subsequent enchant
/// rolls", as mangos's `CastItemCombatSpell` has them (it returns): a
/// chance-on-hit spell under its global cooldown, or one of extra attacks
/// while extra attacks are owed or being swung, stops the weapon's procs
/// there; out of it, it goes off.
#[tokio::test]
async fn a_spell_under_its_cooldown_or_of_extra_attacks_owed_stops_the_weapons_procs() {
    let (_realm, _a, ga, map) = setup_with(data(), Class::Warrior).await;
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            let hit = swing(ga, brute, procs::EX_NORMAL_HIT);
            equip(m, ga, COOLED_BLADE, SLOT_MAIN_HAND);
            let now = m.now;
            m.caster_mut(ga).unwrap().gcd_until = Some(now + std::time::Duration::from_secs(1));
            let cooling = dealt(m, brute, hit, 0.0);
            m.caster_mut(ga).unwrap().gcd_until = None;
            let ready = dealt(m, brute, hit, 0.0);
            equip(m, ga, EXTRA_BLADE, SLOT_MAIN_HAND);
            let extra_swing = dealt(
                m,
                brute,
                Event {
                    extra_attack: true,
                    ..hit
                },
                0.0,
            );
            m.combat_mut(ga).unwrap().extra_attacks = 1;
            let owed = dealt(m, brute, hit, 0.0);
            m.combat_mut(ga).unwrap().extra_attacks = 0;
            let free = dealt(m, brute, hit, 0.0);
            let banked = m.combat(ga).map(|c| c.extra_attacks + c.held_extra_attacks);
            (cooling, ready, extra_swing, owed, (free, banked))
        })
        .await
        .unwrap();
    let (cooling, ready, extra_swing, owed, free) = r;
    assert_eq!(cooling, 0, "the global cooldown stops the weapon's procs");
    assert_eq!(ready, 2, "out of it, both its spells");
    assert_eq!(extra_swing, 0, "an extra attack's swing stops them");
    assert_eq!(owed, 0, "so do extra attacks owed");
    assert_eq!(free, (1, Some(1)), "else an extra attack and the point");
}

/// Thistle Tea's "Destiny buffs its wielder through implicit targeting and
/// expires without leaving strength": Destiny's chance on hit (17152, a
/// chance over 100: one a minute) puts its two hundred strength on the
/// wielder for ten seconds, and they go with it.
#[tokio::test]
#[ignore = "extended"]
async fn destinys_chance_on_hit_gives_its_wielder_strength_for_ten_seconds() {
    const DESTINY: u32 = 647;
    const DESTINY_BUFF: u32 = 17_152;
    let data = crate::game::harness::load_real(&crate::game::harness::real_data("db"));
    let t = data.item(DESTINY).expect("Destiny");
    assert!(
        t.spells
            .iter()
            .any(|s| s.id == DESTINY_BUFF && s.trigger == 2),
        "its chance on hit"
    );
    let chance = enchant::item_spell_chance(
        0.0,
        data.spells.get(DESTINY_BUFF).unwrap().proc_chance,
        t.delay,
    );
    assert!(
        (chance - t.delay as f32 / 600.0).abs() < 1e-4,
        "one a minute"
    );
    let realm = crate::game::harness::realm_with(data);
    let mut a = crate::game::harness::Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, crate::game::map::MapKey::world(0));
    let r = map
        .query(move |m| {
            level(m, ga, 60);
            equip(m, ga, DESTINY, SLOT_MAIN_HAND);
            // A Kobold Vermin.
            let victim = spawn(m, ga, 6, 60, 3.0);
            let strength = |m: &Map| m.players[&ga].fields.get_u32(UNIT_FIELD_STAT0);
            let before = strength(m);
            let _ = dealt(m, victim, swing(ga, victim, procs::EX_NORMAL_HIT), 0.0);
            let lasts = auras::auras(m, ga)
                .and_then(|a| a.holders.iter().find(|h| h.spell == DESTINY_BUFF))
                .and_then(|h| h.expires)
                .map(|t| t.duration_since(m.now).as_millis());
            let buffed = strength(m);
            let on_victim = has(m, victim, DESTINY_BUFF);
            let now = m.now;
            if let Some(h) = auras::auras_mut(m, ga)
                .and_then(|a| a.holders.iter_mut().find(|h| h.spell == DESTINY_BUFF))
            {
                h.expires = Some(now);
            }
            auras::update(m);
            (
                before,
                buffed,
                lasts,
                on_victim,
                strength(m),
                has(m, ga, DESTINY_BUFF),
            )
        })
        .await
        .unwrap();
    let (before, buffed, lasts, on_victim, after, kept) = r;
    assert_eq!(buffed, before + 200, "two hundred strength");
    assert_eq!(lasts, Some(10_000), "for ten seconds");
    assert!(!on_victim, "on the wielder, not the victim");
    assert_eq!((after, kept), (before, false), "gone with it");
}

/// Thistle Tea's "real melee abilities qualify while ordinary ranged
/// spells do not", as mangos's `ProcDamageAndSpell` has it rather: a
/// spell wanting a weapon sets off the weapon's procs unless it stops the
/// attack. Heroic Strike, Sinister Strike and Raptor Strike do, and Auto
/// Shot, Aimed Shot, Throw and Shoot the ranged weapon's; Sap (which stops
/// the attack) and Fireball don't.
#[test]
#[ignore = "extended"]
fn spells_wanting_a_weapon_set_off_its_procs_unless_they_stop_the_attack() {
    let data = crate::game::harness::load_real(&crate::game::harness::real_data("db"));
    let procs = |id: u32| data.spells.get(id).is_some_and(enchant::spell_procs_weapon);
    for id in [78, 1752, 2973, 75, 19_434, 2764, 5019] {
        assert!(procs(id), "{id} procs the weapon");
    }
    for id in [6770, 133] {
        assert!(!procs(id), "{id} doesn't");
    }
}
