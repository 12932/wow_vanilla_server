//! Round 10, against Thistle Tea's casting tests where wvs differed from
//! both it and mangos-classic (`target_codec_test.exs`,
//! `proc_origin_test.exs`, `proc_origin_logic_test.exs`,
//! `combat_test.exs`, `diminishing_returns_test.exs`): the source location
//! a client sends; what an aura's spell procs and what a cast's or another
//! spell's trigger does; whom each puts in a fight; an item's spell of no
//! family; a triggered cast's end; and an aura's stun and root kept apart
//! for diminishing returns.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, holder, pass};
use crate::game::data::GameData;
use crate::game::data::spells::{self as sd, ProcEvent, SpellDuration, SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::map::spells::{self, Dr, Targets, dr_group};
use crate::game::map::{Map, auras, procs};

/// `CMSG_CAST_SPELL`'s targets as the client lays them out, read as the
/// session reads them.
fn read_targets(flags: u16, fields: &[u8]) -> Option<Targets> {
    use wow_world_messages::vanilla::opcodes::ClientOpcodeMessage;
    let mut body = 116u32.to_le_bytes().to_vec();
    body.extend(flags.to_le_bytes());
    body.extend(fields);
    let mut plain = ((body.len() + 4) as u16).to_be_bytes().to_vec();
    plain.extend(0x12Eu32.to_le_bytes());
    plain.extend(body);
    match ClientOpcodeMessage::read_unencrypted(&plain[..]).ok()? {
        ClientOpcodeMessage::CMSG_CAST_SPELL(c) => {
            Some(crate::game::map::spells::targets_of(&c.targets))
        }
        _ => None,
    }
}

fn floats(v: [f32; 3]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn ground(t: &Targets) -> Option<(f32, f32, f32)> {
    t.dest.map(|d| (d.x, d.y, d.z))
}

/// A source location alone (0x20) is the cast's ground; sent with a
/// destination (0x60), the destination is (Thistle Tea's "source location
/// parses ground-target coordinates" and "prefers destination over
/// source"; mangos-classic's `SpellCastTargets::read` keeping `m_srcPos`).
#[test]
fn a_source_location_is_the_ground_a_destination_is_not_sent_with() {
    let source = read_targets(0x20, &floats([4.0, 5.0, 6.0])).expect("read");
    assert_eq!(ground(&source), Some((4.0, 5.0, 6.0)), "the source");
    let mut both = floats([1.0, 1.0, 1.0]);
    both.extend(floats([2.0, 2.0, 2.0]));
    let both = read_targets(0x60, &both).expect("read");
    assert_eq!(ground(&both), Some((2.0, 2.0, 2.0)), "the destination");
}

/// A proc on harmful spells dealt and taken, of three charges and three
/// seconds' cooldown; the same taking procs' spells; a frost bolt of 10,
/// the same no proc's own, a weapon strike, and a heal of no family.
const PLAIN: u32 = 998_420;
const FROM_PROCS: u32 = 998_421;
const BOLT: u32 = 998_422;
const NOT_A_PROC_BOLT: u32 = 998_423;
const STRIKE: u32 = 998_424;
const HEAL: u32 = 998_425;
/// A proc on a cast's end, of ten charges; a channel of three seconds
/// whose aura casts a bolt each second (Arcane Missiles' kind).
const ON_CAST: u32 = 998_426;
const MISSILES: u32 = 998_427;
const LASTS_3S: u32 = 998;
/// A stun of fifteen seconds.
const STUN: u32 = 998_428;
/// A bolt of a family (a mage's).
const FAMILY_BOLT: u32 = 998_430;
/// Missiles of 20 yards a second: of 10 damage, the same of no threat,
/// and a heal.
const MISSILE: u32 = 998_431;
const QUIET_MISSILE: u32 = 998_432;
const HEAL_MISSILE: u32 = 998_433;
/// Fire spells: one cooling for half a minute, one free, one whose
/// cooldown waits for its event, one of a category cooling for twenty
/// seconds; and a frost one.
const FIRE_LONG: u32 = 998_434;
const FIRE_FREE: u32 = 998_435;
const FIRE_EVENT: u32 = 998_436;
const FIRE_CATEGORY: u32 = 998_437;
const FROST_FREE: u32 = 998_438;
const CATEGORY: u32 = 50;
/// An aura of a rogue's whose cooldown of ten seconds waits for it to go,
/// of category 94; the item that casts it, of a minute's cooldown and ten
/// seconds' for the category; half off the rogue's cooldowns; and a
/// debuff whose cooldown of eight seconds waits for it.
const WAITS: u32 = 998_439;
const WAITING_ITEM: u32 = 998_440;
const HALF_COOLDOWN: u32 = 998_441;
const WAITS_AT: u32 = 998_442;
const ITEM_CATEGORY: u32 = 94;
/// A hundred mana given, and fifty a second for five seconds.
const GIVE_MANA: u32 = 998_443;
const MANA_TICKS: u32 = 998_444;

fn proc_aura(id: u32, flags: u32) -> SpellTemplate {
    let mut s = bare(id, 0);
    s.duration_index = LASTS_LONG;
    s.proc_flags = flags;
    s.proc_chance = 100;
    s.proc_charges = 3;
    effect(&mut s, 0, aura(auras::AURA_DUMMY, 0, 0, 1));
    s
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
    let flags = procs::PROC_DONE_NEGATIVE_SPELL
        | procs::PROC_TAKEN_NEGATIVE_SPELL
        | procs::PROC_DONE_MELEE_SPELL
        | procs::PROC_TAKEN_MELEE_SPELL
        | procs::PROC_DONE_POSITIVE_SPELL
        | procs::PROC_TAKEN_POSITIVE_SPELL;
    let plain = proc_aura(PLAIN, flags);
    let mut from_procs = proc_aura(FROM_PROCS, flags);
    from_procs.attributes[3] |= sd::ATTR_EX3_CAN_PROC_FROM_PROCS;
    d.spells.proc_events.insert(
        PLAIN,
        ProcEvent {
            cooldown: 3_000,
            ..Default::default()
        },
    );
    let mut bolt = bare(BOLT, 4);
    effect(&mut bolt, 0, damage(10));
    let mut not_a_proc = bolt.clone();
    not_a_proc.id = NOT_A_PROC_BOLT;
    not_a_proc.attributes[3] |= sd::ATTR_EX3_NOT_A_PROC;
    let mut family_bolt = bolt.clone();
    family_bolt.id = FAMILY_BOLT;
    family_bolt.family_name = 3;
    family_bolt.family_flags = 1;
    let mut strike = bare(STRIKE, 0);
    strike.dmg_class = 2;
    effect(
        &mut strike,
        0,
        SpellEffect {
            effect: 58,
            base_points: 9,
            base_dice: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    let mut heal = bare(HEAL, 1);
    effect(
        &mut heal,
        0,
        SpellEffect {
            effect: 10,
            base_points: 9,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    let mut on_cast = proc_aura(ON_CAST, flags);
    on_cast.proc_charges = 10;
    d.spells.proc_events.insert(
        ON_CAST,
        ProcEvent {
            ex: procs::EX_CAST_END,
            ..Default::default()
        },
    );
    let mut missiles = bare(MISSILES, 6);
    missiles.attributes[1] |= sd::ATTR_EX_CHANNELED;
    missiles.duration_index = LASTS_3S;
    effect(
        &mut missiles,
        0,
        SpellEffect {
            trigger_spell: NOT_A_PROC_BOLT,
            amplitude: 1_000,
            ..aura(auras::AURA_PERIODIC_TRIGGER_SPELL, 0, 0, 1)
        },
    );
    let mut stun = bare(STUN, 0);
    stun.duration_index = LASTS_15S;
    stun.mechanic = 12;
    effect(&mut stun, 0, aura(auras::AURA_MOD_STUN, 0, 0, 6));
    let mut missile = bolt.clone();
    missile.id = MISSILE;
    missile.speed = 20.0;
    let mut quiet = missile.clone();
    quiet.id = QUIET_MISSILE;
    quiet.attributes[1] |= 0x400;
    let mut heal_missile = heal.clone();
    heal_missile.id = HEAL_MISSILE;
    heal_missile.speed = 20.0;
    heal_missile.effects[0].target_a = 25;
    let fire = |id: u32| {
        let mut s = bare(id, 2);
        effect(&mut s, 0, damage(10));
        s
    };
    let mut on_event = fire(FIRE_EVENT);
    on_event.attributes[0] |= 0x200_0000;
    let mut category = fire(FIRE_CATEGORY);
    category.category = CATEGORY;
    let mut frost = fire(FROST_FREE);
    frost.school = 4;
    let mut waits = bare(WAITS, 0);
    waits.attributes[0] |= 0x200_0000;
    waits.family_name = 8;
    waits.family_flags = 1;
    waits.recovery_time = 10_000;
    waits.category = ITEM_CATEGORY;
    waits.category_recovery_time = 5_000;
    waits.duration_index = LASTS_LONG;
    effect(&mut waits, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
    let mut half = bare(HALF_COOLDOWN, 0);
    half.family_name = 8;
    half.duration_index = LASTS_LONG;
    effect(
        &mut half,
        0,
        SpellEffect {
            item_type: 1,
            ..aura(
                auras::AURA_ADD_PCT_MODIFIER,
                crate::game::map::spells::mods::OP_COOLDOWN as i32,
                -50,
                1,
            )
        },
    );
    let mut waits_at = bare(WAITS_AT, 0);
    waits_at.attributes[0] |= 0x200_0000;
    waits_at.recovery_time = 8_000;
    waits_at.duration_index = LASTS_LONG;
    effect(&mut waits_at, 0, aura(auras::AURA_DUMMY, 0, 1, 6));
    let mut item = crate::game::data::items::ItemTemplate {
        entry: WAITING_ITEM,
        class: 0,
        name: "Waiting Charm".into(),
        display_id: 1000,
        stackable: 1,
        ..Default::default()
    };
    item.spells[0] = crate::game::data::items::ItemSpell {
        id: WAITS,
        cooldown: 60_000,
        category: ITEM_CATEGORY,
        category_cooldown: 10_000,
        ..Default::default()
    };
    d.items.insert(WAITING_ITEM, item);
    let mut give = bare(GIVE_MANA, 6);
    effect(
        &mut give,
        0,
        SpellEffect {
            effect: 30,
            base_points: 99,
            base_dice: 1,
            target_a: 1,
            ..Default::default()
        },
    );
    let mut ticks = bare(MANA_TICKS, 6);
    ticks.duration_index = LASTS_5S;
    effect(
        &mut ticks,
        0,
        SpellEffect {
            amplitude: 1_000,
            ..aura(auras::AURA_PERIODIC_ENERGIZE, 0, 50, 1)
        },
    );
    insert(
        d,
        [
            give,
            ticks,
            waits,
            half,
            waits_at,
            fire(FIRE_LONG),
            fire(FIRE_FREE),
            on_event,
            category,
            frost,
            missile,
            quiet,
            heal_missile,
            plain,
            from_procs,
            bolt,
            not_a_proc,
            family_bolt,
            strike,
            heal,
            on_cast,
            missiles,
            stun,
        ],
    );
}

fn charges(m: &Map, g: Guid, spell: u32) -> u32 {
    holder(m, g, spell).map_or(0, |h| h.charges)
}

/// Each side's charges of `spell`.
fn both(m: &Map, a: Guid, b: Guid, spell: u32) -> (u32, u32) {
    (charges(m, a, spell), charges(m, b, spell))
}

/// Both sides with a fresh plain proc and one taking procs' spells.
fn fresh_procs(m: &mut Map, a: Guid, b: Guid) {
    for g in [a, b] {
        auras::remove_where(m, g, |h| h.spell == PLAIN || h.spell == FROM_PROCS);
        kit::apply(m, g, g, PLAIN);
        kit::apply(m, g, g, FROM_PROCS);
    }
}

/// An aura's bolt (a periodic trigger's) procs neither side's plain proc,
/// spending no charge and starting no cooldown, but does an aura taking
/// procs' spells; an aura's weapon strike spends nothing of its caster's
/// plain proc. An aura's bolt that is no proc's own procs the plain one,
/// and a bolt another spell's effect triggers does, as a cast's
/// (mangos-classic's `IsTriggeredAtSpellProcEvent`; Thistle Tea's "aura
/// damage cannot consume either combatant's charges or start cooldowns",
/// "weapon ability feedback retains the origin restriction" and "channel,
/// trap and class exceptions retain the aura-origin restriction").
#[tokio::test]
async fn an_auras_spell_procs_only_what_takes_procs_spells() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P10C1", "Origin", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[]);
            let k = kit::dummy(m, a, 60, 3.0);
            let cooling = |m: &Map, g: Guid| holder(m, g, PLAIN).and_then(|h| h.proc_ready_at);
            fresh_procs(m, a, k);
            spells::cast_by_aura(m, a, BOLT, Some(k));
            let by_aura = (
                both(m, a, k, PLAIN),
                both(m, a, k, FROM_PROCS),
                cooling(m, a).is_none() && cooling(m, k).is_none(),
            );
            fresh_procs(m, a, k);
            spells::cast_by_aura(m, a, STRIKE, Some(k));
            let strike = charges(m, a, PLAIN);
            fresh_procs(m, a, k);
            spells::cast_by_aura(m, a, NOT_A_PROC_BOLT, Some(k));
            let not_a_proc = both(m, a, k, PLAIN);
            fresh_procs(m, a, k);
            spells::cast_triggered(m, a, BOLT, Some(k));
            let direct = both(m, a, k, PLAIN);
            (by_aura, strike, not_a_proc, direct)
        })
        .await
        .unwrap();
    assert_eq!(out.0, ((3, 3), (2, 2), true), "an aura's bolt");
    assert_eq!(out.1, 3, "an aura's strike");
    assert_eq!(out.2, (2, 2), "no proc's own");
    assert_eq!(out.3, (2, 2), "another spell's trigger, as a cast");
}

/// Whether `g` and creature `k` are in a fight: both in combat, the
/// creature at it.
fn fights(m: &Map, k: Guid, g: Guid) -> bool {
    let in_combat = |x: Guid| {
        m.unit_fields(x).is_some_and(|f| {
            f.has_flag(
                UNIT_FIELD_FLAGS,
                crate::game::map::player::UNIT_FLAG_IN_COMBAT,
            )
        })
    };
    in_combat(k) && in_combat(g) && m.creature(k).is_some_and(|c| c.combat.attacking)
}

/// A bolt another spell's effect triggers (a trap's, a script's) puts its
/// creature in the fight as a cast one does; an aura's doesn't, unless it
/// is no proc's own, its damage then starting it (Thistle Tea's "a direct
/// triggered cast retains ordinary combat behavior" and "proc-triggered
/// spells and damage shields do not initiate combat"; mangos-classic's
/// `CasterHitTargetWithSpell` and `DealDamage`, which start one for any).
#[tokio::test]
async fn another_spells_trigger_starts_a_fight_and_an_auras_doesnt() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P10C2", "Engager", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[]);
            let mut out = Vec::new();
            for (spell, by_aura) in [(BOLT, false), (BOLT, true), (NOT_A_PROC_BOLT, true)] {
                crate::game::map::combat::leave_combat(m, a);
                let k = kit::dummy(m, a, 60, 3.0);
                if by_aura {
                    spells::cast_by_aura(m, a, spell, Some(k));
                } else {
                    spells::cast_triggered(m, a, spell, Some(k));
                }
                out.push(fights(m, k, a));
                crate::game::map::combat::leave_combat(m, k);
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(out, vec![true, false, true]);
}

/// A channel of three seconds whose aura casts a bolt each second raises
/// a cast's end as it begins and one for each bolt: four charges of a
/// proc on a cast's end (Arcane Missiles spending a stack of Unstable
/// Power a missile, as mangos-classic's `Spell::cast` raises
/// `PROC_EX_CAST_END` for every cast; Thistle Tea raises none for a
/// triggered one).
#[tokio::test]
async fn each_of_a_channels_bolts_ends_a_cast() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P10C3", "Missiles", Class::Mage).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[MISSILES]);
            let k = kit::dummy(m, a, 60, 5.0);
            kit::apply(m, a, a, ON_CAST);
            cast_fresh(m, a, MISSILES, at(k));
            let begun = charges(m, a, ON_CAST);
            for _ in 0..4 {
                pass(m, 1_000);
            }
            (begun, charges(m, a, ON_CAST), kit::health(m, k))
        })
        .await
        .unwrap();
    assert_eq!(out.0, 9, "its start");
    assert_eq!(out.1, 6, "and each of three bolts");
    assert_eq!(out.2, 1_000_000 - 30, "three bolts");
}

/// An item's bolt of no class family procs nothing on either side, and
/// its heal nothing of its caster's; one of a family procs as a cast
/// (mangos-classic's `prepareDataForTriggerSystem`: no trigger from a cast
/// item's spell of no family; Thistle Tea's "item healing is suppressed
/// even when its spell has NOT_A_PROC").
#[tokio::test]
async fn an_items_spell_of_no_family_procs_nothing() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P10C4", "Grenadier", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[BOLT, FAMILY_BOLT, HEAL]);
            let k = kit::dummy(m, a, 60, 3.0);
            let item = Some(Guid::new(0x4000_0000_0000_0123));
            let mut out = Vec::new();
            for (spell, t) in [
                (BOLT, at(k)),
                (HEAL, Targets::default()),
                (FAMILY_BOLT, at(k)),
            ] {
                fresh_procs(m, a, k);
                kit::fresh(m, a);
                spells::cast(m, a, spell, t, item);
                out.push(both(m, a, k, PLAIN));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(out, vec![(3, 3), (3, 3), (2, 2)]);
}

/// A stun or a root an aura casts is of a group apart from a cast one's:
/// a cast stun twice at a kobold lasts a half the second time, and an
/// aura's then lasts whole (a root diminishes only between players). A
/// charge's stun stays the cast stuns', and Impact's and Pyroclasm's are
/// the triggered ones however they come (mangos-classic's
/// `GetDiminishingReturnsGroupForSpell` with `m_triggeredByAuraSpell`;
/// Thistle Tea's "separates aura-triggered control and preserves charge
/// stuns").
#[tokio::test]
async fn an_auras_stun_and_root_diminish_apart() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P10C5", "Stunner", Class::Warrior).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[]);
            let k = kit::dummy(m, a, 60, 3.0);
            let left = |m: &Map, spell: u32| {
                let now = m.now;
                holder(m, k, spell)
                    .and_then(|h| h.expires)
                    .map(|t| t.duration_since(now).as_millis() as u64)
            };
            let mut out = Vec::new();
            for (spell, by_aura) in [(STUN, false), (STUN, false), (STUN, true)] {
                auras::remove_spell(m, k, spell, None);
                if by_aura {
                    spells::cast_by_aura(m, a, spell, Some(k));
                } else {
                    spells::cast_triggered(m, a, spell, Some(k));
                }
                out.push(left(m, spell));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        vec![Some(15_000), Some(7_500), Some(15_000)],
        "a cast stun, halved, then an aura's whole"
    );
    let stun = |id: u32| SpellTemplate {
        id,
        mechanic: 12,
        ..Default::default()
    };
    let root = SpellTemplate {
        mechanic: 7,
        ..Default::default()
    };
    assert_eq!(dr_group(&stun(1), true), Some(Dr::TriggeredStun));
    assert_eq!(dr_group(&root, true), Some(Dr::TriggeredRoot));
    assert_eq!(dr_group(&root, false), Some(Dr::ControlledRoot));
    for charge in [7922, 20_615] {
        assert_eq!(dr_group(&stun(charge), true), Some(Dr::ControlledStun));
    }
    for impact in [12_355, 18_093] {
        assert_eq!(dr_group(&stun(impact), false), Some(Dr::TriggeredStun));
    }
}

