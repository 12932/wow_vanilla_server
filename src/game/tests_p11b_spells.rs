//! Round 11's 11b, against Thistle Tea's spell-hit and proc tests where
//! wvs differed from mangos-classic (`triggered_hit_dbc_test.exs`,
//! `casting_test.exs`, `spell_objects_test.exs`,
//! `equipment_auras_test.exs`): a travelling spell's hit rolled as it goes
//! and kept; a channel's object; the bearer casting its aura's trigger of
//! no unit target; a missile's bonus before its cast spends a stack; and
//! an item's or an enchantment's proc cast from its item.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, health, pass, set_health};
use crate::game::data::GameData;
use crate::game::data::items::{ItemSpell, ItemTemplate};
use crate::game::data::spells::{self as sd, SpellDuration, SpellEffect};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::inventory::{SLOT_MAIN_HAND, SLOT_OFF_HAND};
use crate::game::map::spells;
use crate::game::map::trinkets::{UNSTABLE_POWER, UNSTABLE_POWER_BONUS};
use crate::game::map::{Map, auras, enchant, procs};

/// A frost bolt of 10 that flies at 20 yards a second.
const BOLT: u32 = 991_500;
/// Spells of the target's: its attackers' frost spells hit 100% less.
const WARDED: u32 = 991_501;
/// A channel of five seconds healing the pet; the same healing the pet
/// and a dummy on its caster (Health Funnel's kind).
const MEND: u32 = 991_502;
const FUNNEL: u32 = 991_503;
/// A channel of no end, its dummy aura on the kobold its script names
/// (Belnistrasz's idol's).
const IDOL: u32 = 991_504;
/// An aura that heals its bearer's caster each second (a self heal), and
/// one that bolts the bearer's selection.
const HEALS_SELF: u32 = 991_505;
const SELF_HEAL: u32 = 991_506;
const BOLTS_ENEMY: u32 = 991_507;
const ENEMY_BOLT: u32 = 991_508;
/// A channel of three seconds whose aura at its caster casts a fire
/// missile each second at the selection (Arcane Missiles' kind).
const MISSILES: u32 = 991_509;
const FIRE_MISSILE: u32 = 991_510;
/// A buff a weapon's blow casts; a trinket's proc and a set's, each a zap.
const MIGHT: u32 = 991_511;
const CHARM: u32 = 991_512;
const ZAP: u32 = 991_513;
const SET_CHARM: u32 = 991_514;
const SET_ZAP: u32 = 991_515;
const LASTS_3S: u32 = 991;
const PET: u32 = 99_611;
const BLADE: u32 = 63_710;
const OFF_BLADE: u32 = 63_711;
const TRINKET: u32 = 63_712;
const SET_PIECE: u32 = 63_713;
const SET: u32 = 991;

fn heal(target: u32) -> SpellEffect {
    SpellEffect {
        effect: 10,
        base_points: 99,
        base_dice: 1,
        target_a: target,
        ..Default::default()
    }
}

fn proc_trigger(id: u32, trigger: u32) -> sd::SpellTemplate {
    let mut s = bare(id, 0);
    s.duration_index = LASTS_LONG;
    s.proc_flags = procs::PROC_DONE_MELEE;
    s.proc_chance = 100;
    effect(
        &mut s,
        0,
        SpellEffect {
            trigger_spell: trigger,
            ..aura(auras::AURA_PROC_TRIGGER_SPELL, 0, 0, 1)
        },
    );
    s
}

fn trinket(entry: u32, spell: u32, set_id: u32) -> ItemTemplate {
    let mut t = ItemTemplate {
        entry,
        class: 4,
        name: format!("Trinket {entry}"),
        display_id: 1000,
        inventory_type: 12,
        stackable: 1,
        set_id,
        ..Default::default()
    };
    if spell != 0 {
        t.spells[0] = ItemSpell {
            id: spell,
            trigger: crate::game::map::inventory::ITEM_SPELL_ON_EQUIP,
            ..Default::default()
        };
    }
    t
}

fn weapon(entry: u32, spell: u32) -> ItemTemplate {
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
    t.spells[0] = ItemSpell {
        id: spell,
        trigger: 2,
        ..Default::default()
    };
    t
}

