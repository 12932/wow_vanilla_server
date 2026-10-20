//! Round 9's player: locks, nodes and their loot windows (Thistle Tea's
//! `containers_test`, `gathering_test`, `game_objects_test`,
//! `fishing_maps_test` and `looting_test`).

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::crafts::{LOCK_KEY_ITEM, LOCK_KEY_SKILL, LockKey};
use crate::game::data::items::ItemSpell;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::spells::Targets;
use crate::game::map::{Map, MapHandle, MapKey, loot, professions};

const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

const CHEST: u32 = 180_950;
const GATE: u32 = 180_951;
const KEYED: u32 = 180_952;
const ORE: u32 = 9450;
const KEY: u32 = 9451;
const WRONG_KEY: u32 = 9452;
const BOX: u32 = 9453;
const COIN: u32 = 9454;
const BRACER: u32 = 9455;
const DUST: u32 = 9456;
const MARK: u32 = 3450;

/// An opening of the fishing lock's type (19: never missed), instant, and
/// one of a second's cast.
const OPEN: u32 = 900_960;
const SLOW_OPEN: u32 = 900_961;
/// An opening of a lock's wrong type.
const WRONG_KIND: u32 = 900_962;
/// A key's spell, and Pick Lock's (a second's cast).
const KEY_SPELL: u32 = 900_963;
const PICK_LOCK: u32 = 900_964;
const LINE: u32 = 900_965;
const DISENCHANT: u32 = 900_966;
const PICK_POCKET: u32 = 900_967;
const STEALTH: u32 = 900_968;
const LOCKPICKING: u32 = 633;
const ENCHANTING: u32 = 333;
const ONE_SECOND: u32 = 7;

const BAD_TARGETS: u8 = 0x0a;
const CHEST_IN_USE: u8 = 0x15;
const ITEM_GONE: u8 = 0x26;
const LOW_CASTLEVEL: u8 = 0x2c;
const OUT_OF_RANGE: u8 = 0x59;
const ONLY_INDOORS: u8 = 0x52;
const SMSG_CAST_RESULT: u16 = 0x130;
const SMSG_LOOT_RESPONSE: u16 = 0x160;

fn item(entry: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        name: format!("Item {entry}"),
        display_id: 1000,
        stackable: 20,
        allowable_class: -1,
        allowable_race: -1,
        ..Default::default()
    }
}

fn spell(id: u32, effects: &[(u32, u32, i32, i32)]) -> SpellTemplate {
    let mut s = SpellTemplate {
        id,
        name: format!("Spell {id}"),
        equipped_item_class: -1,
        range_index: 1,
        ..Default::default()
    };
    for (i, (effect, aura, misc, points)) in effects.iter().enumerate() {
        s.effects[i] = SpellEffect {
            effect: *effect,
            aura: *aura,
            misc_value: *misc,
            base_points: *points - 1,
            base_dice: 1,
            target_a: if *aura != 0 { 1 } else { 25 },
            ..Default::default()
        };
    }
    s
}

