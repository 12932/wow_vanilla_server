//! Phase 4's items in use, Thistle Tea's `ConsumableTest`, `DurabilityTest`,
//! `EnchantmentsTest`, `ItemSpellTest`, `HealingPowerDbcTest` and
//! `ReactiveArmorDbcTest` ported: a consumable's one outcome and what it
//! triggers in turn; Party Time!'s dances (none on the move, none once it
//! has gone); a blow's wear on one slot; enchanting (the item, its level,
//! the rod, the coatings) and what an enchantment binds; the Six Demon Bag,
//! the Bending Shinbone and the Fetid Skull; a resurrection's offer. The
//! healing sets and the reactive armors, on the real data, are in
//! `tests_p4_items_use2`.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{Enchantment, SpellDuration, SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::spells::{self, Targets, checks::fail};
use crate::game::map::{Map, auras, death, enchant, item_scripts};
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p4_items_gear::on_map;

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);
const DUMMY: (u32, u32) = (3300, 50);

/// The consumables and gadgets, and their outcomes.
const NOGGENFOGGER: u32 = 16589;
const NOGGENFOGGER_OUTCOMES: [u32; 3] = [16591, 16593, 16595];
/// What each of the Noggenfogger's outcomes triggers in turn, made up.
const NESTED: [u32; 3] = [9_700_001, 9_700_002, 9_700_003];
const PARTY_TIME: u32 = 8067;
const DANCE: u32 = 94;
const SIX_DEMON_BAG: u32 = 14537;
const BENDING_SHINBONE: u32 = 8856;
const FETID_SKULL: u32 = 17271;
const RESURRECTION: u32 = 2006;

/// Enchanting: a bracer's enchantment, one too high for a level 10
/// bracer, one wanting a rod, a poison for weapons, and a binding one.
const ENCHANT_BRACER: u32 = 9_700_101;
const ENCHANT_BRACER_HIGH: u32 = 9_700_102;
const ENCHANT_BRACER_ROD: u32 = 9_700_103;
const POISON: u32 = 9_700_104;
const ENCHANT_BINDING: u32 = 9_700_105;
const PLAIN: u32 = 9901;
const POISONED: u32 = 9902;
const BINDING: u32 = 9903;

/// Items.
const BRACER: u32 = 9801;
const BLADE: u32 = 9802;
const ROD: u32 = 6218;
const DURABLE: u32 = 9803;

/// Slots.
const MAIN: u8 = 15;
const PACK: u8 = 23;

fn effect(effect: u32, base_points: i32, target_a: u32) -> SpellEffect {
    SpellEffect {
        effect,
        base_points,
        base_dice: 1,
        die_sides: 1,
        target_a,
        ..Default::default()
    }
}

fn aura(kind: u32, target_a: u32) -> SpellEffect {
    SpellEffect {
        aura: kind,
        ..effect(6, 0, target_a)
    }
}

fn spell(id: u32, effects: [SpellEffect; 3]) -> SpellTemplate {
    SpellTemplate {
        id,
        name: format!("Spell {id}"),
        range_index: 1,
        equipped_item_class: -1,
        duration_index: 9,
        effects,
        ..Default::default()
    }
}

fn item(entry: u32, class: u32, subclass: u32, inventory_type: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        class,
        subclass,
        inventory_type,
        name: format!("Item {entry}"),
        display_id: 1000,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    }
}

