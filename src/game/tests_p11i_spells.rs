//! Round 11's test-only rows on spells (Thistle Tea's `cast_context_test`,
//! `caster_location_test`, `casting_movement_test`, `casting_test`,
//! `custom_spell_test` and `triggered_hit_dbc_test`): a normalized blow at
//! each weapon kind's speed with a weapon's talent, a summon's compass
//! spot turned with its caster, a turn ending a channel's auras and their
//! ticks, a resisted spell doing nothing, costs taken only at a cast's end,
//! a script's points leaving the other effects and the template as they
//! were, and a child spell taking its own family's hit modifier.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit, at, cast_fresh, casting, pass};
use crate::game::data::GameData;
use crate::game::data::items::{ItemSpell, ItemTemplate};
use crate::game::data::spells::{ATTR_EX_CHANNELED, ATTR_EX3_ALWAYS_HIT, Enchantment, SpellEffect};
use crate::game::fields::*;
use crate::game::map::auras::*;
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, pets};

async fn on<T: Send + 'static>(
    map: &crate::game::map::MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

fn weapon(entry: u32, subclass: u32, inventory_type: u32) -> ItemTemplate {
    let mut t = ItemTemplate {
        entry,
        class: 2,
        subclass,
        name: format!("Weapon {entry}"),
        display_id: 1000,
        inventory_type,
        delay: 2000,
        stackable: 1,
        ..Default::default()
    };
    t.damages[0] = (10.0, 10.0, 0);
    t
}

const SWORD: u32 = 64_101;
const DAGGER: u32 = 64_102;
const GREATSWORD: u32 = 64_103;
/// A normalized weapon blow (`SPELL_EFFECT_NORMALIZED_WEAPON_DMG`) of no
/// more than the weapon's.
const NORMALIZED: u32 = 997_601;
/// A quarter more physical damage with one kind of weapon: a talent's
/// (`SPELL_AURA_MOD_DAMAGE_PERCENT_DONE` with its weapon class and
/// subclass mask), for each of the three.
const TALENT: [u32; 3] = [997_602, 997_603, 997_604];

fn weapons(d: &mut GameData) {
    for (entry, subclass, inv) in [(SWORD, 7, 13), (DAGGER, 15, 13), (GREATSWORD, 8, 17)] {
        d.items.insert(entry, weapon(entry, subclass, inv));
    }
    let mut blow = bare(NORMALIZED, 0);
    blow.dmg_class = 2;
    blow.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    effect(
        &mut blow,
        0,
        SpellEffect {
            effect: spells::EFFECT_NORMALIZED_WEAPON_DMG,
            base_points: -1,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        },
    );
    let mut all = vec![blow];
    for (id, subclass) in TALENT.into_iter().zip([7, 15, 8]) {
        let mut t = bare(id, 0);
        t.duration_index = LASTS_LONG;
        t.equipped_item_class = 2;
        t.equipped_item_subclass_mask = 1 << subclass;
        effect(&mut t, 0, aura(AURA_MOD_DAMAGE_PERCENT_DONE, 1, 25, 1));
        all.push(t);
    }
    insert(d, all);
}

