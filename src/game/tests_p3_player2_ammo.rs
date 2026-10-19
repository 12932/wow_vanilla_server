//! Phase 3's player tests, ported from Thistle Tea's
//! `test/game/world/entity/player/ammunition_test.exs`
//! (docs/parity/matrix.csv): choosing ammunition, what a shot spends, and
//! what stops a repeat.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::spells::{Targets, wire_targets};
use crate::game::map::{Map, ranged};
use crate::game::store::BAG_PLAYER;
use crate::game::tests_p3_combat::{BRUTE, START, equip, setup_with, spawn};
use crate::game::tests_p3_entity::{NONE_WITHIN, SMSG_CAST_RESULT, on};

pub(crate) const BOW: u32 = 64_101;
pub(crate) const ARROW: u32 = 64_102;
const BULLET: u32 = 64_103;
/// Arrows for level 60, for hunters only, and for a skill no one has.
const HIGH_ARROW: u32 = 64_104;
const HUNTER_ARROW: u32 = 64_105;
const SKILLED_ARROW: u32 = 64_106;
pub(crate) const WAND: u32 = 64_107;
/// Throwing knives, a stack; a tomahawk, one of a point of wear.
const KNIFE: u32 = 64_108;
const TOMAHAWK: u32 = 64_109;

pub(crate) const AUTO_SHOT: u32 = 75;
pub(crate) const SHOOT: u32 = 5019;
const THROW: u32 = 2764;
/// A ranged ability (Arcane Shot's kind), and a ranged-class one that
/// isn't of the ranged slot (Blind's).
const ARCANE: u32 = 64_200;
const BLIND: u32 = 2094;

fn item(entry: u32, class: u32, subclass: u32, inventory_type: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        class,
        subclass,
        name: format!("Item {entry}"),
        display_id: entry - 64_000,
        inventory_type,
        stackable: 1,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    }
}

fn damage(lo: f32, hi: f32, school: u32) -> [(f32, f32, u32); 5] {
    let mut d = [(0.0, 0.0, 0); 5];
    d[0] = (lo, hi, school);
    d
}

