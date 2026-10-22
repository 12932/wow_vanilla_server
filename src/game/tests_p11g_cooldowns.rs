//! Round 11, cooldowns across an equip and a login, against Thistle Tea's
//! `equipment_transitions_test.exs`, `deferred_cooldowns_test.exs` and
//! `game_object_summons_test.exs` (mangos-classic the authority): an item's
//! waiting cooldown started as it is put on, with its own times; the
//! cooldowns `SMSG_INITIAL_SPELLS` carries, with their items and
//! categories, a waiting one flagged; and a cooldown waiting for an
//! object started, and saved, as its owner logs out.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::harness::Client;
use super::tests_p3_cast_kit::*;
use super::tests_p3_spells_kit::{self as kit};
use crate::game::data::GameData;
use crate::game::data::spells::SpellEffect;
use crate::game::map::{Map, auras, objects, spells};

/// (The client's spell and item ids are 16 bits.) An aura whose cooldown of ten seconds waits for it to go, of category
/// 94, and the trinket that casts it, of a minute's cooldown and ten
/// seconds' for the category.
const WAITS: u32 = 60_901;
const TRINKET: u32 = 60_902;
const WAIT_CATEGORY: u32 = 94;
/// A spell no one learns, cast from a charm of a minute's cooldown and
/// half a minute's for category 95.
const CHARM_SPELL: u32 = 60_903;
const CHARM: u32 = 60_904;
const CHARM_CATEGORY: u32 = 95;
/// A spell that raises a post, whose minute waits for the post to go.
const RAISE: u32 = 60_905;
const POST: u32 = 60_906;