/// Thistle Tea's "enchanted weapons retain their normalization and
/// weapon-specific talent bonuses": a normalized blow is the weapon's
/// damage and its attack power at its kind's speed, 2.4 a one-hander's,
/// 1.7 a dagger's and 3.3 a two-hander's (mangos's `GetAPMultiplier`),
/// whatever its own speed; a talent for its kind of weapon puts a quarter
/// on it, and one for another kind nothing. (The enchant half is Thistle
/// Tea's packing of an item's enchantments in its visible entry: wvs reads
/// the weapon's template.)
#[tokio::test]
async fn a_normalized_blow_is_at_its_weapon_kinds_speed_with_its_kinds_talent() {
    let realm = fast_realm(weapons);
    let (_c, g, map) = human(&realm, "P11S1", "Normal", Class::Warrior).await;
    let out = on(&map, move |m| {
        kit::ready(m, g, 60, &[NORMALIZED]);
        let k = kit::dummy(m, g, 60, 2.0);
        let mut out = Vec::new();
        for (entry, speed, mine, other) in [
            (SWORD, 2.4f32, TALENT[0], TALENT[1]),
            (DAGGER, 1.7, TALENT[1], TALENT[2]),
            (GREATSWORD, 3.3, TALENT[2], TALENT[0]),
        ] {
            crate::game::tests_p3_combat::equip(m, g, entry, 15);
            let blow = |m: &mut Map| {
                kit::set_health(m, k, 1_000_000);
                cast_fresh(m, g, NORMALIZED, at(k));
                1_000_000 - kit::health(m, k)
            };
            let ap = m.players[&g].derived.attack_power as f32;
            let expected = 10.0 + ap / 14.0 * speed;
            let plain = blow(m);
            kit::apply(m, g, g, other);
            let others = blow(m);
            remove_spell(m, g, other, None);
            kit::apply(m, g, g, mine);
            let talented = blow(m);
            remove_spell(m, g, mine, None);
            out.push((entry, expected, plain, others, talented));
        }
        out
    })
    .await;
    for (entry, expected, plain, others, talented) in out {
        assert_eq!(plain, expected as u32, "{entry}: its kind's speed");
        assert_eq!(others, plain, "{entry}: another kind's talent");
        assert_eq!(talented, (expected * 1.25) as u32, "{entry}: its talent");
    }
}

/// A wild summon at each of the compass spots (implicit targets 32, 41 to
/// 44, 47 to 50) and its radius.
const SUMMONS: [(u32, u32); 9] = [
    (997_611, 47),
    (997_612, 48),
    (997_613, 49),
    (997_614, 50),
    (997_615, 41),
    (997_616, 42),
    (997_617, 43),
    (997_618, 44),
    (997_619, 32),
];
const IMP: u32 = 64_110;
const TEN_YARDS: u32 = 997;

fn summons(d: &mut GameData) {
    d.creature_templates.insert(
        IMP,
        crate::game::data::CreatureTemplate {
            entry: IMP,
            name: "Imp".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 1,
            level_max: 1,
            faction: 35,
            ..Default::default()
        },
    );
    d.spells.radii.insert(TEN_YARDS, 10.0);
    let mut all = Vec::new();
    for (id, target) in SUMMONS {
        let mut s = bare(id, 0);
        s.dmg_class = 0;
        effect(
            &mut s,
            0,
            SpellEffect {
                effect: pets::EFFECT_SUMMON_WILD,
                misc_value: IMP as i32,
                base_points: 0,
                base_dice: 1,
                target_a: target,
                radius_index: TEN_YARDS,
                ..Default::default()
            },
        );
        all.push(s);
    }
    insert(d, all);
}

/// Thistle Tea's "rotates compass offsets with the caster": a summon at a
/// compass spot stands at its radius from its caster, its angle turned by
/// the caster's facing (mangos's `TARGET_LOCATION_CASTER_FRONT` and the
/// rest): facing north, ahead is north, behind south, the left west, the
/// right east, the diagonals and the minion's spot (32, the front left)
/// between them; its height the caster's.
#[tokio::test]
async fn a_summons_compass_spot_turns_with_its_caster() {
    let realm = fast_realm(summons);
    let (_c, g, map) = human(&realm, "P11S2", "Compass", Class::Warlock).await;
    let out = on(&map, move |m| {
        let ids: Vec<u32> = SUMMONS.iter().map(|s| s.0).collect();
        kit::ready(m, g, 60, &ids);
        let quarter = std::f32::consts::FRAC_PI_2;
        m.players.get_mut(&g).unwrap().movement.orientation = quarter;
        let me = m.players[&g].position();
        let mut out = Vec::new();
        for (id, target) in SUMMONS {
            let before: Vec<Guid> = m.creatures.iter().map(|(_, c)| c.guid).collect();
            spells::cast_triggered(m, g, id, Some(g));
            let new = m
                .creatures
                .iter()
                .find(|(_, c)| c.entry == IMP && !before.contains(&c.guid))
                .map(|(_, c)| c.position);
            out.push((target, me, new));
        }
        out
    })
    .await;
    let d = 50f32.sqrt();
    for (target, me, new) in out {
        let new = new.unwrap_or_else(|| panic!("{target}: summoned"));
        // (ahead, to the left) of a caster facing north: x east, y north.
        let (ahead, left) = match target {
            47 => (10.0, 0.0),
            48 => (-10.0, 0.0),
            49 => (0.0, 10.0),
            50 => (0.0, -10.0),
            41 => (d, -d),
            42 => (-d, -d),
            43 => (-d, d),
            44 | 32 => (d, d),
            _ => unreachable!(),
        };
        let (dx, dy) = (new.x - me.x, new.y - me.y);
        assert!(
            (dx - -left).abs() < 0.01 && (dy - ahead).abs() < 0.01,
            "{target}: ({dx}, {dy})"
        );
        assert_eq!(new.z, me.z, "{target}: the caster's height");
    }
}