fn data() -> GameData {
    let mut d = GameData::empty();
    d.player_level_stats.insert((1, 1, 1), [23, 20, 22, 20, 21]);
    d.player_class_stats.insert((1, 1), (60, 0));
    d.class_level_stats.insert(
        (1, 1),
        ClassLevelStats {
            health: 5000,
            melee_damage: 1.0,
            ..Default::default()
        },
    );
    d.creature_templates.insert(
        DUMMY.0,
        CreatureTemplate {
            entry: DUMMY.0,
            name: "Dummy".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 1,
            level_max: 1,
            faction: 8,
            unit_class: 1,
            health_multiplier: 1.0,
            damage_multiplier: 1.0,
            base_attack_time: 2000,
            ..Default::default()
        },
    );
    d.creature_spawns.insert(
        0,
        vec![CreatureSpawn {
            guid: DUMMY.1,
            entries: vec![DUMMY.0],
            map: 0,
            x: START.0 + 2.0,
            y: START.1,
            z: START.2,
            respawn_min: 1,
            respawn_max: 1,
            ..Default::default()
        }],
    );
    let s = &mut d.spells;
    s.ranges.insert(1, (0.0, 30.0));
    s.durations.insert(
        9,
        SpellDuration {
            base: 30_000,
            ..Default::default()
        },
    );
    let none = SpellEffect::default();
    let mut all = vec![
        spell(NOGGENFOGGER, [effect(3, 0, 1), none, none]),
        spell(FETID_SKULL, [effect(3, 0, 1), none, none]),
        spell(BENDING_SHINBONE, [effect(77, 0, 1), none, none]),
        spell(SIX_DEMON_BAG, [effect(3, 0, 6), none, none]),
        spell(PARTY_TIME, [aura(4, 1), none, none]),
    ];
    // Each of the Noggenfogger's outcomes is a mark, then a spell of its
    // own it triggers.
    for (o, n) in NOGGENFOGGER_OUTCOMES.iter().zip(NESTED) {
        let trigger = SpellEffect {
            trigger_spell: n,
            ..effect(64, 0, 1)
        };
        all.push(spell(*o, [aura(4, 1), trigger, none]));
        all.push(spell(n, [aura(4, 1), none, none]));
    }
    for id in [8854, 8855, 17269, 17270, 14642] {
        all.push(spell(id, [aura(4, 1), none, none]));
    }
    // The bag's outcomes on its target (Polymorph on its user too).
    for id in [11921, 13322, 21179, 13323, 25189] {
        all.push(spell(id, [aura(4, 6), none, none]));
    }
    let mut resurrection = spell(RESURRECTION, [none, none, none]);
    resurrection.effects[0] = SpellEffect {
        effect: 113,
        target_a: 21,
        base_points: 69,
        base_dice: 1,
        misc_value: 135,
        ..Default::default()
    };
    all.push(resurrection);
    // Enchanting.
    let enchanting = |id: u32, kind: u32, enchantment: u32| {
        let mut e = spell(id, [effect(kind, 0, 0), none, none]);
        e.effects[0].misc_value = enchantment as i32;
        e.duration_index = 0;
        e
    };
    let mut bracer = enchanting(ENCHANT_BRACER, 53, PLAIN);
    bracer.equipped_item_class = 4;
    bracer.equipped_item_inventory_type_mask = 1 << 9;
    bracer.base_level = 5;
    let high = SpellTemplate {
        id: ENCHANT_BRACER_HIGH,
        base_level: 50,
        ..bracer.clone()
    };
    let rod = SpellTemplate {
        id: ENCHANT_BRACER_ROD,
        totems: [ROD, 0],
        ..bracer.clone()
    };
    let binding = SpellTemplate {
        id: ENCHANT_BINDING,
        effects: enchanting(ENCHANT_BINDING, 53, BINDING).effects,
        ..bracer.clone()
    };
    // A coating for weapons, sixty seconds of it, past any item level.
    let mut poison = enchanting(POISON, 54, POISONED);
    poison.effects[0].base_points = 59;
    poison.equipped_item_class = 2;
    poison.base_level = 60;
    all.extend([bracer, high, rod, binding, poison]);
    for t in all {
        s.spells.insert(t.id, t);
    }
    for id in [PLAIN, POISONED, BINDING] {
        s.enchantments.insert(
            id,
            Enchantment {
                id,
                effects: [(0, 0, 0, 0); 3],
                name: format!("Enchant {id}"),
            },
        );
    }
    // A poison binds what it is on, as the binding one does.
    for id in [POISONED, BINDING] {
        d.crafts.enchant_flags.insert(id, 1);
    }
    let mut bracer = item(BRACER, 4, 1, 9);
    bracer.item_level = 10;
    let mut blade = item(BLADE, 2, 7, 13);
    blade.item_level = 20;
    let mut durable = item(DURABLE, 4, 1, 5);
    durable.max_durability = 50;
    for t in [bracer, blade, durable, item(ROD, 7, 0, 0)] {
        d.items.insert(t.entry, t);
    }
    d
}