fn data(d: &mut GameData) {
    let mut waits = bare(WAITS, 0);
    waits.attributes[0] |= 0x200_0000;
    waits.recovery_time = 10_000;
    waits.category = WAIT_CATEGORY;
    waits.duration_index = LASTS_LONG;
    effect(&mut waits, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
    let mut charm_spell = bare(CHARM_SPELL, 0);
    effect(&mut charm_spell, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
    let mut raise = bare(RAISE, 0);
    raise.recovery_time = 60_000;
    raise.attributes[0] |= 0x200_0000;
    effect(
        &mut raise,
        0,
        SpellEffect {
            effect: objects::EFFECT_TRANS_DOOR,
            misc_value: POST as i32,
            target_a: 1,
            ..Default::default()
        },
    );
    insert(d, [waits, charm_spell, raise]);
    let item = |entry: u32, name: &str, spell: u32, category: u32, cooldowns: (i32, i32)| {
        let mut t = crate::game::data::items::ItemTemplate {
            entry,
            class: 4,
            inventory_type: 12,
            name: name.into(),
            display_id: 1000,
            stackable: 1,
            ..Default::default()
        };
        t.spells[0] = crate::game::data::items::ItemSpell {
            id: spell,
            cooldown: cooldowns.0,
            category,
            category_cooldown: cooldowns.1,
            ..Default::default()
        };
        t
    };
    d.items.insert(
        TRINKET,
        item(
            TRINKET,
            "Waiting Trinket",
            WAITS,
            WAIT_CATEGORY,
            (60_000, 10_000),
        ),
    );
    d.items.insert(
        CHARM,
        item(
            CHARM,
            "Charm",
            CHARM_SPELL,
            CHARM_CATEGORY,
            (60_000, 30_000),
        ),
    );
    d.gameobject_templates.insert(
        POST,
        crate::game::tests_p3_entity_objects::object(POST, 5, "Post", &[]),
    );
}

/// `entry`'s item given to `g`: its GUID, bag and slot.
fn give(m: &mut Map, g: Guid, entry: u32) -> (Guid, u8, u8) {
    crate::game::map::inventory::add_item(m, g, entry, 1, false).expect("given");
    let i = m.players[&g]
        .items
        .iter()
        .find(|i| i.entry == entry)
        .expect("carried");
    (i.guid, i.bag, i.slot)
}

/// Milliseconds left of a spell's and a category's cooldowns.
fn left(m: &Map, g: Guid, spell: u32, category: u32) -> (Option<u64>, Option<u64>) {
    let now = m.now;
    let c = m.caster(g).unwrap();
    let ms = |t: Option<&std::time::Instant>| {
        t.filter(|t| **t > now)
            .map(|t| t.saturating_duration_since(now).as_millis() as u64)
    };
    (
        ms(c.cooldowns.get(&spell)),
        ms(c.category_cooldowns.get(&category)),
    )
}

/// Log `g` out and in again: the cooldowns `SMSG_INITIAL_SPELLS` carries.
async fn relog(c: &mut Client, g: Guid) -> Vec<CooldownSpell> {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
    c.clear();
    c.login(g).await;
    c.wait_for("SMSG_INITIAL_SPELLS", |m| match m {
        ServerOpcodeMessage::SMSG_INITIAL_SPELLS(s) => Some(s.cooldowns.clone()),
        _ => None,
    })
    .await
}

/// A trinket's aura whose cooldown waits for it to go, cast from the
/// trinket in a bag, starts as the trinket is put on: not the thirty
/// seconds an equip gives, but the trinket's own minute and its
/// category's ten seconds the cast kept, told with `SMSG_COOLDOWN_EVENT`
/// before `SMSG_ITEM_COOLDOWN` (mangos-classic's `ApplyEquipCooldown`:
/// `AddCooldown` lets go of the waiting entry and picks the old item's
/// cooldowns over the thirty; Thistle Tea's "activates a deferred item
/// cooldown using its retained overrides").
#[tokio::test]
async fn a_trinkets_waiting_cooldown_starts_with_its_own_times_as_it_is_put_on() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P11GCD1", "Trinketeer", Class::Rogue).await;
    let (slot, waiting) = map
        .query(move |m| {
            kit::ready(m, a, 60, &[]);
            let (item, bag, slot) = give(m, a, TRINKET);
            let cast = spells::cast(m, a, WAITS, spells::Targets::default(), Some(item));
            let pending = m.caster(a).unwrap().pending_cooldowns.contains(&WAITS);
            ((bag, slot), (cast, pending))
        })
        .await
        .unwrap();
    assert_eq!(waiting, (true, true), "cast from the trinket, waiting");
    c.clear();
    c.send(CMSG_AUTOEQUIP_ITEM {
        source_bag: slot.0,
        source_slot: slot.1,
    })
    .await;
    // The two told, in order.
    let mut told = Vec::new();
    while told.len() < 2 {
        let m = c
            .gets(std::time::Duration::from_secs(5), |m| match m {
                ServerOpcodeMessage::SMSG_COOLDOWN_EVENT(e) if e.id == WAITS => Some("event"),
                ServerOpcodeMessage::SMSG_ITEM_COOLDOWN(e) if e.id == WAITS => Some("item"),
                _ => None,
            })
            .await;
        match m {
            Some(m) => told.push(m),
            None => break,
        }
    }
    assert_eq!(told, ["event", "item"]);
    let out = map
        .query(move |m| {
            (
                m.caster(a).unwrap().pending_cooldowns.contains(&WAITS),
                left(m, a, WAITS, WAIT_CATEGORY),
            )
        })
        .await
        .unwrap();
    assert!(!out.0, "no longer waiting");
    assert!(
        out.1.0.is_some_and(|ms| ms > 50_000 && ms <= 60_000),
        "the trinket's minute: {:?}",
        out.1
    );
    assert!(
        out.1.1.is_some_and(|ms| ms > 5_000 && ms <= 10_000),
        "its category's ten seconds: {:?}",
        out.1
    );
}

/// At login, `SMSG_INITIAL_SPELLS` carries the cooldowns: a charm's spell
/// never learnt, with the charm and its category, a minute's and half a
/// minute's left; and an aura kept across the logout whose cooldown waits
/// for it to go, a millisecond and the category's `0x80000000` (mangos-
/// classic's `SendInitialSpells`, and `_LoadAuras` adding the waiting
/// cooldown; Thistle Tea's "includes unlearned item spells and the
/// permanent cooldown wire flag"). No `SMSG_SPELL_COOLDOWN` follows.
#[tokio::test]
async fn the_initial_spells_carry_each_cooldown_with_its_item_and_category() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P11GCD2", "Charmer", Class::Rogue).await;
    let cast = map
        .query(move |m| {
            kit::ready(m, a, 60, &[WAITS]);
            let (charm, _, _) = give(m, a, CHARM);
            let from_charm =
                spells::cast(m, a, CHARM_SPELL, spells::Targets::default(), Some(charm));
            if let Some(c) = m.caster_mut(a) {
                c.casting = None;
                c.gcd_until = None;
            }
            let waits = spells::cast(m, a, WAITS, spells::Targets::default(), None);
            let pending = m.caster(a).unwrap().pending_cooldowns.contains(&WAITS);
            (from_charm, waits, pending)
        })
        .await
        .unwrap();
    assert_eq!(cast, (true, true, true));
    let mut cooldowns = relog(&mut c, a).await;
    cooldowns.sort();
    let at = |id: u32| {
        cooldowns
            .iter()
            .find(|x| u32::from(x.spell_id) == id)
            .copied()
            .unwrap_or_else(|| panic!("no {id} in {cooldowns:?}"))
    };
    let charm = at(CHARM_SPELL);
    assert_eq!(
        (charm.item_id, charm.spell_category),
        (CHARM as u16, CHARM_CATEGORY as u16),
        "the charm and its category"
    );
    let ms = |d: std::time::Duration| d.as_millis() as u32;
    assert!(ms(charm.cooldown) > 50_000 && ms(charm.cooldown) <= 60_000);
    assert!(ms(charm.category_cooldown) > 20_000 && ms(charm.category_cooldown) <= 30_000);
    let waits = at(WAITS);
    assert_eq!(
        (
            waits.item_id,
            waits.spell_category,
            ms(waits.cooldown),
            ms(waits.category_cooldown)
        ),
        (0, WAIT_CATEGORY as u16, 1, 0x8000_0000),
        "waiting for its aura"
    );
    assert_eq!(cooldowns.len(), 2, "{cooldowns:?}");
    let again = c
        .gets(std::time::Duration::from_millis(300), |m| {
            matches!(m, ServerOpcodeMessage::SMSG_SPELL_COOLDOWN(_)).then_some(())
        })
        .await;
    assert!(again.is_none(), "told once, at the initial spells");
    // Still waiting in the world, and started as the aura goes.
    let after = map
        .query(move |m| {
            let waiting = m.caster(a).unwrap().pending_cooldowns.contains(&WAITS);
            auras::remove_spell(m, a, WAITS, None);
            (waiting, left(m, a, WAITS, WAIT_CATEGORY).0)
        })
        .await
        .unwrap();
    assert_eq!(after, (true, Some(10_000)));
}