/// A five-second channel at an enemy that a turn breaks, and one healing
/// its caster every second that a turn breaks.
const STARE: u32 = 997_621;
const MEND: u32 = 997_622;

fn channels(d: &mut GameData) {
    d.spells.ranges.insert(997, (0.0, 20.0));
    let mut stare = bare(STARE, 5);
    stare.range_index = 997;
    stare.attributes[1] |= ATTR_EX_CHANNELED;
    stare.duration_index = LASTS_5S;
    stare.channel_interrupt_flags = 0x10;
    effect(
        &mut stare,
        0,
        SpellEffect {
            amplitude: 1_000,
            ..aura(AURA_PERIODIC_DAMAGE, 0, 10, 6)
        },
    );
    let mut mend = stare.clone();
    mend.id = MEND;
    mend.effects[0] = SpellEffect {
        amplitude: 1_000,
        bonus_coefficient: -1.0,
        ..aura(AURA_OBS_MOD_HEALTH, 0, 7, 1)
    };
    insert(d, [stare, mend]);
}

/// Thistle Tea's "turning cancels channel auras and their future ticks": a
/// turn ends a channel that says so (`AURA_INTERRUPT_FLAG_TURNING`), its
/// aura on its target and on its caster gone with it, the channel fields
/// cleared, and no tick of it lands after.
#[tokio::test]
async fn a_turn_ends_a_channels_auras_and_their_ticks() {
    let realm = fast_realm(channels);
    let (c, a, map) = human(&realm, "P11S3", "Turner", Class::Warlock).await;
    for (spell, on_self) in [(STARE, false), (MEND, true)] {
        let kobold = on(&map, move |m| {
            kit::ready(m, a, 60, &[spell]);
            let kobold = kit::dummy(m, a, 60, 5.0);
            kit::set_health(m, a, 100);
            assert!(cast_fresh(m, a, spell, at(kobold)));
            kobold
        })
        .await;
        let held = if on_self { a } else { kobold };
        assert!(on(&map, move |m| kit::has(m, held, spell)).await);
        let (pos, o) = on(&map, move |m| {
            let p = &m.players[&a];
            (p.position(), p.movement.orientation)
        })
        .await;
        c.send(MSG_MOVE_SET_FACING_Client {
            info: MovementInfo {
                flags: MovementInfo_MovementFlags::empty(),
                timestamp: 0,
                position: pos,
                orientation: o + 1.0,
                fall_time: 0,
            },
        })
        .await;
        let deadline = tokio::time::Instant::now() + crate::game::harness::WAIT;
        while on(&map, move |m| casting(m, a)).await.is_some() {
            assert!(tokio::time::Instant::now() < deadline, "the turn ended it");
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        let out = on(&map, move |m| {
            let f = &m.players[&a].fields;
            let fields = (
                f.get_guid(UNIT_FIELD_CHANNEL_OBJECT),
                f.get_u32(UNIT_CHANNEL_SPELL),
            );
            let gone = !kit::has(m, held, spell);
            let before = (kit::health(m, a), kit::health(m, kobold));
            pass(m, 3_000);
            (
                fields,
                gone,
                before,
                (kit::health(m, a), kit::health(m, kobold)),
            )
        })
        .await;
        let (fields, gone, before, after) = out;
        assert_eq!(fields, (Guid::zero(), 0), "{spell}: the channel's fields");
        assert!(gone, "{spell}: its aura");
        assert_eq!(after, before, "{spell}: no tick after");
    }
}

/// Frostbolt's like (frost, magic), a ward that turns every spell back,
/// a hit chance a hundred less, and a spell of no damage class.
const FROST: u32 = 997_631;
const PLAIN: u32 = 997_632;
const WARD: u32 = 997_633;
const CLUMSY: u32 = 997_634;

fn resists(d: &mut GameData) {
    let mut frost = bare(FROST, 4);
    effect(&mut frost, 0, damage(50));
    let mut plain = bare(PLAIN, 4);
    plain.dmg_class = 0;
    effect(&mut plain, 0, damage(50));
    let mut ward = bare(WARD, 0);
    ward.duration_index = LASTS_LONG;
    effect(&mut ward, 0, aura(AURA_REFLECT_SPELLS, 0, 100, 1));
    let mut clumsy = bare(CLUMSY, 0);
    clumsy.duration_index = LASTS_LONG;
    effect(&mut clumsy, 0, aura(AURA_MOD_SPELL_HIT_CHANCE, 0, -100, 1));
    insert(d, [frost, plain, ward, clumsy]);
}

fn restat(m: &mut Map, g: Guid) {
    let data = m.realm.data.clone();
    m.players.get_mut(&g).unwrap().apply_stats(&data);
}

/// The next `SMSG_SPELLLOGMISS` of `spell`: its target and reason.
async fn missed(c: &mut crate::game::harness::Client, spell: u32) -> (Guid, u8) {
    let b = c
        .wait_frame(
            "SMSG_SPELLLOGMISS",
            crate::game::map::spells::outcome::SMSG_SPELLLOGMISS,
            |b| b.len() >= 26 && u32::from_le_bytes(b[0..4].try_into().unwrap()) == spell,
        )
        .await;
    (
        Guid::new(u64::from_le_bytes(b[17..25].try_into().unwrap())),
        b[25],
    )
}

/// Whether `SMSG_SPELL_GO` of `spell` came, listing `target`.
async fn went(c: &mut crate::game::harness::Client, spell: u32) -> Vec<Guid> {
    c.wait_for("SMSG_SPELL_GO", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == spell => Some(s.hits.clone()),
        _ => None,
    })
    .await
}