async fn setup() -> (Arc<Realm>, Client, Guid) {
    let realm = harness::realm_with(data());
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Alice", Race::Human, Class::Warrior).await;
    a.sees(unit()).await;
    a.heartbeat(START.0, START.1, START.2).await;
    on_map(&realm, move |m| {
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.clear();
        p.refresh_inventory(&d);
    })
    .await;
    a.clear();
    (realm, a, g)
}

fn unit() -> Guid {
    guid::unit(DUMMY.0, DUMMY.1)
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

fn template(m: &Map, id: u32) -> SpellTemplate {
    m.realm.data.spells.get(id).unwrap().clone()
}

/// A new item of `entry` at the player's `slot`; its guid.
fn put(m: &mut Map, g: Guid, entry: u32, slot: u8) -> Guid {
    let d = m.realm.data.clone();
    let counter = m
        .realm
        .next_item
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let guid = guid::item(counter);
    let p = m.players.get_mut(&g).unwrap();
    assert!(p.item_at(BAG_PLAYER, slot).is_none(), "{slot} is free");
    p.items.push(crate::game::map::player::Item::new(
        &d, guid, g, entry, BAG_PLAYER, slot, 1,
    ));
    p.refresh_inventory(&d);
    guid
}

/// Why `g` can't cast `spell` at `item` now.
fn refused(m: &mut Map, g: Guid, spell: u32, item: Option<Guid>) -> Option<u8> {
    let s = template(m, spell);
    let t = Targets {
        item,
        ..Default::default()
    };
    let mods = spells::mods::Mods::default();
    spells::checks::check(m, g, &s, &t, None, false, &mods)
}

/// `spell` cast by `g` at `item`, known and free to cast.
fn cast_at(m: &mut Map, g: Guid, spell: u32, item: Guid) -> bool {
    if let Some(p) = m.players.get_mut(&g) {
        p.caster.gcd_until = None;
        if !p.spells.contains(&spell) {
            p.spells.push(spell);
        }
    }
    let t = Targets {
        item: Some(item),
        ..Default::default()
    };
    spells::cast(m, g, spell, t, None)
}

// ---------------------------------------------------------------------
// Consumables.

/// Thistle Tea's "one selected alternative fully resolves its nested
/// triggers and retains effect order": the Noggenfogger's one outcome
/// lands, and the spell it triggers in turn lands after it; neither of the
/// other outcomes' does, and each goes off once.
#[tokio::test]
async fn a_consumables_one_outcome_lands_and_then_what_it_triggers() {
    let (realm, mut a, g) = setup().await;
    for _ in 0..12 {
        a.clear();
        let chosen = on_map(&realm, move |m| {
            spells::cast_triggered(m, g, NOGGENFOGGER, Some(g));
            let got: Vec<usize> = (0..3)
                .filter(|i| has(m, g, NOGGENFOGGER_OUTCOMES[*i]))
                .collect();
            let nested: Vec<usize> = (0..3).filter(|i| has(m, g, NESTED[*i])).collect();
            for id in NOGGENFOGGER_OUTCOMES.iter().chain(&NESTED) {
                auras::remove_spell(m, g, *id, None);
            }
            (got, nested)
        })
        .await;
        assert_eq!(chosen.0.len(), 1, "one outcome: {chosen:?}");
        assert_eq!(chosen.0, chosen.1, "and only its own trigger");
        let i = chosen.0[0];
        let gone_off = |id: u32| {
            move |m: &ServerOpcodeMessage| match m {
                ServerOpcodeMessage::SMSG_SPELL_GO(s)
                    if NOGGENFOGGER_OUTCOMES.contains(&s.spell) || NESTED.contains(&s.spell) =>
                {
                    Some(s.spell == id)
                }
                _ => None,
            }
        };
        // The first of them to go off is the outcome, then its trigger.
        assert!(
            a.wait_for("the outcome", gone_off(NOGGENFOGGER_OUTCOMES[i]))
                .await
        );
        assert!(a.wait_for("its trigger", gone_off(NESTED[i])).await);
        assert!(
            a.gets(Duration::from_millis(20), gone_off(0))
                .await
                .is_none(),
            "each once"
        );
    }
}

/// `n` of Party Time!'s dances, each made due and run.
fn dance(m: &mut Map, g: Guid, n: usize) {
    for _ in 0..n {
        let now = m.now;
        m.players.get_mut(&g).unwrap().gadgets.party_at = Some(now);
        item_scripts::update(m);
    }
}

/// The emotes of Party Time! `a` is sent, `n` of them.
async fn emotes(a: &mut Client, g: Guid, n: usize) -> Vec<u32> {
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(
            a.wait_for("an emote", |m| match m {
                ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == g => Some(e.emote.as_int()),
                _ => None,
            })
            .await,
        );
    }
    out
}

