//! Phase 3, round 6, against Thistle Tea's `spell_requirements_test.exs`:
//! Cannibalize's body, and a spell at the caster's pet ranged at the pet.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, has};
use super::tests_p6_wspell::{BEAST, FRIEND, RANGE_30, creature, off, put, sides, unit_at};
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::Map;
use crate::game::map::auras::AURA_DUMMY;
use crate::game::map::pets::Kind;
use crate::game::map::spells::{Targets, checks, fail, mods};

/// Cannibalize and the channel it starts.
const CANNIBALIZE: u32 = 20_577;
const CHANNEL: u32 = 20_578;
/// A monster's humanoid; five yards.
const HUMANOID: u32 = 99_620;
const RANGE_5: u32 = 953;
/// A heal at the caster's pet.
const MEND: u32 = 991_400;

fn data(d: &mut GameData) {
    sides(d);
    d.creature_templates
        .insert(HUMANOID, creature(HUMANOID, 7, 14));
    d.spells.ranges.insert(RANGE_5, (0.0, 5.0));
    let mut cannibalize = bare(CANNIBALIZE, 0);
    cannibalize.range_index = RANGE_5;
    cannibalize.recovery_time = 120_000;
    cannibalize.script_name = "spell_cannibalize".into();
    effect(
        &mut cannibalize,
        0,
        SpellEffect {
            effect: 3,
            target_a: 1,
            ..Default::default()
        },
    );
    let mut channel = bare(CHANNEL, 0);
    channel.duration_index = LASTS_5S;
    effect(&mut channel, 0, aura(AURA_DUMMY, 0, 1, 1));
    let mut mend = bare(MEND, 0);
    mend.range_index = RANGE_30;
    effect(
        &mut mend,
        0,
        SpellEffect {
            effect: 10,
            base_points: 9,
            base_dice: 1,
            target_a: 5,
            ..Default::default()
        },
    );
    let spells: Vec<SpellTemplate> = vec![cannibalize, channel, mend];
    insert(d, spells);
}

/// Why Cannibalize would fail now, if it would.
fn eat(m: &mut Map, caster: Guid) -> Option<u8> {
    let s = m.realm.data.spells.get(CANNIBALIZE).cloned().unwrap();
    let mods = mods::for_spell(m, caster, &s);
    checks::check(m, caster, &s, &Targets::default(), None, false, &mods)
}

/// A body to eat, `dx` yards east of `a`, dead.
fn body(m: &mut Map, a: Guid, entry: u32, dx: f32) -> Guid {
    let g = unit_at(m, a, entry, dx, 0.0);
    crate::game::map::combat::kill(m, a, g);
    g
}

fn size(m: &Map, g: Guid) -> f32 {
    m.unit_fields(g)
        .map(|f| f.get_f32(UNIT_FIELD_BOUNDINGRADIUS))
        .unwrap()
}