/// Whether `g` is in combat.
fn in_combat(m: &Map, g: Guid) -> bool {
    m.unit_fields(g).is_some_and(|f| {
        f.has_flag(
            UNIT_FIELD_FLAGS,
            crate::game::map::player::UNIT_FLAG_IN_COMBAT,
        )
    })
}

/// A player's missile at a kobold 20 yards off puts the player in a fight
/// as it leaves, held for its second's flight and half a second, the
/// kobold not yet in it; it is when the missile lands. One of no threat
/// doesn't, nor a heal, nor a kobold's missile at the player (mangos's
/// `Spell::cast` for a delayed spell: `TriggerCombatTimer(timeDelay +
/// 500)`; Thistle Tea's "player projectiles hold through flight with a
/// half-second margin" and "NPC projectiles and helpful, instant or exempt
/// spells do not initiate").
#[tokio::test]
async fn a_players_missile_holds_them_in_a_fight_while_it_flies() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P10C6", "Launcher", Class::Mage).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            m.spells.travel = Some(true);
            kit::ready(m, a, 60, &[MISSILE, QUIET_MISSILE, HEAL_MISSILE]);
            let mut quiet = Vec::new();
            for spell in [QUIET_MISSILE, HEAL_MISSILE] {
                let k = kit::dummy(m, a, 60, 20.0);
                cast_fresh(m, a, spell, at(k));
                quiet.push(in_combat(m, a));
                crate::game::map::combat::leave_combat(m, a);
            }
            let k = kit::dummy(m, a, 60, 20.0);
            kit::fresh(m, k);
            spells::cast(m, k, MISSILE, at(a), None);
            quiet.push(in_combat(m, a));
            let k = kit::dummy(m, a, 60, 20.0);
            let t0 = m.now;
            cast_fresh(m, a, MISSILE, at(k));
            let held = m.players[&a]
                .combat
                .combat_until
                .map(|t| t.duration_since(t0).as_millis() as u64);
            let flying = (in_combat(m, a), in_combat(m, k));
            pass(m, 1_000);
            (quiet, held, flying, in_combat(m, k))
        })
        .await
        .unwrap();
    assert_eq!(
        out.0,
        vec![false, false, false],
        "no threat, a heal, a kobold's"
    );
    assert_eq!(out.1, Some(1_500), "held for its flight and half a second");
    assert_eq!(out.2, (true, false), "the player, not yet the kobold");
    assert!(out.3, "the kobold once it lands");
}

