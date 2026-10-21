//! Round 11's effect-resolver rows (docs/parity/matrix.csv): what a
//! triggered spell's cast ends, and what it must have to go.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, holder, pass};
use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX_CHANNELED, ProcEvent, SpellDuration, SpellEffect};
use crate::game::harness::{self, Client};
use crate::game::map::Map;
use crate::game::map::auras::*;
use crate::game::map::procs::EX_CAST_END;
use crate::game::map::spells;
use crate::game::map::trinkets::{UNSTABLE_POWER, UNSTABLE_POWER_BONUS};

/// An Arcane Missiles of three missiles a second apart, and its missile.
const MISSILES: u32 = 997_301;
const MISSILE: u32 = 997_302;
const THREE_SECONDS: u32 = 997;

fn missiles_data(d: &mut GameData) {
    // Unstable Power as the real data has it: the parent's dummy, its
    // bonus of 17 spell power a stack, twelve, spent on a cast's end.
    let mut unstable = bare(UNSTABLE_POWER, 0);
    unstable.duration_index = LASTS_LONG;
    effect(&mut unstable, 0, aura(AURA_DUMMY, 0, 0, 1));
    let mut bonus = bare(UNSTABLE_POWER_BONUS, 0);
    bonus.duration_index = LASTS_LONG;
    bonus.stack_amount = 12;
    bonus.proc_flags = 0x15550;
    bonus.proc_chance = 100;
    effect(&mut bonus, 0, aura(AURA_MOD_DAMAGE_DONE, 0x7e, 17, 1));
    d.spells.proc_events.insert(
        UNSTABLE_POWER_BONUS,
        ProcEvent {
            ex: EX_CAST_END,
            ..Default::default()
        },
    );
    d.spells.durations.insert(
        THREE_SECONDS,
        SpellDuration {
            base: 3_000,
            per_level: 0,
            max: 3_000,
        },
    );
    let mut channel = bare(MISSILES, 6);
    channel.attributes[1] |= ATTR_EX_CHANNELED;
    channel.duration_index = THREE_SECONDS;
    effect(
        &mut channel,
        0,
        SpellEffect {
            trigger_spell: MISSILE,
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_TRIGGER_SPELL, 0, 0, 6)
        },
    );
    let mut missile = bare(MISSILE, 6);
    effect(&mut missile, 0, damage(10));
    insert(d, [unstable, bonus, channel, missile]);
}

fn stacks(m: &Map, g: Guid) -> u32 {
    holder(m, g, UNSTABLE_POWER_BONUS).map_or(0, |h| h.stacks.max(1))
}

/// Thistle Tea's "Arcane Missiles snapshots its bonus before the child
/// cast spends one Unstable Power stack" (triggered_hit_dbc_test 34) and
/// "cast completion reaches only the explicit caster owner with its proc
/// origin" (event_sink_test 130): each missile a channel triggers ends a
/// cast of its own and spends a stack, its damage taken with the stacks
/// it went with (the channel, which harms nothing itself, spends none, as
/// Thistle Tea's spender rule has it), as mangos-classic's `Spell::cast`
/// procs `PROC_EX_CAST_END` for a triggered spell too (where wvs's
/// triggered casts ended nothing: the missiles spent none).
#[tokio::test]
async fn each_missile_of_a_channel_spends_a_stack_of_unstable_power() {
    let realm = fast_realm(missiles_data);
    let (_c, a, map) = human(&realm, "P11S1", "Missiles", Class::Mage).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[MISSILES]);
            let kobold = kit::dummy(m, a, 60, 3.0);
            spells::cast_triggered(m, a, UNSTABLE_POWER, Some(a));
            let full = stacks(m, a);
            cast_fresh(m, a, MISSILES, at(kobold));
            let mut each = vec![stacks(m, a)];
            for _ in 0..3 {
                pass(m, 1_000);
                each.push(stacks(m, a));
            }
            (full, each, 1_000_000 - kit::health(m, kobold))
        })
        .await
        .unwrap();
    let (full, each, dealt) = out;
    assert_eq!(full, 12);
    assert_eq!(
        each,
        vec![12, 11, 10, 9],
        "none for the channel, a missile's each"
    );
    assert!(dealt > 0, "the missiles hit: {dealt}");
}

/// A spell that needs an anvil, a dummy aura on its caster; the anvil,
/// five yards' reach, ten yards from where Alice comes in.
const AT_THE_ANVIL: u32 = 997_310;
const ANVIL: u32 = 997_311;

fn anvil_data() -> GameData {
    use crate::game::data::{GameObjectSpawn, GameObjectTemplate};
    use crate::game::tests_p3_combat::START;
    let mut d = crate::game::tests_p3_combat::data();
    let mut s = crate::game::tests_p3_entity::spell(
        AT_THE_ANVIL,
        &[crate::game::tests_p3_entity::aura_effect(
            AURA_DUMMY, 1, 0, 1,
        )],
    );
    s.requires_spell_focus = 1;
    d.spells.spells.insert(AT_THE_ANVIL, s);
    let mut t = GameObjectTemplate {
        entry: ANVIL,
        kind: 8,
        display_id: 1,
        name: "Anvil".into(),
        size: 1.0,
        ..Default::default()
    };
    t.data[0] = 1;
    t.data[1] = 5;
    d.gameobject_templates.insert(ANVIL, t);
    d.gameobject_spawns
        .entry(0)
        .or_default()
        .push(GameObjectSpawn {
            guid: 97,
            entry: ANVIL,
            map: 0,
            x: START.0,
            y: START.1 + 10.0,
            z: START.2,
            state: 1,
            ..Default::default()
        });
    d
}

/// Thistle Tea's "checks the original player before projecting triggered
/// effects" (spell_focus_dbc_test 25): a player's triggered spell that
/// needs a focus fails without one near, told as the cast's failure,
/// and goes by it, as mangos-classic's `SpellStart` checks the focus of
/// every spell (where a triggered one went off anywhere).
#[tokio::test]
async fn a_players_triggered_spell_wants_its_focus() {
    use crate::game::tests_p3_combat::START;
    let realm = harness::realm_with(anvil_data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = world(&realm);
    let away = map
        .query(move |m| {
            m.players.get_mut(&g).unwrap().movement.position = Vector3d {
                x: START.0,
                y: START.1,
                z: START.2,
            };
            spells::cast_triggered(m, g, AT_THE_ANVIL, Some(g));
            kit::has(m, g, AT_THE_ANVIL)
        })
        .await
        .unwrap();
    assert!(!away, "no anvil near: nothing");
    let frame = a
        .wait_frame(
            "a failed SMSG_CAST_RESULT",
            crate::game::map::spells::SMSG_CAST_RESULT,
            |f| f[..4] == AT_THE_ANVIL.to_le_bytes() && f[4] == 2,
        )
        .await;
    assert_eq!(frame[5], spells::fail::REQUIRES_SPELL_FOCUS);
    let by = map
        .query(move |m| {
            m.players.get_mut(&g).unwrap().movement.position = Vector3d {
                x: START.0,
                y: START.1 + 7.0,
                z: START.2,
            };
            spells::cast_triggered(m, g, AT_THE_ANVIL, Some(g));
            kit::has(m, g, AT_THE_ANVIL)
        })
        .await
        .unwrap();
    assert!(by, "by the anvil it goes");
}
