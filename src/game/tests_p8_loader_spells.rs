//! Phase 8's loader round, the spells (Thistle Tea's `world/loader/*_dbc_test.exs`
//! and `*_vmangos_test.exs` of spells): Flare, the state and damage
//! immunities, durability, facing, the areas about the caster, Water
//! Walking in a form, Golemagg's Trust and the effect mechanics, on the
//! real data.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_spells_kit::*;
use crate::game::data::GameData;
use crate::game::data::spells::{self as sd, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::combat::Outcome;
use crate::game::map::spells::checks::{self, fail};
use crate::game::map::spells::{self, mods};
use crate::game::map::{Map, MapHandle, auras};
use crate::game::tests_p3_combat::{equip, forced_swing};

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

fn spell(d: &GameData, id: u32) -> SpellTemplate {
    d.spells
        .get(id)
        .cloned()
        .unwrap_or_else(|| panic!("spell {id}"))
}

/// The units `c` was told `spell` hit.
async fn hits_of(c: &mut Client, spell: u32) -> Option<Vec<Guid>> {
    c.gets(Duration::from_millis(500), |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == spell => Some(g.hits.clone()),
        _ => None,
    })
    .await
}

const FLARE: u32 = 1543;
const FLAMESTRIKE: u32 = 2120;
const STEALTH: u32 = 1784;
const LESSER_INVISIBILITY: u32 = 66;
const OLD_FAERIE_FIRE: u32 = 1069;

/// Thistle Tea's Flare (`flare_dbc_test.exs`): a harmful ground spell whose
/// two areas keep off stealth and invisibility (dispel types 5 and 6) and
/// purge them; its cast hits no unit, where Flamestrike's hits the unit
/// it lands on. A stealthed kobold and an invisible one in it are seen
/// again; neither hides again while the flare burns, and both can once
/// it is out (its 30 seconds).
#[tokio::test]
#[ignore = "extended"]
async fn flare_hits_no_one_reveals_both_hidings_and_keeps_them_off_while_it_burns() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let flare = spell(&d, FLARE);
    assert!(!auras::positive(&flare), "harmful");
    assert!(flare.attr(1, 0x8000), "its immunity purges");
    let areas: Vec<(u32, i32)> = flare
        .effects
        .iter()
        .filter(|e| e.effect == 27)
        .map(|e| (e.aura, e.misc_value))
        .collect();
    assert_eq!(
        areas,
        vec![
            (auras::AURA_DISPEL_IMMUNITY, 5),
            (auras::AURA_DISPEL_IMMUNITY, 6)
        ]
    );
    let (mut c, g, map) = player(&realm, "P8LFLARE", "Flarer", Race::Dwarf, Class::Hunter).await;
    let (sneak, ghost) = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let sneak = dummy(m, g, 60, 15.0);
        let ghost = dummy(m, g, 60, 15.5);
        spells::cast_triggered(m, sneak, STEALTH, Some(sneak));
        spells::cast_triggered(m, ghost, LESSER_INVISIBILITY, Some(ghost));
        (sneak, ghost)
    })
    .await;
    let hidden = move |m: &Map| (has(m, sneak, STEALTH), has(m, ghost, LESSER_INVISIBILITY));
    let before = on(&map, move |m| {
        let before = hidden(m);
        spells::cast_triggered(m, g, FLARE, Some(sneak));
        before
    })
    .await;
    let flare_hits = hits_of(&mut c, FLARE).await;
    let out = on(&map, move |m| {
        pass(m, 1000);
        let revealed = hidden(m);
        let flared = (has(m, sneak, FLARE), has(m, ghost, FLARE));
        spells::cast_triggered(m, sneak, STEALTH, Some(sneak));
        spells::cast_triggered(m, ghost, LESSER_INVISIBILITY, Some(ghost));
        let kept_off = hidden(m);
        for _ in 0..31 {
            pass(m, 1000);
        }
        let out = (has(m, sneak, FLARE), has(m, ghost, FLARE));
        spells::cast_triggered(m, sneak, STEALTH, Some(sneak));
        spells::cast_triggered(m, ghost, LESSER_INVISIBILITY, Some(ghost));
        (before, revealed, flared, kept_off, out, hidden(m))
    })
    .await;
    assert_eq!(flare_hits, Some(vec![]), "Flare hits no unit");
    let (before, revealed, flared, kept_off, out, again) = out;
    assert_eq!(before, (true, true), "hidden first");
    assert_eq!(revealed, (false, false), "revealed");
    assert_eq!(flared, (true, true), "in the flare");
    assert_eq!(kept_off, (false, false), "and can't hide in it");
    assert_eq!(out, (false, false), "out with the flare");
    assert_eq!(again, (true, true), "then hidden again");

    let k = on(&map, move |m| {
        let k = dummy(m, g, 60, 30.0);
        fresh(m, g);
        spells::cast_triggered(m, g, FLAMESTRIKE, Some(k));
        k
    })
    .await;
    assert_eq!(hits_of(&mut c, FLAMESTRIKE).await, Some(vec![k]));
    assert!(on(&map, move |m| health(m, k) < 1_000_000).await, "burned");
}