fn data() -> GameData {
    let mut d = GameData::empty();
    let key = |kind, index, skill| LockKey { kind, index, skill };
    d.crafts.locks.insert(47, vec![key(LOCK_KEY_SKILL, 19, 0)]);
    d.crafts.locks.insert(48, vec![key(LOCK_KEY_ITEM, KEY, 0)]);
    // A lockbox picked from 50.
    d.crafts.locks.insert(49, vec![key(LOCK_KEY_SKILL, 1, 50)]);
    let object = |entry, kind, data: [u32; 24]| GameObjectTemplate {
        entry,
        kind,
        display_id: 1,
        name: format!("Object {entry}"),
        size: 1.0,
        data,
        ..Default::default()
    };
    let mut chest = [0u32; 24];
    (chest[0], chest[1]) = (47, 60);
    let mut keyed = [0u32; 24];
    (keyed[0], keyed[1]) = (48, 60);
    // A gate behind the lock, shut again five seconds after it opens.
    let mut gate = [0u32; 24];
    (gate[1], gate[2]) = (47, 5 << 16);
    for (entry, kind, data) in [(CHEST, 3, chest), (GATE, 0, gate), (KEYED, 3, keyed)] {
        d.gameobject_templates
            .insert(entry, object(entry, kind, data));
    }
    let spawn = |guid, entry, dx: f32, dy: f32| GameObjectSpawn {
        guid,
        entry,
        map: 0,
        x: START.0 + dx,
        y: START.1 + dy,
        z: START.2,
        state: 1,
        respawn_min: 60,
        ..Default::default()
    };
    d.gameobject_spawns.insert(
        0,
        vec![
            spawn(800, CHEST, 1.0, 0.5),
            spawn(801, GATE, 1.0, -0.5),
            spawn(802, KEYED, 1.0, 1.5),
            spawn(803, CHEST, 20.0, 0.0),
        ],
    );
    for e in [ORE, COIN, DUST] {
        d.items.insert(e, item(e));
    }
    let key_item = |entry| ItemTemplate {
        stackable: 1,
        spells: [
            ItemSpell {
                id: KEY_SPELL,
                charges: -1,
                ..Default::default()
            },
            ItemSpell::default(),
            ItemSpell::default(),
            ItemSpell::default(),
            ItemSpell::default(),
        ],
        ..item(entry)
    };
    d.items.insert(KEY, key_item(KEY));
    d.items.insert(WRONG_KEY, key_item(WRONG_KEY));
    // A lockbox its key opens.
    d.items.insert(
        BOX,
        ItemTemplate {
            stackable: 1,
            lock_id: 48,
            ..item(BOX)
        },
    );
    let mut bracer = item(BRACER);
    bracer.class = 4;
    bracer.quality = 2;
    bracer.item_level = 20;
    bracer.stackable = 1;
    bracer.disenchant_id = 30;
    d.items.insert(BRACER, bracer);
    let drop = |item, count| LootEntry {
        item,
        chance: 100.0,
        min: count as i32,
        max: count,
        ..Default::default()
    };
    d.loot.gameobject.insert(60, vec![drop(ORE, 2)]);
    d.loot.item.insert(BOX, vec![drop(COIN, 1)]);
    d.loot.disenchant.insert(30, vec![drop(DUST, 2)]);
    d.loot.pickpocket.insert(20, vec![drop(COIN, 1)]);
    d.creature_templates.insert(
        MARK,
        CreatureTemplate {
            entry: MARK,
            name: "Mark".into(),
            display_ids: [903, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            display_scales: [1.0, 0.0, 0.0, 0.0],
            level_min: 10,
            level_max: 10,
            faction: 32,
            unit_class: 1,
            health_max: 100,
            pickpocket_loot_id: 20,
            ..Default::default()
        },
    );
    d.spells.ranges.insert(1, (0.0, 30.0));
    d.spells.cast_times.insert(ONE_SECOND, 1000);
    let mut slow = spell(SLOW_OPEN, &[(professions::EFFECT_OPEN_LOCK, 0, 19, 0)]);
    slow.casting_time_index = ONE_SECOND;
    let mut pick = spell(PICK_LOCK, &[(professions::EFFECT_OPEN_LOCK, 0, 1, 0)]);
    pick.casting_time_index = ONE_SECOND;
    let mut line = spell(LINE, &[(50, 0, 35_591, 0)]);
    line.effects[0].target_a = 39;
    line.attributes[1] = 0x4;
    line.attributes[0] |= crate::game::data::spells::ATTR_ONLY_INDOORS;
    for s in [
        spell(OPEN, &[(professions::EFFECT_OPEN_LOCK, 0, 19, 0)]),
        slow,
        spell(WRONG_KIND, &[(professions::EFFECT_OPEN_LOCK, 0, 2, 0)]),
        spell(KEY_SPELL, &[(professions::EFFECT_OPEN_LOCK, 0, 1, 0)]),
        pick,
        line,
        spell(DISENCHANT, &[(professions::EFFECT_DISENCHANT, 0, 0, 0)]),
        spell(PICK_POCKET, &[(professions::EFFECT_PICKPOCKET, 0, 0, 0)]),
        spell(
            STEALTH,
            &[(6, crate::game::map::auras::AURA_MOD_STEALTH, 0, 1)],
        ),
    ] {
        d.spells.spells.insert(s.id, s);
    }
    d.gameobject_templates.insert(
        35_591,
        object(35_591, professions::GO_FISHING_NODE, [0; 24]),
    );
    d
}

fn chest() -> Guid {
    guid::game_object(CHEST, 800)
}

fn far_chest() -> Guid {
    guid::game_object(CHEST, 803)
}

fn gate() -> Guid {
    guid::game_object(GATE, 801)
}

fn keyed() -> Guid {
    guid::game_object(KEYED, 802)
}

async fn setup(realm: &Arc<Realm>, account: &str, name: &str) -> (Client, Guid, MapHandle) {
    let mut a = Client::connect(realm, account);
    let g = a.enter(name, Race::Human, Class::Rogue).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    a.sees(chest()).await;
    (a, g, map)
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

fn object(o: Guid) -> Targets {
    Targets {
        object: Some(o),
        ..Default::default()
    }
}

fn item_target(i: Guid) -> Targets {
    Targets {
        item: Some(i),
        ..Default::default()
    }
}

/// Why `caster` can't cast `spell` at `t` now, by the cast's own checks.
fn why(m: &mut Map, caster: Guid, spell: u32, t: &Targets, item: Option<Guid>) -> Option<u8> {
    let s = m.realm.data.spells.get(spell).cloned().unwrap();
    let mods = crate::game::map::spells::mods::for_spell(m, caster, &s);
    crate::game::map::spells::checks::check(m, caster, &s, t, item, false, &mods)
}

/// A fresh cast: no global cooldown or cast left over.
fn cast(m: &mut Map, g: Guid, spell: u32, t: Targets, item: Option<Guid>) {
    if let Some(p) = m.players.get_mut(&g) {
        p.caster.gcd_until = None;
        p.caster.casting = None;
    }
    crate::game::map::spells::cast(m, g, spell, t, item);
}

/// The cast under way ends at the map's next update.
fn hurry(m: &mut Map, g: Guid) {
    let now = m.now;
    if let Some(c) = m.caster_mut(g).and_then(|c| c.casting.as_mut()) {
        c.ends = now;
    }
}

fn learn_skill(m: &mut Map, g: Guid, skill: u32, value: u32) {
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::skills::learn_skill(p, skill, 1);
    let i = (0..128u16)
        .find(|i| p.fields.get_u32(PLAYER_SKILL_INFO_1_1 + i * 3) & 0xFFFF == skill)
        .unwrap();
    p.fields
        .set_u32(PLAYER_SKILL_INFO_1_1 + i * 3 + 1, (300 << 16) | value);
}

fn give(m: &mut Map, g: Guid, entry: u32) -> Guid {
    crate::game::map::inventory::add_item(m, g, entry, 1, false).unwrap();
    let p = &m.players[&g];
    p.items
        .iter()
        .rev()
        .find(|i| i.entry == entry)
        .unwrap()
        .guid
}

fn moved(m: &mut Map, g: Guid, dx: f32) {
    let p = m.players.get_mut(&g).unwrap();
    p.movement.position.x += dx;
}

/// The cast result `a` is told for `spell`: its status and reason.
async fn cast_result(a: &mut Client, spell: u32) -> (u8, Option<u8>) {
    let f = a
        .wait_frame("SMSG_CAST_RESULT", SMSG_CAST_RESULT, |f| {
            u32::from_le_bytes(f[0..4].try_into().unwrap()) == spell
        })
        .await;
    (f[4], f.get(5).copied())
}

/// Whether `a` was told of `spell` going off, or of a loot window, in what
/// it has received.
fn went_off(a: &mut Client, spell: u32) -> bool {
    a.captured().iter().any(|f| {
        matches!(
            ServerOpcodeMessage::read_unencrypted(&f[..]),
            Ok(ServerOpcodeMessage::SMSG_SPELL_GO(g)) if g.spell == spell
        )
    })
}

/// Thistle Tea's "accepts a visible nearby object and rejects missing,
/// distant and foreign-world targets" and "feeds the same lock rules into
/// pre-cast validation": a node here and near opens; one 20 yards off is
/// out of range; one not in this world (gone, or another copy's: each copy
/// its own map) is no target; an opening of the wrong lock type is refused
/// before it is cast.
#[tokio::test]
async fn a_lock_spell_wants_its_object_here_near_and_of_its_kind() {
    let realm = harness::realm_with(data());
    let (mut a, g, map) = setup(&realm, "A", "Alice").await;
    let checks = on(&map, move |m| {
        let elsewhere = guid::game_object(CHEST, 9_999);
        (
            why(m, g, OPEN, &object(chest()), None),
            why(m, g, OPEN, &object(far_chest()), None),
            why(m, g, OPEN, &object(elsewhere), None),
            why(m, g, WRONG_KIND, &object(chest()), None),
        )
    })
    .await;
    assert_eq!(
        checks,
        (
            None,
            Some(OUT_OF_RANGE),
            Some(BAD_TARGETS),
            Some(BAD_TARGETS)
        )
    );
    a.clear();
    on(&map, move |m| cast(m, g, OPEN, object(chest()), None)).await;
    a.wait_frame("SMSG_LOOT_RESPONSE", SMSG_LOOT_RESPONSE, |f| {
        u64::from_le_bytes(f[0..8].try_into().unwrap()) == chest().guid()
    })
    .await;
}

/// Thistle Tea's "rejects a live object that leaves spell range before
/// activation" and "rejected completion sends full failure without
/// success or quest credit": a second's opening whose node is out of reach
/// as it ends, or opened by another meanwhile, fails then, told as a
/// failure alone (no success, no spell going off, no window).
#[tokio::test]
async fn an_opening_fails_as_it_ends_if_its_node_left_its_reach_or_was_taken() {
    let realm = harness::realm_with(data());
    let (mut a, g, map) = setup(&realm, "A", "Alice").await;
    let (_b, gb, _) = setup(&realm, "B", "Bob").await;
    a.clear();
    on(&map, move |m| {
        cast(m, g, SLOW_OPEN, object(chest()), None);
        moved(m, g, 20.0);
        hurry(m, g);
    })
    .await;
    assert_eq!(
        cast_result(&mut a, SLOW_OPEN).await,
        (2, Some(OUT_OF_RANGE))
    );
    on(&map, move |m| moved(m, g, -20.0)).await;
    a.clear();
    on(&map, move |m| {
        cast(m, g, SLOW_OPEN, object(chest()), None);
        // Bob opens it while Alice's cast is under way.
        cast(m, gb, OPEN, object(chest()), None);
        assert_eq!(m.players[&gb].looting, Some(chest()));
        hurry(m, g);
    })
    .await;
    assert_eq!(
        cast_result(&mut a, SLOW_OPEN).await,
        (2, Some(CHEST_IN_USE))
    );
    harness::an_update(&map).await;
    assert!(!went_off(&mut a, SLOW_OPEN), "no spell went off");
    let succeeded = a.captured().iter().any(|f| {
        u16::from_le_bytes([f[2], f[3]]) == SMSG_CAST_RESULT
            && u32::from_le_bytes(f[4..8].try_into().unwrap()) == SLOW_OPEN
            && f[8] == 0
    });
    assert!(!succeeded, "never told it succeeded");
    let looting = on(&map, move |m| m.players[&g].looting).await;
    assert_eq!(looting, None);
}

/// Thistle Tea's "instant key spells consume their charge only when
/// unlocking succeeds" and "item-based opening consumes a charge only
/// after success and never grants skill": a key opens its lockbox and its
/// chest, used up each time, and teaches no lockpicking; a key the lock
/// isn't for is refused before it is cast, and kept.
#[tokio::test]
async fn a_key_opens_its_lock_used_up_and_a_wrong_one_is_kept() {
    let realm = harness::realm_with(data());
    let (mut a, g, map) = setup(&realm, "A", "Alice").await;
    let (wrong, box_item) = on(&map, move |m| (give(m, g, WRONG_KEY), give(m, g, BOX))).await;
    let refused = on(&map, move |m| {
        (
            why(m, g, KEY_SPELL, &item_target(box_item), Some(wrong)),
            why(m, g, KEY_SPELL, &object(keyed()), Some(wrong)),
        )
    })
    .await;
    assert_eq!(refused, (Some(BAD_TARGETS), Some(BAD_TARGETS)));
    let key = on(&map, move |m| give(m, g, KEY)).await;
    a.clear();
    let at = on(&map, move |m| {
        let p = &m.players[&g];
        let i = p.item_by_guid(key).unwrap();
        (p.items[i].bag, p.items[i].slot)
    })
    .await;
    a.send(CMSG_USE_ITEM {
        bag_index: at.0,
        bag_slot: at.1,
        spell_index: 0,
        targets: SpellCastTargets {
            target_flags: SpellCastTargets_SpellCastTargetFlags::empty()
                .set_item(SpellCastTargets_SpellCastTargetFlags_Item::Item { item: box_item }),
        },
    })
    .await;
    a.wait_frame("the box's SMSG_LOOT_RESPONSE", SMSG_LOOT_RESPONSE, |f| {
        u64::from_le_bytes(f[0..8].try_into().unwrap()) == box_item.guid()
    })
    .await;
    let after = on(&map, move |m| {
        let p = &m.players[&g];
        let unlocked = p
            .item_by_guid(box_item)
            .is_some_and(|i| p.items[i].fields.get_u32(ITEM_FIELD_FLAGS) & 0x4 != 0);
        (
            p.count_of(KEY),
            p.count_of(WRONG_KEY),
            unlocked,
            crate::game::map::skills::value(p, LOCKPICKING),
        )
    })
    .await;
    assert_eq!(
        after,
        (0, 1, true, None),
        "the key used up, nothing learned"
    );
    // Another key at the keyed chest: opened, used up.
    let key = on(&map, move |m| {
        loot::close(m, g, box_item);
        give(m, g, KEY)
    })
    .await;
    a.clear();
    on(&map, move |m| {
        cast(m, g, KEY_SPELL, object(keyed()), Some(key))
    })
    .await;
    a.wait_frame("the chest's SMSG_LOOT_RESPONSE", SMSG_LOOT_RESPONSE, |f| {
        u64::from_le_bytes(f[0..8].try_into().unwrap()) == keyed().guid()
    })
    .await;
    let left = on(&map, move |m| {
        let p = &m.players[&g];
        (
            p.count_of(KEY),
            crate::game::map::skills::value(p, LOCKPICKING),
        )
    })
    .await;
    assert_eq!(left, (0, None));
}

/// Thistle Tea's "completion rejects changed skill, removed targets and
/// death": a lockbox's pick, under way, opens nothing if the box is gone,
/// the skill fallen short, or the picker dead as it ends.
#[tokio::test]
async fn a_pick_opens_nothing_if_its_box_skill_or_picker_is_gone_as_it_ends() {
    let mut d = data();
    // This box wants Pick Lock at 50.
    d.items.get_mut(&BOX).unwrap().lock_id = 49;
    let realm = harness::realm_with(d);
    let (mut a, g, map) = setup(&realm, "A", "Alice").await;
    let box_item = on(&map, move |m| {
        learn_skill(m, g, LOCKPICKING, 100);
        give(m, g, BOX)
    })
    .await;
    a.clear();
    on(&map, move |m| {
        cast(m, g, PICK_LOCK, item_target(box_item), None);
        crate::game::map::skills::set_value(m.players.get_mut(&g).unwrap(), LOCKPICKING, 10);
        hurry(m, g);
    })
    .await;
    assert_eq!(
        cast_result(&mut a, PICK_LOCK).await,
        (2, Some(LOW_CASTLEVEL))
    );
    a.clear();
    on(&map, move |m| {
        crate::game::map::skills::set_value(m.players.get_mut(&g).unwrap(), LOCKPICKING, 100);
        cast(m, g, PICK_LOCK, item_target(box_item), None);
        let i = m.players[&g].item_by_guid(box_item).unwrap();
        crate::game::map::inventory::take_item(m, g, i, 1);
        hurry(m, g);
    })
    .await;
    assert_eq!(cast_result(&mut a, PICK_LOCK).await, (2, Some(ITEM_GONE)));
    let box_item = on(&map, move |m| give(m, g, BOX)).await;
    on(&map, move |m| {
        cast(m, g, PICK_LOCK, item_target(box_item), None);
        crate::game::map::death::kill(m, g);
        hurry(m, g);
    })
    .await;
    harness::an_update(&map).await;
    harness::an_update(&map).await;
    let unlocked = on(&map, move |m| {
        let p = &m.players[&g];
        let i = p.item_by_guid(box_item).unwrap();
        (
            p.items[i].fields.get_u32(ITEM_FIELD_FLAGS) & 0x4 != 0,
            p.looting,
        )
    })
    .await;
    assert_eq!(unlocked, (false, None), "nothing picked by the dead");
}

/// Thistle Tea's "routes an open-lock completion through ordinary door
/// use": a gate opened by its lock is opened as one used by hand, and
/// shuts again after its time (mangos's `UseDoorOrButton`).
#[tokio::test]
async fn a_gate_opened_by_its_lock_shuts_again_after_its_time() {
    let realm = harness::realm_with(data());
    let (_a, g, map) = setup(&realm, "A", "Alice").await;
    let opened = on(&map, move |m| {
        cast(m, g, OPEN, object(gate()), None);
        (
            m.gameobject(gate())
                .map(|o| o.fields.get_u32(GAMEOBJECT_STATE)),
            m.objects
                .closes
                .iter()
                .find(|(_, o, _)| *o == gate())
                .map(|(at, _, back)| (at.duration_since(m.now), *back)),
        )
    })
    .await;
    assert_eq!(opened.0, Some(0), "open");
    let (shuts_in, back) = opened.1.expect("shut again after its time");
    assert!(shuts_in <= Duration::from_secs(5) && shuts_in > Duration::from_secs(4));
    assert_eq!(back, 1, "to how it spawned");
}

/// Thistle Tea's "does not create a bobber when launch requirements
/// fail": a line that may be cast only indoors, cast outdoors, is refused
/// with no bobber, no spot and no channel.
#[tokio::test]
async fn a_line_refused_outdoors_casts_no_bobber() {
    let realm = harness::realm_with(data());
    let (mut a, g, map) = setup(&realm, "A", "Alice").await;
    a.clear();
    let left = on(&map, move |m| {
        m.spells.outdoors_override.insert(g, true);
        cast(m, g, LINE, Targets::default(), None);
        (
            m.loot_state.bobbers.len(),
            m.loot_state.fishing_spots.contains_key(&g),
            m.caster(g).is_some_and(|c| c.casting.is_some()),
        )
    })
    .await;
    assert_eq!(left, (0, false, false));
    assert_eq!(cast_result(&mut a, LINE).await, (2, Some(ONLY_INDOORS)));
    let channel = a
        .gets(Duration::from_millis(100), |m| {
            matches!(m, ServerOpcodeMessage::MSG_CHANNEL_START(_)).then_some(())
        })
        .await;
    assert!(channel.is_none());
}

/// Thistle Tea's "closes pocket viewing before opening a corpse window":
/// a rogue looking in a mark's pockets as it dies is told the pockets have
/// shut before the corpse's window opens.
#[tokio::test]
async fn pockets_open_as_their_mark_dies_shut_before_its_corpse_opens() {
    let realm = harness::realm_with(data());
    let (mut a, g, map) = setup(&realm, "A", "Alice").await;
    let mark = on(&map, move |m| {
        let c = crate::game::tests_p3_combat::spawn(m, g, MARK, 10, 2.0);
        crate::game::map::spells::cast_triggered(m, g, STEALTH, Some(g));
        cast(
            m,
            g,
            PICK_POCKET,
            Targets {
                unit: Some(c),
                ..Default::default()
            },
            None,
        );
        c
    })
    .await;
    a.wait_frame("the pockets' SMSG_LOOT_RESPONSE", SMSG_LOOT_RESPONSE, |f| {
        u64::from_le_bytes(f[0..8].try_into().unwrap()) == mark.guid()
    })
    .await;
    a.clear();
    on(&map, move |m| {
        crate::game::map::auras::remove_spell(m, g, STEALTH, None);
        crate::game::map::combat::kill(m, g, mark);
    })
    .await;
    a.send(CMSG_LOOT { guid: mark }).await;
    a.wait_frame("the corpse's SMSG_LOOT_RESPONSE", SMSG_LOOT_RESPONSE, |f| {
        u64::from_le_bytes(f[0..8].try_into().unwrap()) == mark.guid()
    })
    .await;
    let frames = a.captured();
    let opcode = |f: &Arc<[u8]>| u16::from_le_bytes([f[2], f[3]]);
    let shut = frames
        .iter()
        .rposition(|f| opcode(f) == 0x161 && f[4..12] == mark.guid().to_le_bytes())
        .expect("the pockets shut");
    let corpse = frames
        .iter()
        .rposition(|f| opcode(f) == SMSG_LOOT_RESPONSE)
        .unwrap();
    assert!(shut < corpse);
}

/// Thistle Tea's "settles an earlier item-loot window before planning the
/// new inventory": a disenchant's window open as a chest is opened shuts
/// first (as the client is told), and no one is left looking in it; what
/// was left in it goes with it (mangos's release of an item's loot).
#[tokio::test]
async fn an_item_window_open_as_a_chest_opens_shuts_first() {
    let realm = harness::realm_with(data());
    let (mut a, g, map) = setup(&realm, "A", "Alice").await;
    let bracer = on(&map, move |m| {
        learn_skill(m, g, ENCHANTING, 100);
        let b = give(m, g, BRACER);
        cast(m, g, DISENCHANT, item_target(b), None);
        b
    })
    .await;
    a.wait_frame("the bracer's SMSG_LOOT_RESPONSE", SMSG_LOOT_RESPONSE, |f| {
        u64::from_le_bytes(f[0..8].try_into().unwrap()) == bracer.guid()
    })
    .await;
    a.clear();
    let after = on(&map, move |m| {
        cast(m, g, OPEN, object(chest()), None);
        (
            m.players[&g].looting,
            m.loot_state.item_loot.contains_key(&bracer),
        )
    })
    .await;
    assert_eq!(after, (Some(chest()), false));
    let frames = a.captured();
    let opcode = |f: &Arc<[u8]>| u16::from_le_bytes([f[2], f[3]]);
    let shut = frames
        .iter()
        .rposition(|f| opcode(f) == 0x161 && f[4..12] == bracer.guid().to_le_bytes())
        .expect("the bracer's window shut");
    let opened = frames
        .iter()
        .rposition(|f| opcode(f) == SMSG_LOOT_RESPONSE && f[4..12] == chest().guid().to_le_bytes())
        .unwrap();
    assert!(shut < opened);
}

/// Thistle Tea's "an abruptly terminated looter releases access and
/// retains unclaimed contents" and "movement and logout release node
/// access without discarding remaining loot": a looter whose client goes
/// lets go of the chest, its ore kept for the next to open it.
#[tokio::test]
async fn a_looter_gone_lets_go_of_the_chest_and_its_loot_stays() {
    let realm = harness::realm_with(data());
    let (mut a, g, map) = setup(&realm, "A", "Alice").await;
    on(&map, move |m| cast(m, g, OPEN, object(chest()), None)).await;
    a.wait_frame("SMSG_LOOT_RESPONSE", SMSG_LOOT_RESPONSE, |f| {
        u64::from_le_bytes(f[0..8].try_into().unwrap()) == chest().guid()
    })
    .await;
    a.disconnect();
    harness::offline(&realm, g).await;
    let held = on(&map, move |m| {
        let l = m.gameobject(chest()).and_then(|o| o.loot.clone()).unwrap();
        (
            l.looters.len(),
            l.opened_by.len(),
            l.items.iter().filter(|s| !s.taken).count(),
        )
    })
    .await;
    assert_eq!(held, (0, 0, 1), "let go, the ore kept");
    let (mut b, gb, _) = setup(&realm, "B", "Bob").await;
    b.clear();
    on(&map, move |m| cast(m, gb, OPEN, object(chest()), None)).await;
    b.wait_frame("SMSG_LOOT_RESPONSE", SMSG_LOOT_RESPONSE, |f| {
        u64::from_le_bytes(f[0..8].try_into().unwrap()) == chest().guid()
    })
    .await;
    b.send(CMSG_AUTOSTORE_LOOT_ITEM { item_slot: 0 }).await;
    b.wait_for("SMSG_ITEM_PUSH_RESULT", |m| match m {
        ServerOpcodeMessage::SMSG_ITEM_PUSH_RESULT(r) if r.item == ORE => Some(()),
        _ => None,
    })
    .await;
}