fn data(d: &mut GameData) {
    d.spells.ranges.insert(1, (0.0, 40.0));
    d.spells.durations.insert(
        LASTS_3S,
        SpellDuration {
            base: 3_000,
            per_level: 0,
            max: 3_000,
        },
    );
    d.spells.durations.insert(
        992,
        SpellDuration {
            base: -1,
            per_level: 0,
            max: -1,
        },
    );
    let mut pet = crate::game::tests_p6_wspell::creature(PET, 1, 11);
    pet.unit_class = 1;
    d.creature_templates.insert(PET, pet);
    let mut bolt = bare(BOLT, 4);
    bolt.speed = 20.0;
    effect(&mut bolt, 0, damage(10));
    let mut warded = bare(WARDED, 0);
    warded.duration_index = LASTS_LONG;
    effect(
        &mut warded,
        0,
        aura(auras::AURA_MOD_ATTACKER_SPELL_HIT_CHANCE, 0x10, -100, 1),
    );
    let channel = |id: u32| {
        let mut s = bare(id, 0);
        s.attributes[1] |= sd::ATTR_EX_CHANNELED;
        s.duration_index = LASTS_5S;
        s
    };
    let mut mend = channel(MEND);
    effect(
        &mut mend,
        0,
        SpellEffect {
            amplitude: 1_000,
            ..aura(auras::AURA_PERIODIC_HEAL, 0, 10, 5)
        },
    );
    let mut funnel = mend.clone();
    funnel.id = FUNNEL;
    effect(&mut funnel, 1, aura(auras::AURA_DUMMY, 0, 0, 1));
    let mut idol = channel(IDOL);
    idol.duration_index = 992;
    effect(&mut idol, 0, aura(auras::AURA_DUMMY, 0, 0, 38));
    d.spells.script_targets.insert(IDOL, vec![(1, KOBOLD)]);
    let periodic = |id: u32, trigger: u32| {
        let mut s = bare(id, 0);
        s.duration_index = LASTS_LONG;
        effect(
            &mut s,
            0,
            SpellEffect {
                trigger_spell: trigger,
                amplitude: 1_000,
                ..aura(auras::AURA_PERIODIC_TRIGGER_SPELL, 0, 0, 1)
            },
        );
        s
    };
    let mut self_heal = bare(SELF_HEAL, 1);
    effect(&mut self_heal, 0, heal(1));
    let mut enemy_bolt = bare(ENEMY_BOLT, 4);
    effect(&mut enemy_bolt, 0, damage(10));
    let mut missiles = periodic(MISSILES, FIRE_MISSILE);
    missiles.attributes[1] |= sd::ATTR_EX_CHANNELED;
    missiles.duration_index = LASTS_3S;
    let mut fire_missile = bare(FIRE_MISSILE, 2);
    fire_missile.attributes[3] |= sd::ATTR_EX3_NOT_A_PROC;
    effect(&mut fire_missile, 0, damage(10));
    // Unstable Power as the real data has it (`tests_p7_spell_procs`).
    let mut unstable = bare(UNSTABLE_POWER, 0);
    unstable.duration_index = LASTS_LONG;
    effect(&mut unstable, 0, aura(auras::AURA_DUMMY, 0, 0, 1));
    let mut bonus = bare(UNSTABLE_POWER_BONUS, 0);
    bonus.duration_index = LASTS_LONG;
    bonus.stack_amount = 12;
    bonus.proc_flags = 0x15550;
    bonus.proc_chance = 100;
    effect(
        &mut bonus,
        0,
        aura(auras::AURA_MOD_DAMAGE_DONE, 0x7e, 17, 1),
    );
    d.spells.proc_events.insert(
        UNSTABLE_POWER_BONUS,
        sd::ProcEvent {
            ex: procs::EX_CAST_END,
            ..Default::default()
        },
    );
    let mut might = bare(MIGHT, 0);
    might.duration_index = LASTS_LONG;
    might.proc_chance = 100;
    effect(&mut might, 0, aura(auras::AURA_MOD_STAT, 0, 10, 1));
    let mut zap = bare(ZAP, 2);
    effect(&mut zap, 0, damage(1));
    let mut set_zap = zap.clone();
    set_zap.id = SET_ZAP;
    insert(
        d,
        [
            bolt,
            warded,
            mend,
            funnel,
            idol,
            periodic(HEALS_SELF, SELF_HEAL),
            self_heal,
            periodic(BOLTS_ENEMY, ENEMY_BOLT),
            enemy_bolt,
            missiles,
            fire_missile,
            unstable,
            bonus,
            might,
            proc_trigger(CHARM, ZAP),
            zap,
            proc_trigger(SET_CHARM, SET_ZAP),
            set_zap,
        ],
    );
    for t in [
        weapon(BLADE, MIGHT),
        weapon(OFF_BLADE, MIGHT),
        trinket(TRINKET, CHARM, 0),
        trinket(SET_PIECE, 0, SET),
    ] {
        d.items.insert(t.entry, t);
    }
    d.crafts.item_sets.insert(
        SET,
        crate::game::data::crafts::ItemSet {
            id: SET,
            items: vec![SET_PIECE],
            spells: vec![(SET_CHARM, 1)],
            ..Default::default()
        },
    );
}