/// Thistle Tea's "queues a take-side outcome when a hostile magic spell is
/// fully resisted": a hostile magic spell its caster can't land is
/// resisted (`SPELL_MISS_RESIST`, reason 2 in `SMSG_SPELLLOGMISS`; wvs's
/// `SMSG_SPELL_GO` lists every target, misses apart, round 9's call) and
/// does nothing to its target; a ward that turns spells back does so
/// before the roll to hit (mangos-classic's `SpellHitResult`: the reflect
/// first, then `MagicSpellHitResult`), the spell turned on its caster; and
/// a spell of no damage class always lands, the penalty or not.
#[tokio::test]
async fn a_resisted_magic_spell_does_nothing_and_one_of_no_class_lands() {
    let realm = fast_realm(resists);
    let (mut c, a, map) = human(&realm, "P11S4", "Clumsy", Class::Mage).await;
    let kobold = on(&map, move |m| {
        kit::ready(m, a, 60, &[FROST, PLAIN]);
        kit::apply(m, a, a, CLUMSY);
        restat(m, a);
        kit::dummy(m, a, 60, 5.0)
    })
    .await;
    c.sees(kobold).await;
    let hp = on(&map, move |m| {
        cast_fresh(m, a, FROST, at(kobold));
        kit::health(m, kobold)
    })
    .await;
    assert_eq!(hp, 1_000_000, "resisted: nothing");
    went(&mut c, FROST).await;
    assert_eq!(missed(&mut c, FROST).await, (kobold, 2), "a resist");
    let hp = on(&map, move |m| {
        cast_fresh(m, a, PLAIN, at(kobold));
        kit::health(m, kobold)
    })
    .await;
    assert_eq!(hp, 1_000_000 - 50, "no damage class: it lands");
    assert_eq!(went(&mut c, PLAIN).await, vec![kobold]);
    // A ward on the kobold: the bolt is turned back before it is resisted.
    let (hp, mine) = on(&map, move |m| {
        kit::apply(m, kobold, kobold, WARD);
        kit::set_health(m, kobold, 1_000_000);
        kit::set_health(m, a, 1_000);
        cast_fresh(m, a, FROST, at(kobold));
        (kit::health(m, kobold), kit::health(m, a))
    })
    .await;
    assert_eq!(hp, 1_000_000, "the kobold untouched");
    assert_eq!(missed(&mut c, FROST).await, (kobold, 11), "reflected");
    assert!(mine <= 1_000, "turned on its caster: {mine}");
}