/// Thistle Tea's "movement excludes dancing without mutating movement or a
/// persistent pose": on the move (walking, strafing, pitching, jumping,
/// falling, or on a spline's elevation, mangos's `MOVEFLAG_MASK_MOVING`),
/// Party Time! cheers, applauds, laughs and roars, but doesn't dance; its
/// dancer's movement and emote state are as they were. Standing still, it
/// dances too.
#[tokio::test]
async fn party_time_doesnt_dance_on_the_move() {
    use wow_world_messages::vanilla::{
        MovementInfo_MovementFlags as Flags, MovementInfo_MovementFlags_Jumping as Jump,
        MovementInfo_MovementFlags_SplineElevation as Elevation,
    };
    let (realm, mut a, g) = setup().await;
    on_map(&realm, move |m| {
        spells::cast_triggered(m, g, PARTY_TIME, Some(g))
    })
    .await;
    let moving = [
        Flags::empty().set_forward(),
        Flags::empty().set_backward(),
        Flags::empty().set_strafe_left(),
        Flags::empty().set_strafe_right(),
        Flags::empty().set_pitch_up(),
        Flags::empty().set_pitch_down(),
        Flags::empty().set_jumping(Jump::default()),
        Flags::empty().set_fallingfar(),
        Flags::empty().set_spline_elevation(Elevation::default()),
    ];
    for flags in moving {
        a.clear();
        let f = flags;
        let kept = on_map(&realm, move |m| {
            m.players.get_mut(&g).unwrap().movement.flags = f;
            dance(m, g, 30);
            let p = &m.players[&g];
            (p.movement.flags == f, p.fields.get_u32(UNIT_NPC_EMOTESTATE))
        })
        .await;
        assert_eq!(kept, (true, 0), "{flags:?}");
        let seen = emotes(&mut a, g, 30).await;
        assert!(
            seen.iter().all(|e| [21, 4, 19, 11].contains(e)),
            "{flags:?}: {seen:?}"
        );
    }
    a.clear();
    on_map(&realm, move |m| {
        m.players.get_mut(&g).unwrap().movement.flags = Flags::empty();
        dance(m, g, 60);
    })
    .await;
    let seen = emotes(&mut a, g, 60).await;
    assert!(seen.contains(&DANCE), "standing, a dance: {seen:?}");
}