/// The first go of `spell` `c` is told of.
async fn go(c: &mut Client, spell: u32) -> Option<SMSG_SPELL_GO> {
    c.gets(Duration::from_millis(500), |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == spell => Some(g.clone()),
        _ => None,
    })
    .await
}

/// Thistle Tea's "a nonbinary trigger reports its saved resist at launch
/// and impact" and "successful deliveries retain the launch result when
/// target hit modifiers change", as mangos's `Spell::AddUnitTarget` rolls
/// a spell's hit when it takes its target and keeps it to its impact: a
/// travelling bolt at a kobold that wards off frost is resisted in its go
/// and resisted as it lands, though the ward is gone by then, the go its
/// only word of it (mangos's `WriteSpellGoTargets`); one at a
/// kobold without it hits in its go and lands, though the kobold is warded
/// while it flies.
#[tokio::test]
async fn a_travelling_spells_hit_is_rolled_as_it_goes_and_kept() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P11B1", "Launcher", Class::Mage).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            m.spells.travel = Some(true);
            kit::ready(m, a, 60, &[BOLT]);
            let warded = kit::dummy(m, a, 60, 10.0);
            kit::apply(m, warded, warded, WARDED);
            cast_fresh(m, a, BOLT, at(warded));
            auras::remove_spell(m, warded, WARDED, None);
            pass(m, 1_000);
            let resisted = health(m, warded);
            let open = kit::dummy(m, a, 60, 10.0);
            cast_fresh(m, a, BOLT, at(open));
            kit::apply(m, open, open, WARDED);
            pass(m, 1_000);
            (warded, resisted, open, health(m, open))
        })
        .await
        .unwrap();
    let (warded, resisted, open, landed) = out;
    let first = go(&mut c, BOLT).await.expect("the first go");
    assert_eq!(first.hits, Vec::<Guid>::new(), "no hit at launch");
    assert_eq!(
        first
            .misses
            .iter()
            .map(|x| (x.target, x.miss_info))
            .collect::<Vec<_>>(),
        vec![(warded, SpellMiss_SpellMissInfo::Resist)],
        "its resist at launch"
    );
    let told_again = crate::game::tests_p3_entity::gets_frame(
        &mut c,
        crate::game::map::spells::outcome::SMSG_SPELLLOGMISS,
        |b| b[..4] == BOLT.to_le_bytes(),
    )
    .await;
    assert!(!told_again, "the go told it: no miss log at impact");
    assert_eq!(resisted, 1_000_000, "nothing landed");
    let second = go(&mut c, BOLT).await.expect("the second go");
    assert_eq!(second.hits, vec![open], "a hit at launch");
    assert!(second.misses.is_empty());
    assert_eq!(landed, 1_000_000 - 10, "landed, the ward come too late");
}