/// Thistle Tea's immunities (`effect_immunity_dbc_test.exs`): Self
/// Invulnerability (16621) is a physical damage immunity, so a swing does
/// nothing and a Fireball still burns; Charge ignores immunities, Taunt
/// school immunities. The old Faerie Fire (1069) keeps off stealth and
/// invisibility, helpful and harmful alike, and purges them: a stealthed
/// kobold is seen, and can't stealth again under it.
#[tokio::test]
#[ignore = "extended"]
async fn damage_and_state_immunities_keep_off_what_they_name() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    let invulnerable = spell(&d, 16_621);
    let e = &invulnerable.effects[0];
    assert_eq!((e.aura, e.misc_value), (auras::AURA_DAMAGE_IMMUNITY, 1));
    assert!(spell(&d, 100).attr(0, sd::ATTR_NO_IMMUNITIES));
    assert!(spell(&d, 355).attr(2, sd::ATTR_EX2_NO_SCHOOL_IMMUNITIES));
    let fire = spell(&d, OLD_FAERIE_FIRE);
    assert!(
        fire.attr(1, 0x8000) && fire.attr(1, 0x1_0000),
        "purges, both ways"
    );
    let states: Vec<i32> = fire
        .effects
        .iter()
        .filter(|e| e.aura == auras::AURA_STATE_IMMUNITY)
        .map(|e| e.misc_value)
        .collect();
    assert_eq!(states, vec![18, 16], "invisibility and stealth");

    let (_c, g, map) = player(&realm, "P8LIMMUNE2", "Immuna", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let k = dummy(m, g, 60, 2.0);
        apply(m, k, k, 16_621);
        forced_swing(m, g, k, Outcome::Hit);
        let swung = health(m, k);
        spells::cast_triggered(m, g, 133, Some(k));
        let burned = health(m, k);
        let sneak = dummy(m, g, 60, 10.0);
        let stealthed =
            |m: &Map| auras::auras(m, sneak).is_some_and(|a| a.has(auras::AURA_MOD_STEALTH));
        spells::cast_triggered(m, sneak, STEALTH, Some(sneak));
        let hid = stealthed(m);
        spells::cast_triggered(m, g, OLD_FAERIE_FIRE, Some(sneak));
        let seen = (has(m, sneak, OLD_FAERIE_FIRE), has(m, sneak, STEALTH));
        spells::cast_triggered(m, sneak, STEALTH, Some(sneak));
        (swung, burned, hid, seen, stealthed(m))
    })
    .await;
    assert_eq!(out.0, 1_000_000, "no harm from a swing");
    assert!(out.1 < 1_000_000, "a Fireball burns");
    assert!(out.2, "stealthed");
    assert_eq!(out.3, (true, false), "the fire on, the stealth off");
    // Its stealth effect kept off, as mangos's `IsImmuneToSpellEffect`
    // does a state immunity's (the rest of the spell may go on).
    assert!(!out.4, "and no stealth under it");
}

/// An item's durability now, in `g`'s equipment `slot` or the backpack's.
fn durability(m: &Map, g: Guid, bag: u8, slot: u8) -> u32 {
    m.players[&g]
        .items
        .iter()
        .find(|i| i.bag == bag && i.slot == slot)
        .map_or(u32::MAX, |i| i.fields.get_u32(ITEM_FIELD_DURABILITY))
}

/// Thistle Tea's durability spells (`spell_durability_dbc_test.exs`):
/// Melt Weapon (21388) takes a point from the main hand, Force Reactive
/// Disk (22619) one from the off hand, Corrupt Weapon's 23437 all of the
/// ranged weapon's and 23438 (-50,000 points) gives it all back. The test
/// spell 16722 takes 5 from everything carried, half the main hand's, and
/// 1 from everything worn.
#[tokio::test]
#[ignore = "extended"]
async fn durability_spells_wear_the_slots_they_name() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P8LWEAR", "Wearer", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        equip(m, g, 25, 15);
        equip(m, g, 2362, 16);
        equip(m, g, 2504, 17);
        crate::game::map::inventory::add_item(m, g, 1195, 1, false).expect("room");
        let bagged = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == 1195)
            .map(|i| (i.bag, i.slot))
            .unwrap();
        let foe = dummy(m, g, 60, 3.0);
        let all = |m: &Map| {
            (
                durability(m, g, crate::game::store::BAG_PLAYER, 15),
                durability(m, g, crate::game::store::BAG_PLAYER, 16),
                durability(m, g, crate::game::store::BAG_PLAYER, 17),
                durability(m, g, bagged.0, bagged.1),
            )
        };
        let mut seen = vec![all(m)];
        spells::cast_triggered(m, foe, 21_388, Some(g));
        seen.push(all(m));
        for id in [22_619, 23_437, 23_438, 16_722] {
            spells::cast_triggered(m, g, id, Some(g));
            seen.push(all(m));
        }
        seen
    })
    .await;
    assert_eq!(
        out,
        vec![
            (20, 20, 20, 35),
            (19, 20, 20, 35),
            (19, 19, 20, 35),
            (19, 19, 0, 35),
            (19, 19, 20, 35),
            (3, 13, 14, 30),
        ]
    );
}