/// A two-second permanent enchantment that takes a reagent, and the
/// bracer it goes on; a two-second change of the item it is cast from into
/// another, also for a reagent.
const ENCHANT: u32 = 997_641;
const CHANGE: u32 = 997_642;
const ENCHANTMENT: u32 = 2_901;
const BRACER: u32 = 64_120;
const DUST: u32 = 64_121;
const KIT: u32 = 64_122;
const TOOL: u32 = 64_123;

fn costs(d: &mut GameData) {
    d.items.insert(
        BRACER,
        ItemTemplate {
            entry: BRACER,
            class: 4,
            subclass: 1,
            name: "Bracer".into(),
            inventory_type: 9,
            stackable: 1,
            ..Default::default()
        },
    );
    d.items.insert(
        DUST,
        ItemTemplate {
            entry: DUST,
            class: 7,
            name: "Dust".into(),
            stackable: 20,
            ..Default::default()
        },
    );
    let mut kit = ItemTemplate {
        entry: KIT,
        class: 15,
        name: "Kit".into(),
        stackable: 1,
        ..Default::default()
    };
    kit.spells[0] = ItemSpell {
        id: CHANGE,
        trigger: 0,
        charges: -1,
        ..Default::default()
    };
    d.items.insert(KIT, kit);
    d.items.insert(
        TOOL,
        ItemTemplate {
            entry: TOOL,
            class: 15,
            name: "Tool".into(),
            stackable: 1,
            ..Default::default()
        },
    );
    d.spells.enchantments.insert(
        ENCHANTMENT,
        Enchantment {
            id: ENCHANTMENT,
            effects: [(0, 0, 0, 0); 3],
            name: "Nothing".into(),
        },
    );
    let mut enchant = bare(ENCHANT, 0);
    enchant.dmg_class = 0;
    enchant.casting_time_index = CAST_2S;
    enchant.reagents[0] = (DUST, 1);
    effect(
        &mut enchant,
        0,
        SpellEffect {
            effect: 53,
            misc_value: ENCHANTMENT as i32,
            ..Default::default()
        },
    );
    let mut change = enchant.clone();
    change.id = CHANGE;
    change.effects[0] = SpellEffect {
        effect: crate::game::map::extra_effects::EFFECT_SUMMON_CHANGE_ITEM,
        misc_value: TOOL as i32,
        target_a: 1,
        bonus_coefficient: -1.0,
        ..Default::default()
    };
    insert(d, [enchant, change]);
}

fn count(m: &Map, g: Guid, entry: u32) -> u32 {
    m.players[&g].count_of(entry)
}

/// Thistle Tea's "permanent enchants defer material costs to completion
/// and cancel cleanly": an enchantment's reagent is there all its cast,
/// and taken at its end with the enchantment on (mangos's `Spell::cast`:
/// `TakeReagents` once it goes); cancelled, it takes nothing and enchants
/// nothing.
#[tokio::test]
async fn an_enchantment_takes_its_reagent_at_its_end_and_a_cancelled_one_nothing() {
    let realm = fast_realm(costs);
    let (_c, g, map) = human(&realm, "P11S5", "Enchanter", Class::Mage).await;
    let out = on(&map, move |m| {
        kit::ready(m, g, 60, &[ENCHANT]);
        // Not a GM, who pays no reagents.
        m.players.get_mut(&g).unwrap().gm = false;
        let bracer = crate::game::tests_loot::equip(m, g, BRACER, 8);
        crate::game::map::inventory::add_item(m, g, DUST, 2, false).unwrap();
        let enchanted = |m: &Map| {
            let i = m.players[&g]
                .items
                .iter()
                .find(|i| i.guid == bracer)
                .unwrap();
            crate::game::map::enchant::get(i, crate::game::map::enchant::PERM_SLOT).id
        };
        let item = Targets {
            item: Some(bracer),
            ..Default::default()
        };
        // Cancelled a second in.
        assert!(cast_fresh(m, g, ENCHANT, item));
        let casting_dust = count(m, g, DUST);
        pass(m, 1_000);
        spells::cancel(m, g);
        pass(m, 2_000);
        let cancelled = (casting_dust, count(m, g, DUST), enchanted(m));
        // Through to its end.
        assert!(cast_fresh(m, g, ENCHANT, item));
        let during = count(m, g, DUST);
        pass(m, 2_100);
        (cancelled, during, count(m, g, DUST), enchanted(m))
    })
    .await;
    assert_eq!(out.0, (2, 2, 0), "cancelled: nothing taken, nothing on");
    assert_eq!(out.1, 2, "nothing taken as it is cast");
    assert_eq!((out.2, out.3), (1, ENCHANTMENT), "at its end");
}