/// Thistle Tea's "cancel, expiry and death stop periodic animations":
/// Party Time! cancelled, run out, or its dancer dead, it is gone, and
/// there is no dance more.
#[tokio::test]
async fn party_times_dances_end_with_it() {
    let (realm, mut a, g) = setup().await;
    for how in ["cancelled", "expired", "dead"] {
        let gone = on_map(&realm, move |m| {
            spells::cast_triggered(m, g, PARTY_TIME, Some(g));
            assert!(has(m, g, PARTY_TIME), "{how}: on");
            match how {
                "cancelled" => auras::cancel(m, g, PARTY_TIME),
                "expired" => {
                    let now = m.now;
                    for h in auras::auras_mut(m, g).unwrap().holders.iter_mut() {
                        if h.spell == PARTY_TIME {
                            h.expires = Some(now);
                        }
                    }
                    auras::update(m);
                }
                _ => death::kill(m, g),
            }
            let gone = !has(m, g, PARTY_TIME);
            dance(m, g, 5);
            gone
        })
        .await;
        assert!(gone, "{how}: gone");
        let danced = a
            .gets(Duration::from_millis(100), |m| match m {
                ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == g => Some(()),
                _ => None,
            })
            .await;
        assert!(danced.is_none(), "{how}: no dance");
    }
}

// ---------------------------------------------------------------------
// Durability.

/// Every worn item's durability, by slot.
fn worn_durability(m: &Map, g: Guid) -> Vec<(u8, u32)> {
    let mut v: Vec<(u8, u32)> = m.players[&g]
        .items
        .iter()
        .filter(|i| i.bag == BAG_PLAYER && i.slot < 19)
        .map(|i| (i.slot, i.fields.get_u32(ITEM_FIELD_DURABILITY)))
        .collect();
    v.sort();
    v
}

/// Thistle Tea's "one hit wears only its selected slot": a blow that wears
/// takes a point off the one slot it picks, and none off the others; one
/// that picks an empty slot wears nothing.
#[tokio::test]
async fn a_blow_wears_the_one_slot_it_picks() {
    let (realm, _a, g) = setup().await;
    let (wears, only_main) = on_map(&realm, move |m| {
        for slot in 0..19 {
            put(m, g, DURABLE, slot);
        }
        let creature = unit();
        let mut wears = 0;
        for _ in 0..20_000 {
            let before = worn_durability(m, g);
            crate::game::map::vendor::wear_from_blow(m, g, creature);
            let after = worn_durability(m, g);
            let worn: Vec<(u8, u32)> = before
                .iter()
                .zip(&after)
                .filter(|(b, a)| b != a)
                .map(|(b, a)| (b.0, b.1 - a.1))
                .collect();
            assert!(worn.len() <= 1, "one slot: {worn:?}");
            if let Some((_, loss)) = worn.first() {
                assert_eq!(*loss, 1, "a point");
                wears += 1;
            }
        }
        // Only the main hand worn: only it wears.
        let d = m.realm.data.clone();
        let p = m.players.get_mut(&g).unwrap();
        p.items.retain(|i| i.slot == MAIN);
        p.refresh_inventory(&d);
        let mut only_main = 0;
        for _ in 0..60_000 {
            let before = worn_durability(m, g);
            crate::game::map::vendor::wear_from_blow(m, g, creature);
            let after = worn_durability(m, g);
            assert_eq!(after.len(), 1);
            if after != before {
                assert_eq!(after[0].1 + 1, before[0].1);
                only_main += 1;
            }
        }
        (wears, only_main)
    })
    .await;
    // One blow in two hundred wears.
    assert!((40..=200).contains(&wears), "{wears}");
    assert!(only_main > 0, "the main hand wore");
}

// ---------------------------------------------------------------------
// Enchanting.

