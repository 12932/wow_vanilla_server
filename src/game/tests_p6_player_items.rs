//! Round 6's player tests of items, ported from Thistle Tea's
//! `test/game/world/entity/player/item_durations_test.exs` and others
//! (docs/parity/matrix.csv): timed items running out, online and away.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::{GameData, ItemTemplate};
use crate::game::fields::ITEM_FIELD_DURATION;
use crate::game::harness::{self, Client};
use crate::game::map::items::{TEMPLATE_FLAG_CONJURED, TEMPLATE_FLAG_REAL_DURATION};
use crate::game::map::{Map, MapKey};
use crate::game::tests_p3_entity::on;

/// Five minutes' time, its clock stopped while its owner is away; the same
/// by the clock; a conjured loaf; and a timed item with a three-second
/// spell to use.
const HOURGLASS: u32 = 63_200;
const SUNDIAL: u32 = 63_201;
const LOAF: u32 = 63_202;
const TIMED_WAND: u32 = 63_203;
const WAND_SPELL: u32 = 96_400;

fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let item = |entry, flags, duration| ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        flags,
        duration,
        ..Default::default()
    };
    d.items.insert(HOURGLASS, item(HOURGLASS, 0, 300));
    d.items
        .insert(SUNDIAL, item(SUNDIAL, TEMPLATE_FLAG_REAL_DURATION, 300));
    d.items.insert(LOAF, item(LOAF, TEMPLATE_FLAG_CONJURED, 0));
    let mut wand = item(TIMED_WAND, 0, 300);
    wand.spells[0].id = WAND_SPELL;
    d.items.insert(TIMED_WAND, wand);
    let mut s = crate::game::tests_p3_entity::spell(
        WAND_SPELL,
        &[crate::game::data::spells::SpellEffect {
            effect: 3,
            target_a: 1,
            ..Default::default()
        }],
    );
    s.casting_time_index = 14;
    d.spells.cast_times.insert(14, 3_000);
    d.spells.spells.insert(WAND_SPELL, s);
    d
}

/// `g`'s item of `entry` and its time left.
fn timed(m: &Map, g: Guid, entry: u32) -> Option<(Guid, u32)> {
    m.players[&g]
        .items
        .iter()
        .find(|i| i.entry == entry)
        .map(|i| (i.guid, i.fields.get_u32(ITEM_FIELD_DURATION)))
}

/// Thistle Tea's `ItemDurations.sync` and `tick` "publishes remaining
/// seconds" and the expiry it then runs, and "expiration cancels a cast
/// using the removed item": a timed item is told its time, runs down in
/// the world, and at its end is gone (`SMSG_DESTROY_OBJECT`); one whose
/// spell is being cast as it ends stops the cast.
#[tokio::test]
async fn a_timed_item_runs_down_goes_at_its_end_and_stops_its_cast() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (hourglass, left) = on(&map, move |m| {
        crate::game::map::inventory::add_item(m, ga, HOURGLASS, 1, false).unwrap();
        timed(m, ga, HOURGLASS).unwrap()
    })
    .await;
    assert_eq!(left, 300, "its five minutes");
    let told = a
        .wait_for("SMSG_ITEM_TIME_UPDATE", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_TIME_UPDATE(t) if t.guid == hourglass => {
                Some(t.duration)
            }
            _ => None,
        })
        .await;
    assert!((298..=300).contains(&told), "{told}");
    // A second or so in the world takes a second off.
    tokio::time::sleep(Duration::from_millis(2_200)).await;
    let left = on(&map, move |m| timed(m, ga, HOURGLASS).map(|t| t.1)).await;
    assert!(left.is_some_and(|l| (296..300).contains(&l)), "{left:?}");
    on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.item_by_guid(hourglass).unwrap();
        p.items[i].fields.set_u32(ITEM_FIELD_DURATION, 1);
    })
    .await;
    a.wait_for("SMSG_DESTROY_OBJECT", |m| match m {
        ServerOpcodeMessage::SMSG_DESTROY_OBJECT(d) => (d.guid == hourglass).then_some(()),
        _ => None,
    })
    .await;
    assert_eq!(on(&map, move |m| timed(m, ga, HOURGLASS)).await, None);
    // The wand's spell under way as it runs out: the cast stops.
    let casting = on(&map, move |m| {
        crate::game::map::inventory::add_item(m, ga, TIMED_WAND, 1, false).unwrap();
        let (wand, _) = timed(m, ga, TIMED_WAND).unwrap();
        crate::game::map::spells::learn(m, ga, WAND_SPELL);
        crate::game::map::spells::cast(
            m,
            ga,
            WAND_SPELL,
            crate::game::map::spells::Targets::default(),
            Some(wand),
        );
        let on = m
            .caster(ga)
            .and_then(|c| c.casting.as_ref())
            .is_some_and(|c| c.item == Some(wand));
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.item_by_guid(wand).unwrap();
        p.items[i].fields.set_u32(ITEM_FIELD_DURATION, 1);
        (wand, on)
    })
    .await;
    assert!(casting.1, "casting from it");
    a.wait_for("the wand destroyed", |m| match m {
        ServerOpcodeMessage::SMSG_DESTROY_OBJECT(d) => (d.guid == casting.0).then_some(()),
        _ => None,
    })
    .await;
    let still = on(&map, move |m| {
        m.caster(ga).and_then(|c| c.casting.as_ref()).is_some()
    })
    .await;
    assert!(!still, "the cast stopped");
}