/// Thistle Tea's "item transformation replaces only its casting item and
/// defers all inventory costs": an item's change into another (`SPELL_
/// EFFECT_SUMMON_CHANGE_ITEM`) changes the item it is cast from, where it
/// lies, and not another of its kind the spell was aimed at; its reagent
/// and the item are taken only at its end, the item replaced rather than
/// used up; interrupted, nothing changes and nothing is taken.
#[tokio::test]
async fn an_items_change_is_of_its_casting_item_and_costs_only_at_its_end() {
    let realm = fast_realm(costs);
    let (_c, g, map) = human(&realm, "P11S6", "Tinker", Class::Mage).await;
    let out = on(&map, move |m| {
        kit::ready(m, g, 60, &[]);
        m.players.get_mut(&g).unwrap().gm = false;
        crate::game::map::inventory::add_item(m, g, KIT, 1, false).unwrap();
        crate::game::map::inventory::add_item(m, g, KIT, 1, false).unwrap();
        crate::game::map::inventory::add_item(m, g, DUST, 2, false).unwrap();
        let kits: Vec<(Guid, u8, u8)> = m.players[&g]
            .items
            .iter()
            .filter(|i| i.entry == KIT)
            .map(|i| (i.guid, i.bag, i.slot))
            .collect();
        let (cast_from, other) = (kits[0], kits[1]);
        let aimed = Targets {
            item: Some(other.0),
            ..Default::default()
        };
        fresh_cast(m, g, aimed, cast_from.0);
        pass(m, 500);
        spells::interrupt(m, g);
        pass(m, 2_000);
        let interrupted = (count(m, g, KIT), count(m, g, TOOL), count(m, g, DUST));
        fresh_cast(m, g, aimed, cast_from.0);
        let during = (count(m, g, KIT), count(m, g, DUST));
        pass(m, 2_100);
        let at = |guid: Guid| {
            m.players[&g]
                .items
                .iter()
                .find(|i| i.guid == guid)
                .map(|i| (i.entry, i.bag, i.slot))
        };
        (
            interrupted,
            during,
            at(cast_from.0),
            at(other.0),
            count(m, g, DUST),
            (cast_from, other),
        )
    })
    .await;
    let (interrupted, during, changed, kept, dust, (from, other)) = out;
    assert_eq!(interrupted, (2, 0, 2), "interrupted: as it was");
    assert_eq!(during, (2, 2), "nothing taken as it is cast");
    assert_eq!(
        changed,
        Some((TOOL, from.1, from.2)),
        "its casting item, where it lay"
    );
    assert_eq!(kept, Some((KIT, other.1, other.2)), "not the one aimed at");
    assert_eq!(dust, 1, "the reagent at its end");
}

fn fresh_cast(m: &mut Map, g: Guid, t: Targets, item: Guid) {
    kit::fresh(m, g);
    assert!(
        spells::cast(m, g, CHANGE, t, Some(item)),
        "the change begins"
    );
}

/// Three blows of 10, 20 and 30 in one spell.
const THREE: u32 = 997_651;

fn three(d: &mut GameData) {
    let mut s = bare(THREE, 4);
    s.attributes[3] |= ATTR_EX3_ALWAYS_HIT;
    for (i, n) in [10, 20, 30].into_iter().enumerate() {
        effect(&mut s, i, damage(n));
    }
    insert(d, [s]);
}