/// Thistle Tea's facing (`spell_facing_vmangos_test.exs`): vmangos's
/// custom flag (`CUSTOM_FACE_TARGET`), not the spell's harm, says which
/// spells want their target in front: Auto Shot, Heroic Strike,
/// Frostbolt, Fireball, Lightning Bolt, Smite, Shadow Bolt, Sinister
/// Strike, Gouge, Eviscerate and Shoot do; Renew, Corruption and Lesser
/// Heal don't. A Frostbolt, Fireball or Shadow Bolt at a kobold behind is
/// refused; a Corruption isn't, nor a Lesser Heal at a friend behind.
#[tokio::test]
#[ignore = "extended"]
async fn spells_want_their_target_in_front_as_vmangos_says() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    for id in [75, 78, 116, 133, 403, 585, 686, 1752, 1776, 2098, 5019] {
        assert!(spell(&d, id).custom(sd::CUSTOM_FACE_TARGET), "{id}");
    }
    for id in [139, 172, 2050] {
        assert!(!spell(&d, id).custom(sd::CUSTOM_FACE_TARGET), "{id}");
    }
    let (_c, g, map) = player(&realm, "P8LFACE", "Facer", Race::Human, Class::Warlock).await;
    let (_f, friend, _) = player(&realm, "P8LFACE2", "Facee", Race::Human, Class::Priest).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let foe = dummy(m, g, 60, 10.0);
        let p = m.players[&g].position();
        let o = m.players[&g].movement.orientation;
        if let Some(f) = m.players.get_mut(&friend) {
            f.movement.position = Vector3d {
                x: p.x + 10.0 * o.cos(),
                y: p.y + 10.0 * o.sin(),
                z: p.z,
            };
        }
        if let Some(me) = m.players.get_mut(&g) {
            me.movement.orientation += std::f32::consts::PI;
        }
        let refused = |m: &mut Map, id: u32, target: Guid| {
            let s = spell(&m.realm.data, id);
            let mods = mods::for_spell(m, g, &s);
            fresh(m, g);
            checks::check(m, g, &s, &at(target), None, false, &mods)
        };
        [
            refused(m, 116, foe),
            refused(m, 133, foe),
            refused(m, 686, foe),
            refused(m, 172, foe),
            refused(m, 2050, friend),
        ]
    })
    .await;
    let behind = Some(fail::UNIT_NOT_INFRONT);
    assert_eq!(out, [behind, behind, behind, None, None]);
}

/// Thistle Tea's areas about the caster (`spell_friendly_area_dbc_test.exs`):
/// the Healing Auras (5016, 5607, 6275, 11900) are helpful and heal the
/// friends within 10 yards of their caster, no foe; Arcane Explosion
/// (1449) and Inferno's (22703) are harmful and strike the foes within 10
/// yards, no friend.
#[tokio::test]
#[ignore = "extended"]
async fn caster_areas_take_friends_or_foes_as_their_spells_say() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    for id in [5016, 5607, 6275, 11_900] {
        assert!(auras::positive(&spell(&d, id)), "{id} helps");
    }
    for id in [1449, 22_703] {
        assert!(!auras::positive(&spell(&d, id)), "{id} harms");
    }
    let (_c, g, map) = player(&realm, "P8LAREA", "Arear", Race::Human, Class::Mage).await;
    let (_f, friend, _) = player(&realm, "P8LAREA2", "Areaf", Race::Human, Class::Priest).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        ready(m, friend, 60, &[]);
        let p = m.players[&g].position();
        if let Some(f) = m.players.get_mut(&friend) {
            f.movement.position = Vector3d { x: p.x + 3.0, ..p };
        }
        let near = dummy(m, g, 60, 5.0);
        let far = dummy(m, g, 60, 15.0);
        let mut seen = Vec::new();
        for id in [5016, 5607, 6275, 11_900, 1449, 22_703] {
            for u in [g, friend] {
                set_health(m, u, 100);
            }
            for u in [near, far] {
                set_health(m, u, 500_000);
            }
            fresh(m, g);
            spells::cast_triggered(m, g, id, None);
            seen.push((
                id,
                health(m, g) > 100,
                health(m, friend) > 100,
                health(m, near) != 500_000,
                health(m, far) != 500_000,
            ));
        }
        seen
    })
    .await;
    assert_eq!(
        out,
        vec![
            (5016, true, true, false, false),
            (5607, true, true, false, false),
            (6275, true, true, false, false),
            (11_900, true, true, false, false),
            (1449, false, false, true, false),
            (22_703, false, false, true, false),
        ]
    );
}