/// Thistle Tea's `ItemDurations.restore` "ordinary items resume their
/// exact saved budget" and "real-time expiry and conjured bank cleanup
/// occur before login publication": coming back after a while away, an
/// ordinary timed item has the time it had, one by the clock has lost the
/// time away (mangos's `ITEM_FLAG_REAL_DURATION`), or is gone if that was
/// all of it; a conjured item is gone after fifteen minutes away, not
/// before.
#[tokio::test]
async fn coming_back_an_ordinary_timed_item_waited_a_real_time_one_ran_on_and_conjured_ones_go() {
    let realm = harness::realm_with(world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    // Back after `away` seconds: what's left of each.
    let back = |away: i64| {
        let map = map.clone();
        async move {
            on(&map, move |m| {
                let p = m.players.get_mut(&ga).unwrap();
                p.items
                    .retain(|i| ![HOURGLASS, SUNDIAL, LOAF].contains(&i.entry));
                for e in [HOURGLASS, SUNDIAL, LOAF] {
                    crate::game::map::inventory::add_item(m, ga, e, 1, false).unwrap();
                }
                let data = m.realm.data.clone();
                let p = m.players.get_mut(&ga).unwrap();
                p.row.logout_time = crate::game::social::mail::now() - away;
                crate::game::map::items::on_login(p, &data);
                [HOURGLASS, SUNDIAL, LOAF].map(|e| timed(m, ga, e).map(|t| t.1))
            })
            .await
        }
    };
    assert_eq!(
        back(100).await,
        [Some(300), Some(200), Some(0)],
        "the hourglass waited, the sundial ran on"
    );
    assert_eq!(
        back(300).await,
        [Some(300), None, Some(0)],
        "the sundial's time up"
    );
    assert_eq!(
        back(14 * 60).await,
        [Some(300), None, Some(0)],
        "the loaf, fourteen minutes on"
    );
    assert_eq!(
        back(15 * 60).await,
        [Some(300), None, None],
        "fifteen: gone"
    );
    let _ = a.guid;
}

/// A helm of ten points of durability, worn.
const HELM: u32 = 63_210;
/// A spell that kills with no wear (`SPELL_ATTR_EX3_NO_DURABILITY_LOSS`).
const NO_WEAR: u32 = 96_410;

fn wear_world() -> GameData {
    let mut d = world();
    d.items.insert(
        HELM,
        ItemTemplate {
            entry: HELM,
            name: "Helm".into(),
            class: 4,
            subclass: 1,
            inventory_type: 1,
            stackable: 1,
            max_durability: 10,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    let mut s = crate::game::tests_p3_entity::spell(NO_WEAR, &[]);
    s.attributes[3] = crate::game::data::spells::ATTR_EX3_NO_DURABILITY_LOSS;
    d.spells.spells.insert(NO_WEAR, s);
    d
}

/// The helm's durability left.
fn helm_left(m: &Map, g: Guid) -> u32 {
    let p = &m.players[&g];
    p.items.iter().find(|i| i.entry == HELM).map_or(0, |i| {
        i.fields.get_u32(crate::game::fields::ITEM_FIELD_DURABILITY)
    })
}

/// Thistle Tea's `Durability` "one lethal transition wears equipment once
/// and resurrection permits the next penalty" and "PvP, controlled pets,
/// battlegrounds, and exempt spells avoid death penalties": a death to a
/// creature wears the gear a tenth (a point of ten), once however often
/// the dead are struck; up again, the next death wears it again; a death
/// to a player's pet, or by a spell that wears nothing, none (mangos's
/// `Unit::Kill`: no `player_tap`).
#[tokio::test]
async fn a_death_wears_gear_once_and_not_to_a_players_pet_or_by_an_exempt_spell() {
    use crate::game::map::combat;
    use crate::game::tests_p3_combat::{BRUTE, spawn};
    let realm = harness::realm_with(wear_world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Bob", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let worn = on(&map, move |m| {
        crate::game::tests_p3_combat::equip(m, ga, HELM, 0);
        let brute = spawn(m, ga, BRUTE, 20, 3.0);
        combat::kill(m, brute, ga);
        let first = helm_left(m, ga);
        combat::kill(m, brute, ga);
        let again = helm_left(m, ga);
        crate::game::map::death::resurrect(m, ga, 1.0);
        combat::kill(m, brute, ga);
        let next = helm_left(m, ga);
        crate::game::map::death::resurrect(m, ga, 1.0);
        // Bob's pet.
        let pet = spawn(m, ga, BRUTE, 20, 3.0);
        m.creature_mut(pet).unwrap().owner = Some(gb);
        combat::kill(m, pet, ga);
        let to_pet = helm_left(m, ga);
        crate::game::map::death::resurrect(m, ga, 1.0);
        combat::kill_by(m, brute, ga, Some(NO_WEAR));
        let exempt = helm_left(m, ga);
        (first, again, next, to_pet, exempt)
    })
    .await;
    assert_eq!(
        worn,
        (9, 9, 8, 8, 8),
        "once a death; none to a pet or the exempt spell"
    );
    let _ = (a.guid, b.guid);
}

/// Bracers to enchant with five health for a dust, and with a temporary
/// enchant of the same bonus.
const BRACER: u32 = 63_220;
const DUST: u32 = 63_221;
const ENCHANT: u32 = 96_420;
const COAT: u32 = 96_421;
const HEALTH_5: u32 = 41;

fn enchant_world() -> GameData {
    use crate::game::data::spells::{Enchantment, SpellEffect};
    let mut d = world();
    d.items.insert(
        BRACER,
        ItemTemplate {
            entry: BRACER,
            name: "Bracers".into(),
            class: 4,
            subclass: 1,
            inventory_type: 9,
            display_id: 1_000,
            stackable: 1,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    d.items.insert(
        DUST,
        ItemTemplate {
            entry: DUST,
            name: "Dust".into(),
            stackable: 20,
            allowable_class: -1,
            allowable_race: -1,
            ..Default::default()
        },
    );
    // The coat's points are its seconds: ten.
    for (id, effect, points) in [(ENCHANT, 53, 0), (COAT, 54, 9)] {
        let mut s = crate::game::tests_p3_entity::spell(
            id,
            &[SpellEffect {
                effect,
                misc_value: HEALTH_5 as i32,
                base_points: points,
                base_dice: 1,
                target_a: 1,
                ..Default::default()
            }],
        );
        s.equipped_item_class = 4;
        s.equipped_item_inventory_type_mask = 1 << 9;
        if id == ENCHANT {
            s.reagents[0] = (DUST, 1);
        }
        d.spells.spells.insert(id, s);
    }
    d.spells.enchantments.insert(
        HEALTH_5,
        Enchantment {
            id: HEALTH_5,
            effects: [(5, 5, 5, 1), (0, 0, 0, 0), (0, 0, 0, 0)],
            name: "Health +5".into(),
        },
    );
    d
}

/// `g`'s maximum health, the bracers' permanent enchantment, and the dust
/// left.
fn enchanted(m: &Map, g: Guid) -> (u32, u32, u32) {
    let p = &m.players[&g];
    let bracer = p.items.iter().find(|i| i.entry == BRACER).map_or(0, |i| {
        i.fields
            .get_u32(crate::game::fields::ITEM_FIELD_ENCHANTMENT)
    });
    (
        p.fields.get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH),
        bracer,
        p.count_of(DUST),
    )
}

/// Cast `spell` at `g`'s bracers.
fn enchant(m: &mut Map, g: Guid, spell: u32) {
    let bracer = m.players[&g]
        .items
        .iter()
        .find(|i| i.entry == BRACER)
        .map(|i| i.guid);
    if let Some(c) = m.caster_mut(g) {
        c.cooldowns.clear();
        c.gcd_until = None;
    }
    crate::game::map::spells::cast(
        m,
        g,
        spell,
        crate::game::map::spells::Targets {
            item: bracer,
            ..Default::default()
        },
        None,
    );
}

/// Thistle Tea's `Enchantments.apply_permanent` "commits exact target,
/// costs, ... and visible bonuses together", "checks ... missing materials
/// without partial changes", "replacement does not stack and bagged
/// enchants grant no bonus" and "removing enchanted equipment uses the
/// normal inventory commit", and `expire` "keeps permanent bonuses when
/// temporary enchants expire": an enchant on worn bracers takes its dust
/// and gives its five health; a second replaces it, five still; without
/// the dust, or at bracers gone, nothing changes; in the bags the bracers give nothing, worn
/// again they do; a temporary enchant's end leaves the permanent one; and
/// the bracers gone, so is the bonus.
#[tokio::test]
async fn an_enchant_takes_its_dust_gives_its_bonus_once_and_only_while_worn() {
    let realm = harness::realm_with(enchant_world());
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let base = on(&map, move |m| {
        crate::game::tests_p3_combat::equip(m, ga, BRACER, 8);
        crate::game::map::inventory::add_item(m, ga, DUST, 2, false).unwrap();
        crate::game::map::spells::learn(m, ga, ENCHANT);
        crate::game::map::spells::learn(m, ga, COAT);
        // No GM: she pays her dust.
        m.players.get_mut(&ga).unwrap().gm = false;
        enchanted(m, ga).0
    })
    .await;
    let once = on(&map, move |m| {
        enchant(m, ga, ENCHANT);
        enchanted(m, ga)
    })
    .await;
    assert_eq!(once, (base + 5, HEALTH_5, 1), "the dust spent, five health");
    let visible = on(&map, move |m| {
        m.players[&ga]
            .fields
            .get_u32(crate::game::fields::PLAYER_VISIBLE_ITEM_1_0 + 8 * 12 + 1)
    })
    .await;
    assert_eq!(visible, HEALTH_5, "shown on the wrist");
    let twice = on(&map, move |m| {
        enchant(m, ga, ENCHANT);
        enchanted(m, ga)
    })
    .await;
    assert_eq!(twice, (base + 5, HEALTH_5, 0), "replaced, not stacked");
    let without = on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.items.iter().position(|i| i.entry == BRACER).unwrap();
        p.items[i]
            .fields
            .set_u32(crate::game::fields::ITEM_FIELD_ENCHANTMENT, 0);
        let data = m.realm.data.clone();
        p.apply_stats(&data);
        enchant(m, ga, ENCHANT);
        enchanted(m, ga)
    })
    .await;
    assert_eq!(without, (base, 0, 0), "no dust, no change");
    // At bracers not hers (none there): nothing spent either.
    let stale = on(&map, move |m| {
        crate::game::map::inventory::add_item(m, ga, DUST, 1, false).unwrap();
        if let Some(c) = m.caster_mut(ga) {
            c.gcd_until = None;
        }
        crate::game::map::spells::cast(
            m,
            ga,
            ENCHANT,
            crate::game::map::spells::Targets {
                item: Some(crate::game::guid::item(0x7FFF_FFF0)),
                ..Default::default()
            },
            None,
        );
        let left = enchanted(m, ga);
        let p = m.players.get_mut(&ga).unwrap();
        p.items.retain(|i| i.entry != DUST);
        left
    })
    .await;
    assert_eq!(stale, (base, 0, 1), "a target gone, the dust kept");
    let (bagged, worn) = on(&map, move |m| {
        crate::game::map::inventory::add_item(m, ga, DUST, 1, false).unwrap();
        enchant(m, ga, ENCHANT);
        let data = m.realm.data.clone();
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.items.iter().position(|i| i.entry == BRACER).unwrap();
        p.items[i].slot = crate::game::map::inventory::PACK_START + 5;
        p.refresh_inventory(&data);
        p.apply_stats(&data);
        let bagged = enchanted(m, ga).0;
        let p = m.players.get_mut(&ga).unwrap();
        p.items[i].slot = 8;
        p.refresh_inventory(&data);
        p.apply_stats(&data);
        (bagged, enchanted(m, ga).0)
    })
    .await;
    assert_eq!((bagged, worn), (base, base + 5), "nothing in the bags");
    // A coat for ten seconds over it, run out: the permanent one stays.
    let coated = on(&map, move |m| {
        enchant(m, ga, COAT);
        enchanted(m, ga).0
    })
    .await;
    assert_eq!(coated, base + 10, "both");
    on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.items.iter().position(|i| i.entry == BRACER).unwrap();
        // Its time up (the temporary slot's duration).
        p.items[i]
            .fields
            .set_u32(crate::game::fields::ITEM_FIELD_ENCHANTMENT + 3 + 1, 1);
    })
    .await;
    let expired = until_health(&map, ga, base + 5).await;
    assert!(expired, "the coat gone, the enchant kept");
    let gone = on(&map, move |m| {
        let bracer = m.players[&ga]
            .items
            .iter()
            .find(|i| i.entry == BRACER)
            .unwrap()
            .guid;
        crate::game::map::items::destroy(m, ga, bracer);
        enchanted(m, ga).0
    })
    .await;
    assert_eq!(gone, base, "the bracers gone, the bonus with them");
}