/// Thistle Tea's "requires an owned compatible item of sufficient level",
/// "validates target equipment rather than the caster weapon and requires
/// a rod" and "temporary coatings validate their target without permanent
/// enchant level restrictions": an enchantment is put on a carried item
/// that fits it (a naked enchanter's bracer, not the gear they wear) of its
/// level at least, a rod at hand when it wants one, the enchanter alive; a
/// coating, on a weapon of any level.
#[tokio::test]
async fn an_enchantment_wants_a_fitting_item_of_its_level_and_its_rod() {
    let (realm, _a, g) = setup().await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let out = on_map(&realm, move |m| {
        let bracer = put(m, g, BRACER, PACK);
        let blade = put(m, g, BLADE, PACK + 1);
        m.players.get_mut(&gb).unwrap().items.clear();
        let theirs = put(m, gb, BRACER, PACK);
        let mut out = vec![
            ("the bracer", refused(m, g, ENCHANT_BRACER, Some(bracer))),
            ("no item", refused(m, g, ENCHANT_BRACER, None)),
            ("a blade", refused(m, g, ENCHANT_BRACER, Some(blade))),
            ("Bob's", refused(m, g, ENCHANT_BRACER, Some(theirs))),
            ("too low", refused(m, g, ENCHANT_BRACER_HIGH, Some(bracer))),
            ("no rod", refused(m, g, ENCHANT_BRACER_ROD, Some(bracer))),
            ("the coating", refused(m, g, POISON, Some(blade))),
            ("a coated bracer", refused(m, g, POISON, Some(bracer))),
            ("nothing coated", refused(m, g, POISON, None)),
        ];
        put(m, g, ROD, PACK + 2);
        out.push(("the rod", refused(m, g, ENCHANT_BRACER_ROD, Some(bracer))));
        let enchanted = cast_at(m, g, ENCHANT_BRACER, bracer);
        let i = m.players[&g].item_by_guid(bracer).unwrap();
        let on = enchant::get(&m.players[&g].items[i], enchant::PERM_SLOT).id;
        death::kill(m, g);
        out.push(("dead", refused(m, g, ENCHANT_BRACER, Some(bracer))));
        (out, enchanted, on)
    })
    .await;
    let (out, enchanted, on) = out;
    assert_eq!(
        out,
        vec![
            ("the bracer", None),
            ("no item", Some(fail::ITEM_GONE)),
            ("a blade", Some(fail::BAD_TARGETS)),
            ("Bob's", Some(fail::ITEM_GONE)),
            ("too low", Some(fail::LOWLEVEL)),
            ("no rod", Some(fail::TOTEMS)),
            ("the coating", None),
            ("a coated bracer", Some(fail::BAD_TARGETS)),
            ("nothing coated", Some(fail::ITEM_GONE)),
            ("the rod", None),
            ("dead", Some(fail::CASTER_DEAD)),
        ]
    );
    assert!(enchanted);
    assert_eq!(on, PLAIN);
}

/// Which of `items` Alice may offer Bob in a trade, slot by slot.
async fn tradeable(realm: &Arc<Realm>, a: &mut Client, items: &[u8]) -> Vec<bool> {
    let g = a.guid;
    on_map(realm, move |m| {
        if let Some(t) = m.players.get_mut(&g).unwrap().trade.as_mut() {
            t.items = Default::default();
        }
    })
    .await;
    for (trade_slot, slot) in items.iter().enumerate() {
        a.send(CMSG_SET_TRADE_ITEM {
            trade_slot: trade_slot as u8,
            bag: BAG_PLAYER,
            slot: *slot,
        })
        .await;
    }
    a.send(CMSG_NAME_QUERY { guid: g }).await;
    a.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
    let n = items.len();
    on_map(realm, move |m| {
        let t = m.players[&g].trade.as_ref().expect("trading");
        (0..n).map(|i| t.items[i].is_some()).collect()
    })
    .await
}