/// Thistle Tea's "Cat cancels Water Walking while Moonkin retains it"
/// (`spell_shapeshift_dbc_test.exs`): Water Walking (546) breaks when its
/// bearer takes a shapeshifting form (`AURA_INTERRUPT_FLAG_SHAPESHIFTING`),
/// but not Moonkin, which acts as no shapeshift.
#[tokio::test]
#[ignore = "extended"]
async fn cat_form_ends_water_walking_and_moonkin_keeps_it() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let d = realm.data.clone();
    assert_ne!(
        spell(&d, 546).aura_interrupt_flags & auras::INTERRUPT_SHAPESHIFT,
        0
    );
    let (_c, g, map) = player(
        &realm,
        "P8LWATER",
        "Waterwalk",
        Race::NightElf,
        Class::Druid,
    )
    .await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let mut kept = Vec::new();
        for form in [768, 24_858] {
            spells::cast_triggered(m, g, 546, Some(g));
            let on = has(m, g, 546);
            spells::cast_triggered(m, g, form, Some(g));
            kept.push((form, on, has(m, g, form), has(m, g, 546)));
            auras::remove_spell(m, g, form, None);
            auras::remove_spell(m, g, 546, None);
        }
        kept
    })
    .await;
    assert_eq!(
        out,
        vec![(768, true, true, false), (24_858, true, true, true)]
    );
}

/// Thistle Tea's Golemagg's Trust (`golemaggs_trust_dbc_test.exs`): 20553
/// lasts two seconds and gives 500 physical damage and half again the
/// attack speed (2,000 ms to 1,333), and when it ends, neither.
#[tokio::test]
#[ignore = "extended"]
async fn golemaggs_trust_gives_damage_and_haste_for_two_seconds() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let (_c, g, map) = player(&realm, "P8LTRUST", "Trusted", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        ready(m, g, 60, &[]);
        let rager = dummy(m, g, 63, 5.0);
        let state = |m: &Map| {
            let speed = m
                .unit_fields(rager)
                .map_or(0, |f| f.get_u32(UNIT_FIELD_BASEATTACKTIME));
            let damage =
                auras::auras(m, rager).map_or(0, |a| a.sum_mask(auras::AURA_MOD_DAMAGE_DONE, 1));
            (speed, damage)
        };
        let before = state(m);
        apply(m, rager, rager, 20_553);
        let lasts = holder(m, rager, 20_553).and_then(|h| h.duration);
        let buffed = state(m);
        pass(m, 2000);
        (before, lasts, buffed, has(m, rager, 20_553), state(m))
    })
    .await;
    assert_eq!(out.0, (2000, 0));
    assert_eq!(out.1, Some(2000));
    assert_eq!(out.2, (1333, 500));
    assert_eq!((out.3, out.4), (false, (2000, 0)));
}

/// Thistle Tea's effect mechanics (`mechanic_resistance_dbc_test.exs`):
/// Hardiness (20573) resists stuns (mechanic 12) by 25%; Frostbolt has no
/// mechanic of its own but its slow's effect is a snare (11); and an
/// effect's mechanic from vmangos's `spell_effect_mod` stands over the
/// DBC's: Disarm's (8379) is 3, its DBC row's 0.
#[test]
#[ignore = "extended"]
fn effect_mechanics_are_the_dbcs_or_vmangos_over_them() {
    let Some(d) = crate::game::data::load::real_data() else {
        return;
    };
    let hardiness = spell(d, 20_573);
    let e = &hardiness.effects[0];
    assert_eq!(
        (e.aura, e.misc_value, spells::points(&hardiness, 0, 60)),
        (auras::AURA_MOD_MECHANIC_RESISTANCE, 12, 25)
    );
    let frostbolt = spell(d, 116);
    assert_eq!(frostbolt.mechanic, 0);
    assert_eq!(frostbolt.effects[0].mechanic, 11);
    assert_eq!(spell(d, 8379).effects[0].mechanic, 3);
}
