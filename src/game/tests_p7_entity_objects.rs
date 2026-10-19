//! Round 7's game object tests, ported from Thistle Tea's
//! `test/game/world/entity/game_object_spell_cast_test.exs` and its
//! neighbours (docs/parity/matrix.csv): a creature using a quest object,
//! an object's damage making it no foe.

use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::scripts::ScriptStep;
use crate::game::data::spells::SpellEffect;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::map::objects::{self, GO_FLAG_IN_USE, GO_GOOBER, GO_STATE_ACTIVE};
use crate::game::tests_p3_combat::{BRUTE, START, data, setup_with, spawn};
use crate::game::tests_p3_entity::spell;
use crate::game::tests_p3_entity_objects::object;

/// A quest object for a quest, and a spot it stands at.
const QUEST_OBJECT: u32 = 97_710;
/// An object a script casts from.
const BRAZIER: u32 = 97_711;
/// Two hundred fire, at the unit aimed at.
const FLAME: u32 = 97_712;
/// A quest object whose spell is for a brute near.
const SHRINE: u32 = 97_713;
/// A dummy aura on the brute near its caster its script names.
const BLESSING: u32 = 97_714;

fn world() -> GameData {
    let mut d = data();
    for t in [
        // On quest 9_999 (none here), closing after a second.
        object(QUEST_OBJECT, GO_GOOBER, "Quest Object", &[(1, 9_999)]),
        object(BRAZIER, 5, "Brazier", &[]),
        object(SHRINE, GO_GOOBER, "Shrine", &[(10, BLESSING)]),
    ] {
        d.gameobject_templates.insert(t.entry, t);
    }
    let mut flame = spell(
        FLAME,
        &[SpellEffect {
            effect: 2,
            base_points: 199,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        }],
    );
    flame.school = 2;
    flame.dmg_class = 1;
    d.spells.spells.insert(FLAME, flame);
    d.spells.spells.insert(
        BLESSING,
        spell(
            BLESSING,
            &[crate::game::tests_p3_entity::aura_effect(
                crate::game::map::auras::AURA_DUMMY,
                1,
                0,
                38,
            )],
        ),
    );
    d.spells.script_targets.insert(BLESSING, vec![(1, BRUTE)]);
    d
}

fn at(yards: f32) -> Vector3d {
    Vector3d {
        x: START.0 + yards,
        y: START.1,
        z: START.2,
    }
}

/// A creature uses a quest object where it stands, as a script has it
/// (command 13): it goes off for it, with no player's quest asked about
/// (Thistle Tea's "a creature uses a quest object in place, with no
/// player quest checks"; mangos-classic's `GameObject::Use` asks a quest
/// only of a player).
#[tokio::test]
async fn a_creature_uses_a_quest_object_without_a_players_quest() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let user = spawn(m, ga, BRUTE, 10, 1.0);
            let g = objects::summon(m, None, QUEST_OBJECT, at(2.0), 0.0, 0, 0, 0, None).unwrap();
            let before = m.gameobject(g).unwrap().fields.get_u32(GAMEOBJECT_STATE);
            objects::used_by_script(m, user, g);
            let o = m.gameobject(g).unwrap();
            (
                before,
                o.fields.get_u32(GAMEOBJECT_STATE),
                o.fields.get_u32(GAMEOBJECT_FLAGS) & GO_FLAG_IN_USE,
            )
        })
        .await
        .unwrap();
    assert_ne!(out.0, GO_STATE_ACTIVE, "at rest");
    assert_eq!(
        (out.1, out.2),
        (GO_STATE_ACTIVE, GO_FLAG_IN_USE),
        "gone off, in use"
    );
}

/// A quest object's spell is for the creatures its spell names, near:
/// a brute by it has it, the player who used it doesn't, and with none
/// near no one does; it isn't put on the user instead (Thistle Tea's
/// "quest objects resolve nearby creatures instead of the user" and "does
/// not substitute an activating player for a missing creature"). The
/// user casts it, as mangos-classic's `GameObject::Use` has a goober's
/// spell cast (its TODO: by the object), so near the user.
#[tokio::test]
async fn a_quest_objects_spell_is_for_the_creature_it_names_not_its_user() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let shrine = objects::summon(m, None, SHRINE, at(2.0), 0.0, 0, 0, 0, None).unwrap();
            let blessed = |m: &crate::game::map::Map, g| {
                crate::game::map::auras::auras(m, g).is_some_and(|a| a.has_spell(BLESSING))
            };
            objects::used_by_script(m, ga, shrine);
            let alone = blessed(m, ga);
            m.objects.goobers.remove(&shrine);
            let brute = spawn(m, ga, BRUTE, 10, 3.0);
            objects::used_by_script(m, ga, shrine);
            (alone, blessed(m, ga), blessed(m, brute))
        })
        .await
        .unwrap();
    assert_eq!(out, (false, false, true));
}

/// A script's cast from an object (command 15, a brazier's flame) harms
/// whom it lands on, but makes the object no one's foe: a creature has no
/// object on its threat list and no fight of it, and fights on the one it
/// fought (Thistle Tea's "object damage
/// retains health loss without creating an object combat opponent").
#[tokio::test]
async fn an_objects_damage_harms_but_makes_it_no_foe() {
    let (_realm, _a, ga, map) = setup_with(world(), Class::Warrior).await;
    let out = map
        .query(move |m| {
            let brazier = objects::summon(m, None, BRAZIER, at(2.0), 0.0, 0, 0, 0, None).unwrap();
            let f = &mut m.players.get_mut(&ga).unwrap().fields;
            f.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
            f.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
            let idle = spawn(m, ga, BRUTE, 10, 3.0);
            let fighting = spawn(m, ga, BRUTE, 10, 4.0);
            crate::game::map::ai::attacked(m, fighting, ga);
            let cast = |m: &mut crate::game::map::Map, target| {
                crate::game::map::scripts::execute(
                    m,
                    &ScriptStep {
                        command: 15,
                        datalong: [FLAME, 0, 0, 0],
                        ..Default::default()
                    },
                    brazier,
                    target,
                );
            };
            let health =
                |m: &crate::game::map::Map, g| m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_HEALTH);
            let before = [idle, fighting, ga].map(|g| health(m, g));
            for g in [idle, fighting, ga] {
                cast(m, g);
            }
            let after = [idle, fighting, ga].map(|g| health(m, g));
            let threat = |m: &crate::game::map::Map, g| {
                m.creature(g)
                    .unwrap()
                    .threat
                    .iter()
                    .map(|(t, _)| *t)
                    .collect::<Vec<_>>()
            };
            let idle_view = (
                threat(m, idle),
                m.creature(idle).unwrap().combat.attacking,
                m.unit_fields(idle).unwrap().get_u32(UNIT_FIELD_FLAGS)
                    & crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            );
            let fighting_view = (
                threat(m, fighting),
                m.creature(fighting).unwrap().combat.victim,
            );
            (before, after, idle_view, fighting_view, brazier)
        })
        .await
        .unwrap();
    let (before, after, idle, fighting, brazier) = out;
    assert!(guid::is_game_object(brazier));
    for i in 0..3 {
        assert!(after[i] < before[i], "harmed: {before:?} {after:?}");
    }
    assert_eq!(idle, (vec![], false, 0), "no foe, no fight");
    assert_eq!(fighting, (vec![ga], ga), "its fight its own");
}