pub(crate) fn world() -> GameData {
    let mut d = crate::game::tests_p3_combat::data();
    let mut bow = item(BOW, 2, 2, 15);
    bow.ammo_type = 2;
    bow.delay = 2000;
    bow.damages = damage(20.0, 20.0, 0);
    let mut wand = item(WAND, 2, 19, 26);
    wand.delay = 1500;
    wand.damages = damage(20.0, 20.0, 5);
    wand.max_durability = 50;
    let mut knife = item(KNIFE, 2, 16, 25);
    knife.delay = 2000;
    knife.stackable = 200;
    knife.damages = damage(20.0, 30.0, 0);
    let mut tomahawk = item(TOMAHAWK, 2, 16, 25);
    tomahawk.delay = 2000;
    tomahawk.max_durability = 1;
    tomahawk.damages = damage(20.0, 30.0, 0);
    let arrow = |entry: u32, subclass: u32| ItemTemplate {
        stackable: 200,
        damages: damage(10.0, 10.0, 0),
        ..item(entry, 6, subclass, 24)
    };
    let mut high = arrow(HIGH_ARROW, 2);
    high.required_level = 60;
    let mut hunter = arrow(HUNTER_ARROW, 2);
    hunter.allowable_class = 4;
    let mut skilled = arrow(SKILLED_ARROW, 2);
    skilled.required_skill = 999;
    skilled.required_skill_rank = 1;
    for t in [
        bow,
        wand,
        knife,
        tomahawk,
        arrow(ARROW, 2),
        arrow(BULLET, 3),
        high,
        hunter,
        skilled,
    ] {
        d.items.insert(t.entry, t);
    }
    let s = &mut d.spells;
    s.ranges.insert(114, (0.0, 35.0));
    let ranged = |id: u32, dmg_class: u32, mask: i32, effect: SpellEffect| {
        let mut t = SpellTemplate {
            id,
            name: format!("Spell {id}"),
            range_index: 114,
            equipped_item_class: 2,
            equipped_item_subclass_mask: mask,
            dmg_class,
            ..Default::default()
        };
        t.attributes[0] = crate::game::data::spells::ATTR_RANGED;
        t.effects[0] = effect;
        t
    };
    let weapon_hit = SpellEffect {
        effect: 58,
        target_a: 6,
        ..Default::default()
    };
    let bolt = SpellEffect {
        effect: 2,
        base_points: 9,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    let mut auto = ranged(AUTO_SHOT, 3, 0x4000C, weapon_hit);
    auto.attributes[2] = 0x20;
    let mut shoot = ranged(SHOOT, 1, 1 << 19, weapon_hit);
    shoot.attributes[2] = 0x20;
    let mut throw = ranged(THROW, 3, 1 << 16, weapon_hit);
    throw.attributes[2] = 0x20;
    let mut arcane = ranged(ARCANE, 3, 0x4000C, bolt);
    // Five rage.
    arcane.power_type = 1;
    arcane.mana_cost = 50;
    let mut blind = ranged(BLIND, 3, 0, bolt);
    blind.attributes[0] = 0;
    blind.equipped_item_class = -1;
    for t in [auto, shoot, throw, arcane, blind] {
        s.spells.insert(t.id, t);
    }
    d
}

/// Alice with a bow and two arrows, and a brute fifteen yards off.
async fn armed() -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    crate::game::map::MapHandle,
    Guid,
) {
    let (realm, a, ga, map) = setup_with(world(), Class::Warrior).await;
    let brute = on(&map, move |m| {
        equip(m, ga, BOW, 17);
        crate::game::map::inventory::add_item(m, ga, ARROW, 2, false).unwrap();
        let brute = spawn(m, ga, BRUTE, 20, 15.0);
        // It hits back for a point at most.
        let c = m.creature_mut(brute).unwrap();
        c.fields.set_f32(UNIT_FIELD_MINDAMAGE, 0.0);
        c.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 0.0);
        brute
    })
    .await;
    (realm, a, ga, map, brute)
}

pub(crate) fn count(m: &Map, g: Guid, entry: u32) -> u32 {
    m.players[&g].count_of(entry)
}

/// The name of a failure `c` is sent, waiting for it.
async fn failure(c: &mut Client) -> String {
    c.wait_for("SMSG_INVENTORY_CHANGE_FAILURE", |m| match m {
        ServerOpcodeMessage::SMSG_INVENTORY_CHANGE_FAILURE(x) => {
            let s = format!("{:?}", x.result);
            Some(s.split([' ', '{']).next().unwrap_or("").to_string())
        }
        _ => None,
    })
    .await
}

/// Wait until `c`'s earlier packets have been handled.
pub(crate) async fn handled(c: &mut Client) {
    c.send(CMSG_NAME_QUERY { guid: c.guid }).await;
    c.wait_for("SMSG_NAME_QUERY_RESPONSE", |m| match m {
        ServerOpcodeMessage::SMSG_NAME_QUERY_RESPONSE(_) => Some(()),
        _ => None,
    })
    .await;
}

/// `CMSG_SET_AMMO`, handled.
pub(crate) async fn choose(c: &mut Client, item: u32) {
    c.send(CMSG_SET_AMMO { item }).await;
    handled(c).await;
}

/// Fire `g`'s repeat now, as the update does when it is due.
pub(crate) fn fire(m: &mut Map, g: Guid) {
    let now = m.now;
    if let Some(c) = m.combat_mut(g)
        && let Some(a) = c.auto_repeat.as_mut()
    {
        a.next_at = now;
    }
    ranged::update(m);
}

/// How many of `spell`'s shots `c` sees go, in what has come.
pub(crate) async fn shots(c: &mut Client, spell: u32) -> usize {
    let mut n = 0;
    while c
        .gets(NONE_WITHIN, |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == spell => Some(()),
            _ => None,
        })
        .await
        .is_some()
    {
        n += 1;
    }
    n
}