/// A post whose spell's minute waits for it to go, its owner logging out
/// with it standing: the minute starts as they leave, and is saved, told
/// at their next login (mangos-classic's `Unit::RemoveGameObject` at
/// `RemoveFromWorld`, `AddCooldown` before the save; Thistle Tea's
/// "logout saves an activated object cooldown").
#[tokio::test]
async fn a_cooldown_waiting_for_an_object_is_started_and_saved_at_logout() {
    let realm = fast_realm(data);
    let (mut c, a, map) = human(&realm, "P11GCD3", "Poster", Class::Rogue).await;
    let raised = map
        .query(move |m| {
            kit::ready(m, a, 60, &[RAISE]);
            let cast = spells::cast(m, a, RAISE, spells::Targets::default(), None);
            let post = m
                .objects
                .summoned
                .iter()
                .any(|s| s.owner == Some(a) && s.spell == RAISE);
            let pending = m.caster(a).unwrap().pending_cooldowns.contains(&RAISE);
            (cast, post, pending)
        })
        .await
        .unwrap();
    assert_eq!(raised, (true, true, true), "raised, waiting for the post");
    let cooldowns = relog(&mut c, a).await;
    let raise = cooldowns
        .iter()
        .find(|x| u32::from(x.spell_id) == RAISE)
        .copied();
    let ms = raise.map(|x| x.cooldown.as_millis() as u32);
    assert!(
        ms.is_some_and(|ms| ms > 50_000 && ms <= 60_000),
        "the minute, started at the logout: {cooldowns:?}"
    );
    let back = map.query(move |m| left(m, a, RAISE, 0).0).await.unwrap();
    assert!(back.is_some_and(|ms| ms > 50_000), "{back:?}");
}

/// A spell whose wait is its category's alone, in three ranks, as Frost
/// Nova's (Spell.dbc's category 35, 25 seconds, no wait of its own), and a
/// spell of another category.
const NOVA: [u32; 3] = [60_907, 60_908, 60_909];
const NOVA_CATEGORY: u32 = 97;
const OTHER: u32 = 60_910;