/// Whether `g`'s maximum health comes to `want` within a few seconds.
async fn until_health(map: &crate::game::map::MapHandle, g: Guid, want: u32) -> bool {
    for _ in 0..300 {
        let h = on(map, move |m| {
            m.players[&g]
                .fields
                .get_u32(crate::game::fields::UNIT_FIELD_MAXHEALTH)
        })
        .await;
        if h == want {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

/// A vial of the coat, spent with each use.
const VIAL: u32 = 63_222;

/// Where `g`'s item `entry` is, and how many.
fn at_slot(m: &Map, g: Guid, entry: u32) -> Option<(u8, u8, u32)> {
    m.players[&g]
        .items
        .iter()
        .find(|i| i.entry == entry)
        .map(|i| (i.bag, i.slot, i.count()))
}

/// Thistle Tea's `Enchantments.apply_temporary` "commits a coating once
/// with charges, bonuses, and the client timer", "rejects ... dead casters
/// ... atomically" and "instant item use delegates consumption to the
/// owner transaction", and `restore` "death and reconnect preserve
/// remaining charges": a vial used on worn bracers coats them (their
/// temporary slot, its bonus on), the client told its time, and one vial
/// is spent; the dead use none; a relog keeps the coat's time and charges.
#[tokio::test]
async fn a_coating_is_one_vial_its_time_told_and_kept_with_its_charges_over_a_relog() {
    let mut d = enchant_world();
    d.items.insert(
        VIAL,
        ItemTemplate {
            entry: VIAL,
            name: "Vial".into(),
            stackable: 5,
            allowable_class: -1,
            allowable_race: -1,
            spells: [
                crate::game::data::items::ItemSpell {
                    id: COAT,
                    charges: -1,
                    ..Default::default()
                },
                Default::default(),
                Default::default(),
                Default::default(),
                Default::default(),
            ],
            ..Default::default()
        },
    );
    let realm = harness::realm_with(d);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (bracer, vial, base) = on(&map, move |m| {
        crate::game::tests_p3_combat::equip(m, ga, BRACER, 8);
        crate::game::map::inventory::add_item(m, ga, VIAL, 2, false).unwrap();
        let bracer = m.players[&ga]
            .items
            .iter()
            .find(|i| i.entry == BRACER)
            .unwrap()
            .guid;
        (bracer, at_slot(m, ga, VIAL).unwrap(), enchanted(m, ga).0)
    })
    .await;
    let coat = SpellCastTargets {
        target_flags: SpellCastTargets_SpellCastTargetFlags::empty()
            .set_item(SpellCastTargets_SpellCastTargetFlags_Item::Item { item: bracer }),
    };
    let use_vial = CMSG_USE_ITEM {
        bag_index: vial.0,
        bag_slot: vial.1,
        spell_index: 0,
        targets: coat,
    };
    // Dead, nothing.
    on(&map, move |m| crate::game::map::death::kill(m, ga)).await;
    a.send(use_vial.clone()).await;
    crate::game::tests_p3_player2_ammo::handled(&mut a).await;
    let dead = on(&map, move |m| at_slot(m, ga, VIAL).map(|v| v.2)).await;
    assert_eq!(dead, Some(2), "the dead spend none");
    on(&map, move |m| {
        crate::game::map::death::resurrect(m, ga, 1.0)
    })
    .await;
    a.clear();
    a.send(use_vial.clone()).await;
    let told = a
        .wait_for("SMSG_ITEM_ENCHANT_TIME_UPDATE", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_ENCHANT_TIME_UPDATE(t) if t.item == bracer => {
                Some((t.slot, t.duration))
            }
            _ => None,
        })
        .await;
    assert_eq!(told, (1, 10), "the temporary slot, ten seconds");
    let (left, health, coat_id) = on(&map, move |m| {
        let p = &m.players[&ga];
        let i = p.item_by_guid(bracer).unwrap();
        (
            at_slot(m, ga, VIAL).map(|v| v.2),
            enchanted(m, ga).0,
            p.items[i]
                .fields
                .get_u32(crate::game::fields::ITEM_FIELD_ENCHANTMENT + 3),
        )
    })
    .await;
    assert_eq!(
        (left, health, coat_id),
        (Some(1), base + 5, HEALTH_5),
        "one vial"
    );
    // Charges on it, then a relog: time and charges kept.
    on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.item_by_guid(bracer).unwrap();
        p.items[i]
            .fields
            .set_u32(crate::game::fields::ITEM_FIELD_ENCHANTMENT + 3 + 2, 7);
    })
    .await;
    a.send(CMSG_LOGOUT_REQUEST {}).await;
    a.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    a.clear();
    a.login(ga).await;
    let back = a
        .wait_for("the coat's time told", |m| match m {
            ServerOpcodeMessage::SMSG_ITEM_ENCHANT_TIME_UPDATE(t) if t.item == bracer => {
                Some(t.duration)
            }
            _ => None,
        })
        .await;
    assert!(back <= 10, "not renewed: {back}");
    let map = realm.registry.online(ga).unwrap().map();
    let charges = on(&map, move |m| {
        let p = &m.players[&ga];
        let i = p.item_by_guid(bracer).unwrap();
        p.items[i]
            .fields
            .get_u32(crate::game::fields::ITEM_FIELD_ENCHANTMENT + 3 + 2)
    })
    .await;
    assert_eq!(charges, 7, "its charges kept");
}