pub(crate) fn repeating(m: &Map, g: Guid) -> bool {
    m.players[&g].combat.auto_repeat.is_some()
}

/// Thistle Tea's "dispatch selects ammunition, recomputes damage, and
/// saves the choice": `CMSG_SET_AMMO` chooses the arrows, which the bow's
/// damage takes in, and the choice is kept over a logout; clearing it
/// takes them out again.
#[tokio::test]
async fn chosen_ammunition_adds_its_damage_and_is_kept_over_a_logout() {
    let (realm, mut a, ga, map, _) = armed().await;
    let damage = |m: &Map, g: Guid| m.players[&g].fields.get_f32(UNIT_FIELD_MINRANGEDDAMAGE);
    let before = on(&map, move |m| damage(m, ga)).await;
    choose(&mut a, ARROW).await;
    let (chosen, with) = on(&map, move |m| {
        (m.players[&ga].fields.get_u32(PLAYER_AMMO_ID), damage(m, ga))
    })
    .await;
    assert_eq!(chosen, ARROW);
    // Ten a second over the bow's two seconds.
    assert!((with - before - 20.0).abs() < 0.01, "{before} then {with}");
    assert_eq!(
        on(&map, move |m| count(m, ga, ARROW)).await,
        2,
        "none spent"
    );
    a.disconnect();
    let mut b = Client::connect(&realm, "A");
    b.send(CMSG_CHAR_ENUM {}).await;
    b.wait_for("SMSG_CHAR_ENUM", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_CHAR_ENUM(_)).then_some(())
    })
    .await;
    b.login(ga).await;
    let map = realm.registry.online(ga).unwrap().map();
    let (kept, again) = on(&map, move |m| {
        (m.players[&ga].fields.get_u32(PLAYER_AMMO_ID), damage(m, ga))
    })
    .await;
    assert_eq!(kept, ARROW, "kept over the logout");
    assert!((again - with).abs() < 0.01, "{with} then {again}");
    choose(&mut b, 0).await;
    let (cleared, without) = on(&map, move |m| {
        (m.players[&ga].fields.get_u32(PLAYER_AMMO_ID), damage(m, ga))
    })
    .await;
    assert_eq!(cleared, 0);
    assert!((without - before).abs() < 0.01, "{before} then {without}");
}

/// Thistle Tea's "rejects absent, banked, non-ammo, and unusable items"
/// and "rejects selection and clearing while dead", as mangos's
/// `CanUseAmmo` refuses them: each is told why, and the choice stands.
#[tokio::test]
async fn ammunition_not_carried_not_ammunition_or_not_usable_is_refused() {
    let (_realm, mut a, ga, map, _) = armed().await;
    choose(&mut a, ARROW).await;
    on(&map, move |m| {
        for entry in [HIGH_ARROW, HUNTER_ARROW, SKILLED_ARROW, BULLET] {
            crate::game::map::inventory::add_item(m, ga, entry, 1, false).unwrap();
        }
        // Bullets in the bank.
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.items.iter().position(|i| i.entry == BULLET).unwrap();
        p.items[i].bag = BAG_PLAYER;
        p.items[i].slot = crate::game::map::inventory::BANK_START;
    })
    .await;
    for (entry, why) in [
        (64_999, "ItemNotFound"),
        (BOW, "OnlyAmmoCanGoHere"),
        (BULLET, "ItemNotFound"),
        (HIGH_ARROW, "CantEquipLevelI"),
        (HUNTER_ARROW, "YouCanNeverUseThatItem"),
        (SKILLED_ARROW, "NoRequiredProficiency"),
    ] {
        a.send(CMSG_SET_AMMO { item: entry }).await;
        assert_eq!(failure(&mut a).await, why, "{entry}");
    }
    let chosen = on(&map, move |m| m.players[&ga].fields.get_u32(PLAYER_AMMO_ID)).await;
    assert_eq!(chosen, ARROW, "the choice stands");
    on(&map, move |m| crate::game::map::death::kill(m, ga)).await;
    for entry in [ARROW, 0] {
        a.send(CMSG_SET_AMMO { item: entry }).await;
        assert_eq!(failure(&mut a).await, "YouAreDead", "{entry}");
    }
    let chosen = on(&map, move |m| m.players[&ga].fields.get_u32(PLAYER_AMMO_ID)).await;
    assert_eq!(chosen, ARROW, "not cleared by the dead");
}