/// Thistle Tea's "keeps pet channels active when the client selects the
/// caster" and "aims a channel cast on oneself at the scripted creature
/// its aura lands on", as mangos's `SendChannelStart` takes the first unit
/// its first effect lands on: a heal on the pet channelled with its
/// caster selected (Mend Pet's kind), or an enemy (Health Funnel's, also on
/// its caster), is at the pet, and cancelled takes the pet's heal off; a
/// channel cast on oneself whose aura goes on the kobold its script names
/// is at the kobold, and cancelled takes the kobold's aura off. A channel
/// at an enemy it lands on is at the enemy, as before.
#[tokio::test]
async fn a_channels_object_is_the_unit_its_first_effect_lands_on() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P11B2", "Channeler", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[MEND, FUNNEL, IDOL, MISSILES]);
            let to = crate::game::tests_p6_wspell::off(m, a, 2.0, 0.0);
            let pet = crate::game::map::pets::testing::spawn(
                m,
                a,
                PET,
                crate::game::map::pets::Kind::Hunter,
                60,
                to,
                0.0,
            );
            m.players.get_mut(&a).unwrap().owned.pet = Some(pet);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let object = |m: &Map| {
                m.unit_fields(a)
                    .map(|f| f.get_guid(UNIT_FIELD_CHANNEL_OBJECT))
                    .unwrap_or_default()
            };
            let mut out = Vec::new();
            for (spell, t) in [(MEND, a), (FUNNEL, kobold), (FUNNEL, a)] {
                let began = cast_fresh(m, a, spell, at(t));
                let at_pet = (began, object(m) == pet, kit::has(m, pet, spell));
                spells::interrupt(m, a);
                out.push((at_pet, kit::has(m, pet, spell)));
            }
            cast_fresh(m, a, IDOL, at(a));
            let idol = (object(m) == kobold, kit::has(m, kobold, IDOL));
            spells::interrupt(m, a);
            let idol = (idol, kit::has(m, kobold, IDOL));
            cast_fresh(m, a, MISSILES, at(kobold));
            let missiles = object(m) == kobold;
            spells::interrupt(m, a);
            (out, idol, missiles)
        })
        .await
        .unwrap();
    let (pets, idol, missiles) = out;
    assert_eq!(
        pets,
        vec![((true, true, true), false); 3],
        "at the pet, and its heal ended with it"
    );
    assert_eq!(idol, ((true, true), false), "at the kobold, its aura ended");
    assert!(missiles, "a channel aimed at an enemy is at it");
}

/// Thistle Tea's "the bearer of an aura from an object resolves its object
/// targets itself", as mangos's `Aura::TriggerSpell` has its bearer cast
/// a trigger of no unit target, and any trigger of an object's aura: a
/// kobold's aura on a player that heals its caster heals the player, cast
/// by the player; an object's does too; and an object's aura that bolts an
/// enemy bolts the player's selection, cast by the player.
#[tokio::test]
async fn the_bearer_casts_its_auras_trigger_of_no_unit_target() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P11B3", "Bearer", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            let stone = crate::game::guid::game_object(100, 77);
            m.unit_fields_mut(a)
                .unwrap()
                .set_guid(UNIT_FIELD_TARGET, kobold);
            let mut out = Vec::new();
            for caster in [kobold, stone] {
                set_health(m, a, 100);
                set_health(m, kobold, 100);
                kit::apply(m, caster, a, HEALS_SELF);
                kit::tick_now(m, a, HEALS_SELF);
                auras::remove_spell(m, a, HEALS_SELF, None);
                out.push((health(m, a), health(m, kobold)));
            }
            set_health(m, kobold, 1_000_000);
            kit::apply(m, stone, a, BOLTS_ENEMY);
            kit::tick_now(m, a, BOLTS_ENEMY);
            (out, 1_000_000 - health(m, kobold))
        })
        .await
        .unwrap();
    let (heals, bolted) = out;
    assert_eq!(heals, vec![(200, 100), (200, 100)], "the bearer healed");
    for spell in [SELF_HEAL, SELF_HEAL, ENEMY_BOLT] {
        let g = go(&mut c, spell).await.expect("its go");
        assert_eq!(g.caster, a, "cast by the bearer");
    }
    assert_eq!(bolted, 10, "the bearer's selection bolted");
}