/// A fire lockout of ten seconds tells the client of the fire spells it
/// holds longer than they were held: not one cooling for half a minute,
/// nor one of a category cooling for twenty seconds, nor one whose
/// cooldown waits for its event, nor a frost one (mangos-classic's
/// `Player::LockOutSpells`; Thistle Tea's "preserves longer spell and
/// category cooldowns and deferred entries").
#[tokio::test]
async fn a_lockout_tells_the_client_only_of_what_it_holds_longer() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P10C7", "Locked", Class::Mage).await;
    map.query(move |m| {
        kit::ready(
            m,
            a,
            60,
            &[FIRE_LONG, FIRE_FREE, FIRE_EVENT, FIRE_CATEGORY, FROST_FREE],
        );
        let now = m.now;
        let c = m.caster_mut(a).unwrap();
        c.cooldowns
            .insert(FIRE_LONG, now + std::time::Duration::from_secs(30));
        c.category_cooldowns
            .insert(CATEGORY, now + std::time::Duration::from_secs(20));
        spells::cooldowns::lock_school(m, a, FIRE_FREE, 10_000);
    })
    .await
    .unwrap();
    let told = c
        .wait_for("SMSG_SPELL_COOLDOWN", |msg| match msg {
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_SPELL_COOLDOWN(s)
                if s.guid == a =>
            {
                Some(
                    s.cooldowns
                        .iter()
                        .map(|x| (x.id, x.cooldown_time.as_millis() as u32))
                        .collect::<Vec<_>>(),
                )
            }
            _ => None,
        })
        .await;
    assert_eq!(told, vec![(FIRE_FREE, 10_000)]);
}