/// Cannibalize wants a body near: a dead enemy humanoid or undead
/// creature (not the living, a friend's, a beast, one on a flight) within
/// five yards of both edges; a dead player, not a ghost, nor a friend's; a
/// released player's corpse, its owner's side its own. Without one it is
/// refused and its cooldown not started; with one it starts the channel
/// and its cooldown, and cast again is refused for that cooldown first
/// (Thistle Tea's `SpellRequirements.corpse` and `cast_result`; mangos's
/// `CannibalizeObjectCheck`).
#[tokio::test]
async fn cannibalize_wants_an_enemys_body_within_five_yards() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6XH", "Eater", Class::Warrior).await;
    let (_d, b, _) = human(&realm, "P6XI", "Victim", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[CANNIBALIZE]);
            let far = off(m, a, 0.0, 40.0);
            put(m, b, far);
            let mut out = Vec::new();
            let none = eat(m, a);
            let living = unit_at(m, a, HUMANOID, 1.0, 0.0);
            out.push(eat(m, a));
            m.remove_creature(living);
            for entry in [FRIEND, BEAST] {
                let g = body(m, a, entry, 1.0);
                out.push(eat(m, a));
                m.remove_creature(g);
            }
            let flying = body(m, a, HUMANOID, 1.0);
            m.unit_fields_mut(flying)
                .unwrap()
                .set_flag(UNIT_FIELD_FLAGS, 0x0010_0000, true);
            out.push(eat(m, a));
            m.remove_creature(flying);
            // Just beyond and just within five yards of both edges.
            let g = body(m, a, HUMANOID, 1.0);
            let reach = 5.0 + size(m, a) + size(m, g);
            let edge = |m: &Map, d: f32| off(m, a, d, 0.0);
            let beyond = edge(m, reach + 0.02);
            put(m, g, beyond);
            let beyond = eat(m, a);
            let within = edge(m, reach - 0.02);
            put(m, g, within);
            let within = eat(m, a);
            m.remove_creature(g);
            // A player: a friend's body, an enemy's, a ghost; a corpse.
            let near = off(m, a, 1.0, 0.0);
            put(m, b, near);
            crate::game::map::death::kill(m, b);
            let friend = eat(m, a);
            m.players
                .get_mut(&b)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_FACTIONTEMPLATE, 14);
            let enemy = eat(m, a);
            let now = m.now;
            m.players.get_mut(&b).unwrap().death =
                crate::game::map::death::Death::Ghost { since: now };
            let ghost = eat(m, a);
            m.players.get_mut(&b).unwrap().death =
                crate::game::map::death::Death::Dead { since: now };
            crate::game::map::death::release(m, b);
            let to = off(m, a, 0.0, 40.0);
            put(m, b, to);
            let corpse = eat(m, a);
            let corpse_present = !m.corpses.is_empty();
            (
                none,
                out,
                (beyond, within),
                (friend, enemy, ghost, corpse, corpse_present),
            )
        })
        .await
        .unwrap();
    let edible = None;
    let refused = Some(fail::NO_EDIBLE_CORPSES);
    assert_eq!(out.0, refused, "no body");
    assert_eq!(
        out.1,
        [refused, refused, refused, refused],
        "not the living, a friend's, a beast's, nor one on a flight"
    );
    assert_eq!(out.2, (refused, edible), "five yards of both edges");
    let (friend, enemy, ghost, corpse, corpse_present) = out.3;
    assert!(corpse_present, "released, a corpse");
    assert_eq!(
        (friend, enemy, ghost, corpse),
        (refused, edible, refused, edible),
        "a player's: not a friend's, an enemy's, not a ghost, their corpse"
    );
}

/// Without a body, Cannibalize is refused before its cooldown; with one, it
/// goes off and starts the channel and the cooldown; again, the cooldown
/// refuses it first (Thistle Tea's `cast_result`).
#[tokio::test]
async fn cannibalize_refused_starts_no_cooldown_and_eaten_starts_the_channel() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6XJ", "Ghoul", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[CANNIBALIZE]);
            let refused = kit::cast_fresh(m, a, CANNIBALIZE, Targets::default());
            let cooling = |m: &Map| {
                m.caster(a)
                    .is_some_and(|c| c.cooldowns.contains_key(&CANNIBALIZE))
            };
            let no_cooldown = !cooling(m);
            body(m, a, HUMANOID, 1.0);
            let eaten = kit::cast_fresh(m, a, CANNIBALIZE, Targets::default());
            let channel = has(m, a, CHANNEL);
            let cooled = cooling(m);
            if let Some(c) = m.caster_mut(a) {
                c.gcd_until = None;
            }
            let again = eat(m, a);
            (refused, no_cooldown, eaten, channel, cooled, again)
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (false, true, true, true, true, Some(fail::NOT_READY)),
        "refused without its cooldown; eaten, the channel and its cooldown; then not ready"
    );
}

/// A spell at the caster's pet is ranged at the pet, whatever unit the
/// client named (Thistle Tea's implicit pet range; mangos's
/// `TARGET_UNIT_CASTER_PET` range check).
#[tokio::test]
async fn a_spell_at_the_pet_is_ranged_at_the_pet() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6XK", "Mender", Class::Warrior).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[MEND]);
            let to = off(m, a, 50.0, 0.0);
            let pet =
                crate::game::map::pets::testing::spawn(m, a, BEAST, Kind::Hunter, 60, to, 0.0);
            m.players.get_mut(&a).unwrap().owned.pet = Some(pet);
            let s = m.realm.data.spells.get(MEND).cloned().unwrap();
            let mods = mods::for_spell(m, a, &s);
            let far = checks::check(m, a, &s, &at(a), None, false, &mods);
            let near = off(m, a, 10.0, 0.0);
            put(m, pet, near);
            let close = checks::check(m, a, &s, &at(a), None, false, &mods);
            (far, close)
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (Some(fail::OUT_OF_RANGE), None),
        "the pet 50 yards off, then 10"
    );
}