/// A rank of Frost Nova cast holds its other ranks: the server refuses
/// them, and its client is told of each known rank of the category in the
/// cast's `SMSG_SPELL_COOLDOWN` (a time there is a spell's own to the
/// client: it greys only the spells named), as mangos's
/// `AddSpellAndCategoryCooldowns` holds each; the go names the caster in
/// the item's place, so the client starts the category's wait itself as
/// well (mangos's `SendSpellGo`). A spell of another category is not told.
/// `.debug cooldowns` clears each rank.
#[tokio::test]
async fn a_rank_cast_tells_its_client_of_every_known_rank_of_its_category() {
    let realm = fast_realm(|d| {
        let ranks = NOVA.map(|id| {
            let mut s = bare(id, 16);
            s.category = NOVA_CATEGORY;
            s.category_recovery_time = 25_000;
            effect(&mut s, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
            s
        });
        let mut other = bare(OTHER, 16);
        other.category = NOVA_CATEGORY + 1;
        other.category_recovery_time = 25_000;
        effect(&mut other, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
        insert(d, ranks);
        insert(d, [other]);
    });
    let (mut c, g, map) = human(&realm, "P11G4", "Novice", Class::Mage).await;
    map.query(move |m| {
        kit::ready(m, g, 60, &[NOVA[0], NOVA[1], NOVA[2], OTHER]);
    })
    .await
    .unwrap();
    c.clear();
    c.send(CMSG_CAST_SPELL {
        spell: NOVA[0],
        targets: spells::wire_targets(&Default::default()),
    })
    .await;
    let named = c
        .wait_for("SMSG_SPELL_GO", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == NOVA[0] => Some(s.cast_item),
            _ => None,
        })
        .await;
    assert_eq!(named, g, "the go names its caster in the item's place");
    let mut told = c
        .wait_for("SMSG_SPELL_COOLDOWN", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_COOLDOWN(s) if s.guid == g => Some(
                s.cooldowns
                    .iter()
                    .map(|x| (x.id, x.cooldown_time.as_millis() as u64))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .await;
    told.sort_unstable();
    assert_eq!(
        told,
        NOVA.map(|id| (id, 25_000)).to_vec(),
        "every known rank, the category's time, nothing of another category"
    );
    for rank in [NOVA[1], NOVA[2]] {
        c.send(CMSG_CAST_SPELL {
            spell: rank,
            targets: spells::wire_targets(&Default::default()),
        })
        .await;
        let f = c
            .wait_frame("SMSG_CAST_RESULT", 0x130, |f| {
                u32::from_le_bytes(f[0..4].try_into().unwrap()) == rank
            })
            .await;
        assert_eq!(
            (f[4], f[5]),
            (2, spells::fail::NOT_READY),
            "{rank} refused as cooling down"
        );
    }
    // `.debug cooldowns` lets the client have each rank back: none has a
    // wait of its own, the category's held them all.
    c.clear();
    c.say(".debug cooldowns").await;
    for rank in NOVA {
        c.wait_for("SMSG_CLEAR_COOLDOWN", |m| match m {
            ServerOpcodeMessage::SMSG_CLEAR_COOLDOWN(x) if x.id == rank => Some(()),
            _ => None,
        })
        .await;
    }
}

/// Food and drink as the data has them: Food's and Drink's spells wait a
/// minute for their categories (11 and 59) and have no global cooldown;
/// the items that cast them have none of their own and a second for the
/// category (Tough Jerky, Tough Hunk of Bread, Refreshing Spring Water).
/// And a spell on the global cooldown, cast between.
const FOOD: u32 = 60_911;
const OTHER_FOOD: u32 = 60_912;
const DRINK: u32 = 60_913;
const JERKY: u32 = 60_914;
const BREAD: u32 = 60_915;
const WATER: u32 = 60_916;
const ON_GCD: u32 = 60_917;
const FOOD_CATEGORY: u32 = 11;
const DRINK_CATEGORY: u32 = 59;

fn food(d: &mut GameData) {
    let spell = |id: u32, category: u32| {
        let mut s = bare(id, 0);
        s.category = category;
        s.category_recovery_time = 60_000;
        s.duration_index = LASTS_15S;
        effect(&mut s, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
        s
    };
    let mut on_gcd = bare(ON_GCD, 0);
    on_gcd.start_recovery_category = spells::GCD_SPELLS;
    on_gcd.start_recovery_time = 1_500;
    effect(&mut on_gcd, 0, aura(auras::AURA_DUMMY, 0, 1, 1));
    insert(
        d,
        [
            spell(FOOD, FOOD_CATEGORY),
            spell(OTHER_FOOD, FOOD_CATEGORY),
            spell(DRINK, DRINK_CATEGORY),
            on_gcd,
        ],
    );
    for (entry, spell, category) in [
        (JERKY, FOOD, FOOD_CATEGORY),
        (BREAD, OTHER_FOOD, FOOD_CATEGORY),
        (WATER, DRINK, DRINK_CATEGORY),
    ] {
        let mut t = crate::game::data::items::ItemTemplate {
            entry,
            name: format!("Made-up {entry}"),
            display_id: 1000,
            stackable: 20,
            ..Default::default()
        };
        t.spells[0] = crate::game::data::items::ItemSpell {
            id: spell,
            cooldown: 0,
            category,
            category_cooldown: 1_000,
            ..Default::default()
        };
        d.items.insert(entry, t);
    }
}

/// `item` (at `at`) used: whether its spell went, and what its client
/// was told of cooldowns meanwhile: the `SMSG_SPELL_COOLDOWN` entries and
/// how many `SMSG_ITEM_COOLDOWN`s.
async fn eat(c: &mut Client, at: (u8, u8), spell: u32) -> (bool, Vec<(u32, u64)>, usize) {
    c.clear();
    c.send(CMSG_USE_ITEM {
        bag_index: at.0,
        bag_slot: at.1,
        spell_index: 0,
        targets: spells::wire_targets(&Default::default()),
    })
    .await;
    let result = c
        .wait_frame("SMSG_CAST_RESULT", spells::SMSG_CAST_RESULT, |b| {
            b[..4] == spell.to_le_bytes()
        })
        .await;
    let went = result[4] == 0;
    if went {
        c.wait_for("the go", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == spell => Some(()),
            _ => None,
        })
        .await;
    } else {
        assert_eq!(result[5], spells::fail::NOT_READY, "refused as not ready");
    }
    // What else comes of it.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let mut told = Vec::new();
    while let Some(x) = c
        .gets(std::time::Duration::ZERO, |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_COOLDOWN(s) => Some(
                s.cooldowns
                    .iter()
                    .map(|x| (x.id, x.cooldown_time.as_millis() as u64))
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .await
    {
        told.extend(x);
    }
    let items = c.take_frames(0x0B0).len();
    (went, told, items)
}

/// The user's "eating food should not have a cooldown, only the initial
/// eat should have a 1 second GCD, same goes for drink": a player eats
/// Jerky, and the go names the Jerky; no `SMSG_ITEM_COOLDOWN` is sent
/// (that is an equip's: the client starts thirty seconds on the item's
/// spell for it), and what `SMSG_SPELL_COOLDOWN` says is a second at most.
/// Bread, of the same category, is refused for that second, and goes
/// after it; Water, of another category, goes at once, as does a spell on
/// the global cooldown, which eating doesn't start. The server holds the
/// category a second, not the spell's minute (mangos's `AddCooldown`: an
/// item's times of zero or more over its spell's).
#[tokio::test]
async fn food_and_drink_wait_a_second_for_their_category_and_no_more() {
    let realm = fast_realm(food);
    let (mut c, g, map) = human(&realm, "P11GFOOD", "Glutton", Class::Warrior).await;
    let (jerky, bread, water) = map
        .query(move |m| {
            kit::ready(m, g, 60, &[ON_GCD]);
            let at = |m: &mut Map, entry| {
                let (_, bag, slot) = give(m, g, entry);
                (bag, slot)
            };
            (at(m, JERKY), at(m, BREAD), at(m, WATER))
        })
        .await
        .unwrap();
    let jerky_guid = map
        .query(move |m| m.players[&g].items[m.players[&g].item_at(jerky.0, jerky.1).unwrap()].guid)
        .await
        .unwrap();
    c.clear();
    c.send(CMSG_USE_ITEM {
        bag_index: jerky.0,
        bag_slot: jerky.1,
        spell_index: 0,
        targets: spells::wire_targets(&Default::default()),
    })
    .await;
    let named = c
        .wait_for("the Jerky's go", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == FOOD => Some(s.cast_item),
            _ => None,
        })
        .await;
    assert_eq!(named, jerky_guid, "the go names the Jerky");
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let mut told = Vec::new();
    while let Some(x) = c
        .gets(std::time::Duration::ZERO, |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_COOLDOWN(s) => Some(s.cooldowns.clone()),
            _ => None,
        })
        .await
    {
        told.extend(
            x.into_iter()
                .map(|x| (x.id, x.cooldown_time.as_millis() as u64)),
        );
    }
    assert!(
        told.iter().all(|(id, ms)| *id == FOOD && *ms <= 1_000),
        "a second at most, for the Food alone: {told:?}"
    );
    assert_eq!(c.take_frames(0x0B0).len(), 0, "no SMSG_ITEM_COOLDOWN");
    let held = map
        .query(move |m| (left(m, g, FOOD, FOOD_CATEGORY), left(m, g, OTHER_FOOD, 0).0))
        .await
        .unwrap();
    assert_eq!(held.0.0, None, "no wait of the Food's own");
    assert!(
        held.0.1.is_some_and(|ms| ms <= 1_000),
        "the category's second, not a minute: {held:?}"
    );
    assert_eq!(held.1, None);
    // The Bread waits out the second; the Water and a spell on the global
    // cooldown don't.
    assert_eq!(
        eat(&mut c, bread, OTHER_FOOD).await,
        (false, vec![], 0),
        "the Bread refused"
    );
    let drank = eat(&mut c, water, DRINK).await;
    assert!(drank.0, "drinking after eating: {drank:?}");
    assert!(
        drank.1.iter().all(|(id, ms)| *id == DRINK && *ms <= 1_000),
        "{drank:?}"
    );
    assert_eq!(drank.2, 0, "no SMSG_ITEM_COOLDOWN");
    c.clear();
    c.send(CMSG_CAST_SPELL {
        spell: ON_GCD,
        targets: spells::wire_targets(&Default::default()),
    })
    .await;
    c.wait_for(
        "a spell on the global cooldown, after eating",
        |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(s) if s.spell == ON_GCD => Some(()),
            _ => None,
        },
    )
    .await;
    // A second on (the cooldowns put back by one, not the map's clock run
    // on), the Bread goes.
    map.query(move |m| {
        let c = m.caster_mut(g).unwrap();
        let back = std::time::Duration::from_millis(1_100);
        for t in c
            .cooldowns
            .values_mut()
            .chain(c.category_cooldowns.values_mut())
        {
            *t = t.checked_sub(back).unwrap_or(*t);
        }
    })
    .await
    .unwrap();
    let ate = eat(&mut c, bread, OTHER_FOOD).await;
    assert!(ate.0, "the Bread after a second: {ate:?}");
    assert_eq!(ate.2, 0, "no SMSG_ITEM_COOLDOWN");
}

/// The same on the real world's food and drink: Tough Jerky (117) and
/// Tough Hunk of Bread (4540), both Food (433, a minute for category 11 in
/// Spell.dbc), and Refreshing Spring Water (159, Drink, 430): each a
/// second for its category, from the item, and no `SMSG_ITEM_COOLDOWN`.
#[tokio::test]
#[ignore = "extended"]
async fn the_real_food_and_drink_wait_a_second() {
    let d = crate::game::harness::load_real(&crate::game::harness::real_data("db"));
    let realm = crate::game::harness::realm_with(d);
    let (mut c, g, map) = human(&realm, "P11GREALFOOD", "Gourmand", Class::Warrior).await;
    let (jerky, bread, water) = map
        .query(move |m| {
            let at = |m: &mut Map, entry| {
                let (_, bag, slot) = give(m, g, entry);
                (bag, slot)
            };
            (at(m, 117), at(m, 4540), at(m, 159))
        })
        .await
        .unwrap();
    let ate = eat(&mut c, jerky, 433).await;
    assert!(ate.0, "the Jerky eaten: {ate:?}");
    assert!(ate.1.iter().all(|(_, ms)| *ms <= 1_000), "{ate:?}");
    assert_eq!(ate.2, 0, "no SMSG_ITEM_COOLDOWN");
    let held = map.query(move |m| left(m, g, 433, 11)).await.unwrap();
    assert_eq!(held.0, None, "no wait of Food's own");
    assert!(held.1.is_some_and(|ms| ms <= 1_000), "a second: {held:?}");
    assert!(
        !eat(&mut c, bread, 433).await.0,
        "the Bread waits the second"
    );
    let drank = eat(&mut c, water, 430).await;
    assert!(drank.0, "drinking after eating: {drank:?}");
    assert_eq!(drank.2, 0, "no SMSG_ITEM_COOLDOWN");
    map.query(move |m| {
        let c = m.caster_mut(g).unwrap();
        let back = std::time::Duration::from_millis(1_100);
        for t in c
            .cooldowns
            .values_mut()
            .chain(c.category_cooldowns.values_mut())
        {
            *t = t.checked_sub(back).unwrap_or(*t);
        }
    })
    .await
    .unwrap();
    let again = eat(&mut c, bread, 433).await;
    assert!(again.0, "the Bread after a second: {again:?}");
}