/// An item's aura whose cooldown waits for it to go starts, as it goes,
/// the item's minute and its category's ten seconds, halved by the
/// modifier its caster has then (none at the cast); a debuff's waiting
/// cooldown starts as it goes from the kobold it was on, the caster's
/// own (mangos-classic's `SpellAuraHolder` removal: `caster->AddCooldown`,
/// the old item's times and the spell modifiers of the time; Thistle
/// Tea's "retains item overrides and applies current cooldown modifiers
/// once" and "removal requests the caster's timer").
#[tokio::test]
async fn a_waiting_cooldown_starts_as_its_aura_goes_with_the_items_times_and_todays_modifiers() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P10C8", "Waiter", Class::Rogue).await;
    let out = map
        .query(move |m| {
            kit::ready(m, a, 60, &[WAITS_AT]);
            let item = crate::game::map::inventory::add_item(m, a, WAITING_ITEM, 1, false)
                .ok()
                .and_then(|_| {
                    m.players[&a]
                        .items
                        .iter()
                        .find(|i| i.entry == WAITING_ITEM)
                        .map(|i| i.guid)
                });
            let cast = spells::cast(m, a, WAITS, Targets::default(), item);
            let pending = m.caster(a).unwrap().pending_cooldowns.contains(&WAITS);
            kit::apply(m, a, a, HALF_COOLDOWN);
            let now = m.now;
            auras::remove_spell(m, a, WAITS, None);
            let left = |t: Option<std::time::Instant>| {
                t.map(|t| t.saturating_duration_since(now).as_millis() as u64)
            };
            let c = m.caster(a).unwrap();
            let item_times = (
                left(c.cooldowns.get(&WAITS).copied()),
                left(c.category_cooldowns.get(&ITEM_CATEGORY).copied()),
            );
            let k = kit::dummy(m, a, 60, 3.0);
            cast_fresh(m, a, WAITS_AT, at(k));
            let waiting = m.caster(a).unwrap().pending_cooldowns.contains(&WAITS_AT);
            auras::remove_spell(m, k, WAITS_AT, None);
            let at_kobold = left(m.caster(a).unwrap().cooldowns.get(&WAITS_AT).copied());
            (
                (item.is_some(), cast, pending),
                item_times,
                (waiting, at_kobold),
            )
        })
        .await
        .unwrap();
    assert_eq!(out.0, (true, true, true), "cast from the item, waiting");
    assert_eq!(
        out.1,
        (Some(30_000), Some(5_000)),
        "the item's times, halved"
    );
    assert_eq!(out.2, (true, Some(8_000)), "the caster's, from the kobold");
}