/// Thistle Tea's "two arrows pay for exactly two repeats and depletion
/// cancels without another projectile" and "requires matching carried
/// ammo": each shot spends an arrow, and with none left the repeat stops,
/// its client told there is no ammo and the repeat cancelled, the choice
/// kept; bullets chosen don't load a bow.
#[tokio::test]
async fn two_arrows_pay_for_two_shots_and_then_the_repeat_stops() {
    let (_realm, mut a, ga, map, brute) = armed().await;
    choose(&mut a, ARROW).await;
    let spent = on(&map, move |m| {
        ranged::start(
            m,
            ga,
            AUTO_SHOT,
            Targets {
                unit: Some(brute),
                ..Default::default()
            },
        );
        let mut left = Vec::new();
        for _ in 0..2 {
            fire(m, ga);
            left.push((count(m, ga, ARROW), repeating(m, ga)));
        }
        fire(m, ga);
        left.push((count(m, ga, ARROW), repeating(m, ga)));
        (left, m.players[&ga].fields.get_u32(PLAYER_AMMO_ID))
    })
    .await;
    assert_eq!(spent.0, vec![(1, true), (0, true), (0, false)]);
    assert_eq!(spent.1, ARROW, "the choice kept");
    a.wait_frame("no ammo", SMSG_CAST_RESULT, |b| {
        b[..4] == AUTO_SHOT.to_le_bytes() && b.get(4..6) == Some(&[2, 0x43][..])
    })
    .await;
    a.wait_for("the repeat cancelled", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_CANCEL_AUTO_REPEAT).then_some(())
    })
    .await;
    assert_eq!(shots(&mut a, AUTO_SHOT).await, 2, "no third projectile");
    // Bullets chosen, in the place of arrows: a bow has no ammunition.
    let refused = on(&map, move |m| {
        crate::game::map::inventory::add_item(m, ga, BULLET, 5, false).unwrap();
        m.players
            .get_mut(&ga)
            .unwrap()
            .fields
            .set_u32(PLAYER_AMMO_ID, BULLET);
        ranged::pay_ammunition(m, ga).err()
    })
    .await;
    assert_eq!(refused, Some(0x43), "no ammo");
}

/// `g` takes `weapon` into the ranged slot and starts `spell` at `target`.
fn shoot_with(m: &mut Map, g: Guid, weapon: u32, count: u32, spell: u32, target: Guid) {
    equip(m, g, weapon, 17);
    if count > 1 {
        let i = m.players[&g]
            .items
            .iter()
            .position(|i| i.bag == BAG_PLAYER && i.slot == 17)
            .unwrap();
        m.players.get_mut(&g).unwrap().items[i].set_count(count);
    }
    m.players.get_mut(&g).unwrap().derived.ranged_hit = 100.0;
    ranged::start(
        m,
        g,
        spell,
        Targets {
            unit: Some(target),
            ..Default::default()
        },
    );
}