/// Thistle Tea's "temporary binding lasts only as long as its enchant", as
/// mangos's `IsBoundByEnchant` has it: an ordinary enchantment doesn't
/// bind its item, so it may be traded; a poison (an enchantment flagged
/// soulbound) binds its weapon for as long as it lasts, and one that binds
/// on a bracer for good; a bound item is bound.
#[tokio::test]
async fn a_poison_binds_its_weapon_for_as_long_as_it_lasts() {
    let (realm, mut a, g) = setup().await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let blade = on_map(&realm, move |m| {
        let plain = put(m, g, BRACER, PACK);
        let blade = put(m, g, BLADE, PACK + 1);
        let binding = put(m, g, BRACER, PACK + 2);
        let flagged = put(m, g, BLADE, PACK + 3);
        assert!(cast_at(m, g, ENCHANT_BRACER, plain));
        assert!(cast_at(m, g, POISON, blade));
        assert!(cast_at(m, g, ENCHANT_BINDING, binding));
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(flagged).unwrap();
        p.items[i].fields.set_u32(
            ITEM_FIELD_FLAGS,
            crate::game::map::items::ITEM_FLAG_SOULBOUND,
        );
        blade
    })
    .await;
    a.send(CMSG_INITIATE_TRADE { guid: gb }).await;
    b.wait_for("the trade asked", |m| match m {
        ServerOpcodeMessage::SMSG_TRADE_STATUS(s) => {
            matches!(s.status, SMSG_TRADE_STATUS_TradeStatus::BeginTrade { .. }).then_some(())
        }
        _ => None,
    })
    .await;
    b.send(CMSG_BEGIN_TRADE {}).await;
    b.send(CMSG_NAME_QUERY { guid: gb }).await;
    b.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
    let offered = tradeable(&realm, &mut a, &[PACK, PACK + 1, PACK + 2, PACK + 3]).await;
    assert_eq!(offered, vec![true, false, false, false]);
    // The poison runs out: the blade is free again.
    let left = on_map(&realm, move |m| {
        let p = m.players.get_mut(&g).unwrap();
        let i = p.item_by_guid(blade).unwrap();
        let mut e = enchant::get(&p.items[i], enchant::TEMP_SLOT);
        assert_eq!((e.id, e.duration_ms), (POISONED, 60_000));
        e.duration_ms = 500;
        enchant::set(&mut p.items[i], enchant::TEMP_SLOT, e);
        let now = m.now;
        m.loot_state.enchant_tick_at = now.checked_sub(Duration::from_secs(2));
        enchant::update(m);
        let p = &m.players[&g];
        enchant::get(&p.items[p.item_by_guid(blade).unwrap()], enchant::TEMP_SLOT).id
    })
    .await;
    assert_eq!(left, 0, "run out");
    let offered = tradeable(&realm, &mut a, &[PACK + 1]).await;
    assert_eq!(offered, vec![true]);
}

// ---------------------------------------------------------------------
// Item spells.

/// How often each of `n` uses of `spell` by `g` at `target` leaves each of
/// `outcomes` on the target and on `g`.
fn outcomes(
    m: &mut Map,
    g: Guid,
    spell: u32,
    target: Guid,
    outcomes: &[u32],
    n: usize,
) -> Vec<(u32, usize, usize)> {
    let mut counts: Vec<(u32, usize, usize)> = outcomes.iter().map(|o| (*o, 0, 0)).collect();
    for _ in 0..n {
        spells::cast_triggered(m, g, spell, Some(target));
        let mut landed = 0;
        for c in counts.iter_mut() {
            if target != g && has(m, target, c.0) {
                c.1 += 1;
                landed += 1;
            }
            if has(m, g, c.0) {
                c.2 += 1;
                landed += 1;
            }
            auras::remove_spell(m, target, c.0, None);
            auras::remove_spell(m, g, c.0, None);
        }
        assert_eq!(landed, 1, "one outcome a use");
    }
    counts
}

/// Whether `count` of `n` is `percent` give or take `by` points.
fn near(count: usize, n: usize, percent: f64, by: f64) -> bool {
    let got = count as f64 * 100.0 / n as f64;
    (got - percent).abs() <= by
}