/// Thistle Tea's "custom points preserve unmodified effects and the cached
/// spell": a script's cast with its own points for some effects
/// (`cast_template_with_points`, mangos's `CastCustomSpell`) has those
/// effects at the points given, of no dice, the others as the template has
/// them, and the template itself unchanged.
#[tokio::test]
async fn a_scripts_points_leave_the_other_effects_and_the_template_as_they_were() {
    let realm = fast_realm(three);
    let (_c, g, map) = human(&realm, "P11S7", "Custom", Class::Mage).await;
    let out = on(&map, move |m| {
        kit::ready(m, g, 60, &[]);
        let k = kit::dummy(m, g, 60, 5.0);
        let template = m.realm.data.spells.get(THREE).cloned().unwrap();
        spells::scripts::cast_template_with_points(m, g, template, k, [Some(1), None, Some(5)]);
        let taken = 1_000_000 - kit::health(m, k);
        let after = m.realm.data.spells.get(THREE).cloned().unwrap();
        let effects: Vec<(i32, i32, i32)> = after
            .effects
            .iter()
            .map(|e| (e.base_points, e.base_dice, e.die_sides))
            .collect();
        kit::set_health(m, k, 1_000_000);
        spells::cast_triggered(m, g, THREE, Some(k));
        (taken, effects, 1_000_000 - kit::health(m, k))
    })
    .await;
    assert_eq!(out.0, 1 + 20 + 5, "the points given, the middle as it was");
    assert_eq!(
        out.1,
        vec![(9, 1, 0), (19, 1, 0), (29, 1, 0)],
        "the template"
    );
    assert_eq!(out.2, 60, "and cast as it is after");
}

/// A mage's child spell and a warlock's, of the family mask 1, a hundred
/// percent less to hit, and the mage's +100% to hit (`SPELLMOD_RESIST_
/// MISS_CHANCE`) for its family's mask 1.
const MAGES: u32 = 997_661;
const WARLOCKS: u32 = 997_662;
const FOCUS: u32 = 997_663;

fn families(d: &mut GameData) {
    resists(d);
    let mut mages = bare(MAGES, 4);
    mages.family_name = 3;
    mages.family_flags = 1;
    effect(&mut mages, 0, damage(50));
    let mut warlocks = mages.clone();
    warlocks.id = WARLOCKS;
    warlocks.family_name = 5;
    warlocks.school = 5;
    let mut focus = bare(FOCUS, 0);
    focus.family_name = 3;
    focus.duration_index = LASTS_LONG;
    let mut e = aura(AURA_ADD_FLAT_MODIFIER, 16, 100, 1);
    e.item_type = 1;
    effect(&mut focus, 0, e);
    insert(d, [mages, warlocks, focus]);
}

/// Thistle Tea's "foreign triggers match the child family mask using the
/// original hit snapshot": a triggered child spell takes its caster's
/// modifiers for its own family and mask at its cast (`mods::for_spell`),
/// so a hit modifier for the mage's mask lands the mage's child through a
/// hundred percent less to hit, and a warlock's spell of the same mask,
/// another family, is resisted.
#[tokio::test]
async fn a_triggered_childs_hit_takes_its_own_familys_modifier() {
    let realm = fast_realm(families);
    let (_c, g, map) = human(&realm, "P11S8", "Focused", Class::Mage).await;
    let out = on(&map, move |m| {
        kit::ready(m, g, 60, &[]);
        kit::apply(m, g, g, CLUMSY);
        kit::apply(m, g, g, FOCUS);
        restat(m, g);
        let k = kit::dummy(m, g, 60, 5.0);
        spells::cast_triggered(m, g, MAGES, Some(k));
        let mages = 1_000_000 - kit::health(m, k);
        kit::set_health(m, k, 1_000_000);
        spells::cast_triggered(m, g, WARLOCKS, Some(k));
        let warlocks = 1_000_000 - kit::health(m, k);
        (mages, warlocks)
    })
    .await;
    assert_eq!(
        out,
        (50, 0),
        "the mage's child lands, the warlock's is resisted"
    );
}
