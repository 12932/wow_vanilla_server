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
/// A craft at a focus of 50 001, two seconds, 10 mana and 5 seconds'
/// cooldown; foci of it reaching 5 and 100 yards, and one of another.
const CRAFT: u32 = 991_401;
const FOCUS: u32 = 99_630;
const FOCUS_FAR: u32 = 99_631;
const FOCUS_OTHER: u32 = 99_632;

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
    let mut craft = bare(CRAFT, 0);
    craft.requires_spell_focus = 50_001;
    craft.casting_time_index = CAST_2S;
    craft.mana_cost = 10;
    craft.recovery_time = 5_000;
    effect(&mut craft, 0, aura(AURA_DUMMY, 0, 1, 1));
    for (entry, id, reach) in [
        (FOCUS, 50_001, 5),
        (FOCUS_FAR, 50_001, 100),
        (FOCUS_OTHER, 50_002, 5),
    ] {
        let mut data = [0u32; 24];
        data[0] = id;
        data[1] = reach;
        d.gameobject_templates.insert(
            entry,
            crate::game::data::GameObjectTemplate {
                entry,
                kind: 8,
                name: format!("Focus {entry}"),
                size: 1.0,
                data,
                ..Default::default()
            },
        );
    }
    // A spell for humans alone, one for orcs alone, one for any female.
    for (spell, race_mask, gender) in [(991_402, 0x1, 2), (991_403, 0x2, 2), (991_404, 0, 1)] {
        d.scripts.spell_areas.push(crate::game::data::scripts::SpellArea {
            spell,
            race_mask,
            gender,
            ..Default::default()
        });
    }
    let spells: Vec<SpellTemplate> = vec![cannibalize, channel, mend, craft];
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

fn focus_at(m: &mut Map, a: Guid, entry: u32, dx: f32) -> Guid {
    let at = off(m, a, dx, 0.0);
    crate::game::map::objects::summon(m, None, entry, at, 0.0, 0, 0, 0, None).unwrap()
}

fn craft(m: &mut Map, caster: Guid) -> Option<u8> {
    kit::fresh(m, caster);
    let s = m.realm.data.spells.get(CRAFT).cloned().unwrap();
    let mods = mods::for_spell(m, caster, &s);
    checks::check(m, caster, &s, &Targets::default(), None, false, &mods)
}

/// A focus is one of the spell's, within its template's reach of both
/// edges in three dimensions, however far that reach (a hundred yards for
/// some); one of another kind won't do. A creature needs none. Lost while
/// the cast goes on, the cast fails as it ends, nothing paid and no
/// cooldown; kept, it goes off once, paid (Thistle Tea's `SpellFocus`;
/// mangos's `GameObjectFocusCheck` in `CheckCast`, at the start and the end).
#[tokio::test]
async fn a_focus_is_within_its_reach_and_checked_again_as_the_cast_ends() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6XL", "Crafter", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[CRAFT]);
            let none = craft(m, a);
            let other = focus_at(m, a, FOCUS_OTHER, 1.0);
            let wrong = craft(m, a);
            crate::game::map::objects::despawn(m, other);
            let reach = 5.0 + size(m, a) + 0.389;
            let f = focus_at(m, a, FOCUS, reach - 0.02);
            let within = craft(m, a);
            crate::game::map::objects::despawn(m, f);
            let f = focus_at(m, a, FOCUS, reach + 0.02);
            let beyond = craft(m, a);
            crate::game::map::objects::despawn(m, f);
            let far = focus_at(m, a, FOCUS_FAR, 80.0);
            let long = craft(m, a);
            crate::game::map::objects::despawn(m, far);
            let creature = unit_at(m, a, BEAST, 3.0, 0.0);
            let creature_casts = kit::cast_fresh(m, creature, CRAFT, Targets::default());
            // Lost as the cast goes on, then kept.
            let mana = |m: &Map| kit::power(m, a, 0);
            let cooling = |m: &Map| {
                m.caster(a)
                    .is_some_and(|c| c.cooldowns.contains_key(&CRAFT))
            };
            let f = focus_at(m, a, FOCUS, 1.0);
            kit::set_power(m, a, 0, 1_000);
            let before = mana(m);
            assert!(kit::cast_fresh(m, a, CRAFT, Targets::default()));
            crate::game::map::objects::despawn(m, f);
            kit::pass(m, 2_100);
            let lost = (
                kit::casting(m, a),
                before - mana(m),
                cooling(m),
                has(m, a, CRAFT),
            );
            focus_at(m, a, FOCUS, 1.0);
            kit::set_power(m, a, 0, 1_000);
            let before = mana(m);
            assert!(kit::cast_fresh(m, a, CRAFT, Targets::default()));
            kit::pass(m, 2_100);
            let kept = (
                kit::casting(m, a),
                before - mana(m),
                cooling(m),
                has(m, a, CRAFT),
            );
            (
                none,
                wrong,
                within,
                beyond,
                long,
                creature_casts,
                lost,
                kept,
            )
        })
        .await
        .unwrap();
    let refused = Some(fail::REQUIRES_SPELL_FOCUS);
    assert_eq!((out.0, out.1), (refused, refused), "none, or another kind");
    assert_eq!((out.2, out.3), (None, refused), "its reach of both edges");
    assert_eq!(out.4, None, "a hundred yards' focus at eighty");
    assert!(out.5, "a creature needs none");
    assert_eq!(
        out.6,
        (None, 0, false, false),
        "lost: failed, unpaid, no cooldown"
    );
    assert_eq!(out.7, (None, 10, true, true), "kept: once, paid");
}

/// A spell kept to a race or a gender by its area rules is judged for a
/// pet by its master: a human's pet may cast the humans' spell, not the
/// orcs'; a creature of no master, neither asking that, nor one for women
/// (Thistle Tea's `SpellAreas.context`, its controlling player's facts;
/// mangos's `GetSpellAllowedInLocationError` with `GetBeneficiaryPlayer`).
#[tokio::test]
async fn a_pets_area_rules_are_its_masters() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P6XV", "Arean", Class::Warrior).await;
    let out = map
        .query(move |m| {
            let to = off(m, a, 2.0, 0.0);
            let pet = crate::game::map::pets::testing::spawn(m, a, BEAST, Kind::Hunter, 60, to, 0.0);
            m.players.get_mut(&a).unwrap().owned.pet = Some(pet);
            let wild = unit_at(m, a, BEAST, 4.0, 0.0);
            let allowed = |m: &Map, g: Guid| {
                [991_402, 991_403, 991_404]
                    .map(|s| crate::game::map::spell_areas::allowed(m, g, s))
            };
            let male = m.players[&a].row.gender == 0;
            (allowed(m, a), allowed(m, pet), allowed(m, wild), male)
        })
        .await
        .unwrap();
    let (me, pet, wild, male) = out;
    assert_eq!(me, [true, false, !male], "the human");
    assert_eq!(pet, me, "the pet as its master");
    assert_eq!(wild, [false, false, false], "no master to ask");
}