/// Thistle Tea's "wand repeats use the equipped school and skill without
/// consuming ammunition" and "queued wand shots recheck movement, broken
/// gear, and weapon replacement": a wand's Shoot strikes with the wand's
/// school and damage and spends no arrows; it stops when its shooter
/// moves, when the wand breaks, and when a bow takes its place.
#[tokio::test]
async fn a_wand_spends_nothing_and_stops_when_moved_broken_or_replaced() {
    let (_realm, mut a, ga, map, brute) = armed().await;
    choose(&mut a, ARROW).await;
    let (arrows, worn) = on(&map, move |m| {
        shoot_with(m, ga, WAND, 1, SHOOT, brute);
        fire(m, ga);
        let p = &m.players[&ga];
        let wand = p.items.iter().find(|i| i.entry == WAND).unwrap();
        (
            count(m, ga, ARROW),
            wand.fields.get_u32(ITEM_FIELD_DURABILITY),
        )
    })
    .await;
    assert_eq!((arrows, worn), (2, 50), "no arrow and no wear");
    let (school, amount) = a
        .wait_for("the wand's blow", |m| match m {
            ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(x) if x.spell == SHOOT => {
                Some((x.school, x.damage))
            }
            _ => None,
        })
        .await;
    assert_eq!(school, SpellSchool::Shadow, "the wand's school");
    assert!(amount == 20 || amount == 30, "the wand's damage: {amount}");
    // Moving stops it.
    a.heartbeat(START.0 + 1.0, START.1, START.2).await;
    handled(&mut a).await;
    let moved = on(&map, move |m| repeating(m, ga)).await;
    assert!(!moved, "stopped by moving");
    // A broken wand stops it.
    let broken = on(&map, move |m| {
        ranged::start(
            m,
            ga,
            SHOOT,
            Targets {
                unit: Some(brute),
                ..Default::default()
            },
        );
        let p = m.players.get_mut(&ga).unwrap();
        let i = p.items.iter().position(|i| i.entry == WAND).unwrap();
        p.items[i].fields.set_u32(ITEM_FIELD_DURABILITY, 0);
        fire(m, ga);
        repeating(m, ga)
    })
    .await;
    assert!(!broken, "stopped by a broken wand");
    // A bow in the wand's place stops it.
    let replaced = on(&map, move |m| {
        shoot_with(m, ga, WAND, 1, SHOOT, brute);
        equip(m, ga, BOW, 17);
        fire(m, ga);
        (repeating(m, ga), count(m, ga, ARROW))
    })
    .await;
    assert_eq!(replaced, (false, 2), "stopped by a bow, an arrow unspent");
    a.clear();
}

/// Thistle Tea's "the final thrown weapon retains damage and its
/// projectile before removal" and "nonstacking thrown weapons spend
/// durability without consuming selected arrows": the last knife thrown
/// still strikes as a knife and is seen to fly, then is gone and the
/// throwing stops; a tomahawk wears down and leaves the arrows chosen, and
/// broken it throws no more.
#[tokio::test]
async fn the_last_knife_strikes_as_a_knife_and_a_tomahawk_wears_out() {
    let (_realm, mut a, ga, map, brute) = armed().await;
    choose(&mut a, ARROW).await;
    let (left, again) = on(&map, move |m| {
        shoot_with(m, ga, KNIFE, 1, THROW, brute);
        fire(m, ga);
        let left = count(m, ga, KNIFE);
        fire(m, ga);
        (left, repeating(m, ga))
    })
    .await;
    assert_eq!(
        (left, again),
        (0, false),
        "the last knife gone, and no more"
    );
    let go = a
        .wait_for("the throw", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == THROW => Some(s.flags),
            _ => None,
        })
        .await;
    let ammo = go.get_ammo().expect("a projectile");
    assert_eq!(
        (ammo.ammo_display_id, ammo.ammo_inventory_type),
        (KNIFE - 64_000, 25),
        "the knife seen to fly"
    );
    let struck = a
        .wait_for("the knife's blow", |m| match m {
            ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(x) if x.spell == THROW => {
                Some(x.damage)
            }
            _ => None,
        })
        .await;
    assert!(struck >= 20, "struck as a knife: {struck}");
    assert_eq!(shots(&mut a, THROW).await, 0, "one throw only");
    let (worn, arrows, again) = on(&map, move |m| {
        shoot_with(m, ga, TOMAHAWK, 1, THROW, brute);
        fire(m, ga);
        let p = &m.players[&ga];
        let t = p.items.iter().find(|i| i.entry == TOMAHAWK).unwrap();
        let worn = t.fields.get_u32(ITEM_FIELD_DURABILITY);
        fire(m, ga);
        (worn, count(m, ga, ARROW), repeating(m, ga))
    })
    .await;
    assert_eq!(worn, 0, "worn out");
    assert_eq!(arrows, 2, "the arrows untouched");
    assert!(!again, "a broken tomahawk throws no more");
    assert_eq!(shots(&mut a, THROW).await, 1, "one throw of the tomahawk");
}