/// Thistle Tea's "Arcane Missiles snapshots its bonus before the child
/// cast spends one Unstable Power stack": each missile a channel's aura
/// casts spends a stack of Unstable Power (mangos-classic's `Spell::cast`
/// raising `PROC_EX_CAST_END` for a triggered cast too, round 10's 10b),
/// and its damage has the stack it spends: of two stacks, the first missile
/// has both, the second the last, the third none.
#[tokio::test]
async fn a_missile_takes_its_bonus_before_spending_a_stack() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P11B4", "Missiles", Class::Mage).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[MISSILES]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            m.unit_fields_mut(a)
                .unwrap()
                .set_guid(UNIT_FIELD_TARGET, kobold);
            spells::cast_triggered(m, a, UNSTABLE_POWER, Some(a));
            if let Some(h) = auras::auras_mut(m, a).and_then(|x| {
                x.holders
                    .iter_mut()
                    .find(|h| h.spell == UNSTABLE_POWER_BONUS)
            }) {
                h.stacks = 2;
            }
            let stacks = |m: &Map| kit::holder(m, a, UNSTABLE_POWER_BONUS).map_or(0, |h| h.stacks);
            cast_fresh(m, a, MISSILES, at(kobold));
            let mut each = Vec::new();
            for _ in 0..3 {
                let hp = health(m, kobold);
                pass(m, 1_000);
                each.push((stacks(m), hp - health(m, kobold)));
            }
            each
        })
        .await
        .unwrap();
    let stacks: Vec<u32> = out.iter().map(|x| x.0).collect();
    assert_eq!(stacks, vec![1, 0, 0], "a stack a missile");
    let per_stack = out[1].1 - out[2].1;
    assert_eq!(out[2].1, 10, "no bonus left");
    assert!(
        per_stack > 0,
        "the last stack's bonus had by its missile: {out:?}"
    );
    assert!(
        out[0].1.abs_diff(10 + 2 * per_stack) <= 1,
        "both stacks' by the first: {out:?}"
    );
}

/// Thistle Tea's "item and enchant procs retain their item identity while
/// sets have no casting item", as mangos's `CastItemCombatSpell` and
/// `TriggerProccedSpell` cast with the item: a weapon's blow casts its
/// buff from the weapon, so a main hand's and an off hand's are two; a
/// trinket's proc casts from the trinket; a set's, from no item.
#[tokio::test]
async fn an_items_proc_is_cast_from_its_item_and_a_sets_from_none() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P11B5", "Wielder", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[]);
            m.players
                .get_mut(&a)
                .unwrap()
                .spells
                .push(crate::game::map::derive::SPELL_DUAL_WIELD);
            let blade = crate::game::tests_loot::equip(m, a, BLADE, SLOT_MAIN_HAND);
            let off = crate::game::tests_loot::equip(m, a, OFF_BLADE, SLOT_OFF_HAND);
            let charm = crate::game::tests_loot::equip(m, a, TRINKET, 12);
            crate::game::tests_loot::equip(m, a, SET_PIECE, 13);
            crate::game::map::inventory::sync_gear(m, a);
            let kobold = kit::dummy(m, a, 60, 3.0);
            enchant::FORCED_PROC_ROLL.with(|f| f.set(Some(0.0)));
            for offhand in [false, true] {
                procs::trigger(
                    m,
                    procs::Event {
                        actor: a,
                        target: kobold,
                        actor_flags: procs::PROC_DONE_MELEE,
                        target_flags: procs::PROC_TAKEN_MELEE,
                        ex: procs::EX_NORMAL_HIT,
                        damage: 10,
                        school: 1,
                        offhand,
                        ..Default::default()
                    },
                );
            }
            enchant::FORCED_PROC_ROLL.with(|f| f.set(None));
            let mut mights: Vec<Option<Guid>> = auras::auras(m, a)
                .unwrap()
                .holders
                .iter()
                .filter(|h| h.spell == MIGHT)
                .map(|h| h.item)
                .collect();
            mights.sort_by_key(|g| g.map(|g| g.guid()));
            let mut both = vec![Some(blade), Some(off)];
            both.sort_by_key(|g| g.map(|g| g.guid()));
            (mights == both, blade, charm)
        })
        .await
        .unwrap();
    let (two, blade, charm) = out;
    assert!(two, "a buff from each weapon");
    assert_eq!(go(&mut c, MIGHT).await.unwrap().cast_item, blade);
    assert_eq!(go(&mut c, ZAP).await.unwrap().cast_item, charm);
    assert_eq!(go(&mut c, SET_ZAP).await.unwrap().cast_item, Guid::zero());
}