/// Mana given to a mage a kobold fights is half its worth in threat on
/// her: fifty for a hundred given, and twenty-five a tick of fifty
/// (mangos-classic's `EnergizeBySpell` and `PeriodicTick`, `threatAssist`
/// of half, any power; Thistle Tea gives mana none, and rage half of what
/// it gained).
#[tokio::test]
async fn mana_given_is_half_its_worth_in_threat() {
    let realm = fast_realm(data);
    let (_c, a, map) = human(&realm, "P10C9", "Wellspring", Class::Mage).await;
    let out = map
        .query(move |m| {
            m.spells.rolls = Some(false);
            kit::ready(m, a, 60, &[GIVE_MANA, MANA_TICKS]);
            kit::set_power(m, a, 0, 1_000);
            let k = kit::dummy(m, a, 60, 3.0);
            m.creature_mut(k).unwrap().threat = vec![(a, 0.0)];
            let threat = |m: &Map| {
                m.creature(k)
                    .and_then(|c| c.threat.iter().find(|(g, _)| *g == a).map(|(_, t)| *t))
            };
            cast_fresh(m, a, GIVE_MANA, Targets::default());
            let given = threat(m);
            cast_fresh(m, a, MANA_TICKS, Targets::default());
            pass(m, 1_000);
            (given, threat(m))
        })
        .await
        .unwrap();
    assert_eq!(out, (Some(50.0), Some(75.0)));
}