/// Thistle Tea's "canceled repeats and casts cannot spend items later":
/// a repeat cancelled before its shot spends nothing when the shot would
/// have been due.
#[tokio::test]
async fn a_cancelled_repeat_spends_nothing_later() {
    let (_realm, mut a, ga, map, brute) = armed().await;
    choose(&mut a, ARROW).await;
    on(&map, move |m| {
        shoot_with(m, ga, BOW, 1, AUTO_SHOT, brute);
    })
    .await;
    a.send(CMSG_CANCEL_AUTO_REPEAT_SPELL {}).await;
    handled(&mut a).await;
    let left = on(&map, move |m| {
        fire(m, ga);
        count(m, ga, ARROW)
    })
    .await;
    assert_eq!(left, 2);
    assert_eq!(shots(&mut a, AUTO_SHOT).await, 0);
}

/// Thistle Tea's "successful cast launches once and pays exactly one
/// arrow", "cast completion rechecks ammunition before spending mana or
/// sending spell go", and "wands, non-ranged spells, and ranged exceptions
/// spend no ammunition": a ranged ability spends one arrow and is seen
/// with it; with none left it is refused, its mana kept and nothing seen
/// to go; a spell of the ranged class that isn't of the ranged slot
/// (Blind) spends none.
#[tokio::test]
async fn a_ranged_ability_spends_one_arrow_and_without_one_costs_nothing() {
    let (_realm, mut a, ga, map, brute) = armed().await;
    choose(&mut a, ARROW).await;
    on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.spells.extend([ARCANE, BLIND]);
        p.fields.set_u32(UNIT_FIELD_POWER1 + 1, 500);
    })
    .await;
    let cast = |spell: u32| CMSG_CAST_SPELL {
        spell,
        targets: wire_targets(&Targets {
            unit: Some(brute),
            ..Default::default()
        }),
    };
    a.send(cast(BLIND)).await;
    a.wait_for("Blind", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == BLIND => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(
        on(&map, move |m| count(m, ga, ARROW)).await,
        2,
        "Blind spends none"
    );
    a.send(cast(ARCANE)).await;
    let go = a
        .wait_for("the shot", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == ARCANE => Some(s.flags),
            _ => None,
        })
        .await;
    let ammo = go.get_ammo().expect("an arrow seen");
    assert_eq!(ammo.ammo_display_id, ARROW - 64_000);
    assert_eq!(on(&map, move |m| count(m, ga, ARROW)).await, 1, "one arrow");
    let paid = on(&map, move |m| {
        m.players[&ga].fields.get_u32(UNIT_FIELD_POWER1 + 1)
    })
    .await;
    assert!(paid < 500, "the shot's rage paid: {paid}");
    assert_eq!(shots(&mut a, ARCANE).await, 0, "launched once");
    // None left: refused, and nothing paid.
    let rage = on(&map, move |m| {
        let p = m.players.get_mut(&ga).unwrap();
        p.items.retain(|i| i.entry != ARROW);
        p.caster.gcd_until = None;
        p.fields.get_u32(UNIT_FIELD_POWER1 + 1)
    })
    .await;
    a.send(cast(ARCANE)).await;
    a.wait_frame("no ammo", SMSG_CAST_RESULT, |b| {
        b[..4] == ARCANE.to_le_bytes() && b[4] == 2
    })
    .await;
    assert_eq!(shots(&mut a, ARCANE).await, 0, "nothing seen to go");
    let after = on(&map, move |m| {
        m.players[&ga].fields.get_u32(UNIT_FIELD_POWER1 + 1)
    })
    .await;
    assert_eq!(after, rage);
}