/// Thistle Tea's "Six Demon Bag preserves every outcome's chance, caster,
/// recipient": a quarter Fireball, a quarter Frostbolt, a fifth Chain
/// Lightning, 15% Enveloping Winds, 7% Polymorph on the target and 3% on
/// its user, 5% a felhound for its user. Its second effect does nothing,
/// and it isn't used on the dead.
#[tokio::test]
async fn the_six_demon_bag_looses_one_of_seven_things() {
    let (realm, _a, g) = setup().await;
    let n = 3000;
    let (counts, second, dead) = on_map(&realm, move |m| {
        let target = unit();
        let all = [11921, 13322, 21179, 13323, 25189, 14642];
        let counts = outcomes(m, g, SIX_DEMON_BAG, target, &all, n);
        let bag = template(m, SIX_DEMON_BAG);
        let second = item_scripts::effect(m, g, target, &bag, 1, None)
            || all.iter().any(|o| has(m, target, *o) || has(m, g, *o));
        crate::game::map::combat::kill(m, g, target);
        let mods = spells::mods::Mods::default();
        let at = Targets {
            unit: Some(target),
            ..Default::default()
        };
        let dead = spells::checks::check(m, g, &bag, &at, None, false, &mods);
        (counts, second, dead)
    })
    .await;
    let expected = [
        (11921, 25.0, 0.0),
        (13322, 25.0, 0.0),
        (21179, 20.0, 0.0),
        (13323, 7.0, 3.0),
        (25189, 15.0, 0.0),
        (14642, 0.0, 5.0),
    ];
    for ((id, on_target, on_self), (eid, target_pct, self_pct)) in counts.iter().zip(expected) {
        assert_eq!(*id, eid);
        assert!(near(*on_target, n, target_pct, 4.0), "{counts:?}");
        assert!(near(*on_self, n, self_pct, 1.6), "{counts:?}");
    }
    assert!(!second, "its second effect does nothing");
    assert_eq!(dead, Some(fail::TARGETS_DEAD));
}

/// Thistle Tea's "Dragonmaw Shinbones usually break and Mystic Crystals
/// split skulls from bone dust": the Bending Shinbone breaks (8855) four
/// times in five, else is whole (8854); the Fetid Skull resonates or turns
/// to dust, one in two; each on its user, and used on another nothing.
#[tokio::test]
async fn the_shinbone_mostly_breaks_and_the_skull_splits_evenly() {
    let (realm, _a, g) = setup().await;
    let n = 1500;
    let (bone, skull, other) = on_map(&realm, move |m| {
        let bone = outcomes(m, g, BENDING_SHINBONE, g, &[8854, 8855], n);
        let skull = outcomes(m, g, FETID_SKULL, g, &[17269, 17270], n);
        let target = unit();
        let all = [8854, 8855, 17269, 17270];
        for id in [BENDING_SHINBONE, FETID_SKULL] {
            let s = template(m, id);
            item_scripts::effect(m, g, target, &s, 0, None);
        }
        let other = all.iter().any(|o| has(m, target, *o) || has(m, g, *o));
        (bone, skull, other)
    })
    .await;
    assert!(near(bone[0].2, n, 20.0, 4.0), "{bone:?}");
    assert!(near(bone[1].2, n, 80.0, 4.0), "{bone:?}");
    assert!(near(skull[0].2, n, 50.0, 5.0), "{skull:?}");
    assert!(near(skull[1].2, n, 50.0, 5.0), "{skull:?}");
    assert!(!other, "nothing for another");
}

/// Thistle Tea's "ordinary resurrection still creates its offer
/// immediately": Resurrection on the dead offers its health and mana at
/// once, and the dead are asked.
#[tokio::test]
async fn a_resurrection_offers_its_health_and_mana_at_once() {
    let (realm, mut a, g) = setup().await;
    let mut b = Client::connect(&realm, "B");
    let priest = b.enter("Bob", Race::Human, Class::Priest).await;
    a.clear();
    let offer = on_map(&realm, move |m| {
        death::kill(m, g);
        spells::cast_triggered(m, priest, RESURRECTION, Some(g));
        m.players[&g].resurrect_offer
    })
    .await;
    assert_eq!(offer, Some((priest, 70, 135)));
    a.wait_frame(
        "SMSG_RESURRECT_REQUEST",
        spells::SMSG_RESURRECT_REQUEST_OPCODE,
        |b| b[..8] == priest.guid().to_le_bytes(),
    )
    .await;
}
