//! Phase 3's fourth round of the world's entity, visibility and message
//! tests, ported from Thistle Tea's `test/game/world/entity`,
//! `test/game/world/visibility` and `test/game/network/message`
//! (docs/parity/matrix.csv): what a viewer sees, and how a cast's
//! refusal is laid out.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::guid;
use crate::game::map::Map;
use crate::game::map::death::{self, Corpse, Death};
use crate::game::map::spells::{self, Targets, fail};
use crate::game::tests_p3_combat::{BRUTE, data, setup_with, spawn};
use crate::game::tests_p3_entity::spell;
use crate::game::tests_p3_entity_objects::object;

pub(crate) const SMSG_CAST_RESULT: u16 = 0x130;

pub(crate) async fn on<T: Send + 'static>(
    map: &crate::game::map::MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map answers")
}

/// A spell Thistle Tea's test knows nothing of.
const UNKNOWN: u32 = 123;
/// Shoot, of a bow, gun or crossbow.
const SHOOT: u32 = 98_000;

/// Thistle Tea's "encodes ID-only failures with unknown requirements" and
/// "encodes disarmed weapon failures without item-class payloads": a
/// refusal for an area, a focus or gear of a spell the server doesn't know
/// still carries its payload (no area, no focus, any class), and a
/// disarmed weapon's (`SPELL_FAILED_EQUIPPED_ITEM`) none. A shot with no
/// ranged weapon names the gear Shoot wants too: it went without it, and
/// the client read past the packet's end.
#[tokio::test]
async fn a_refusals_payload_is_there_whatever_the_spell() {
    let mut d = data();
    let mut shoot = spell(SHOOT, &[]);
    shoot.equipped_item_class = 2;
    shoot.equipped_item_subclass_mask = 0x4_000C;
    d.spells.spells.insert(SHOOT, shoot);
    let (_realm, mut a, g, map) = setup_with(d, Class::Warrior).await;
    let body = |reason: u8, payload: &[i32]| {
        let mut b = UNKNOWN.to_le_bytes().to_vec();
        b.extend([2, reason]);
        for p in payload {
            b.extend(p.to_le_bytes());
        }
        b
    };
    for (reason, payload) in [
        (fail::REQUIRES_AREA, vec![0]),
        (fail::REQUIRES_SPELL_FOCUS, vec![0]),
        (fail::EQUIPPED_ITEM_CLASS, vec![-1, 0, 0]),
        (fail::EQUIPPED_ITEM, vec![]),
    ] {
        a.clear();
        on(&map, move |m| {
            spells::send_cast_result(m, g, UNKNOWN, Some(reason))
        })
        .await;
        let got = a
            .wait_frame("the refusal", SMSG_CAST_RESULT, |_| true)
            .await;
        assert_eq!(got, body(reason, &payload), "reason {reason:#x}");
    }
    a.clear();
    let brute = on(&map, move |m| spawn(m, g, BRUTE, 20, 5.0)).await;
    on(&map, move |m| {
        crate::game::map::ranged::start(
            m,
            g,
            SHOOT,
            Targets {
                unit: Some(brute),
                ..Default::default()
            },
        )
    })
    .await;
    let got = a
        .wait_frame("the shot's refusal", SMSG_CAST_RESULT, |_| true)
        .await;
    let mut want = SHOOT.to_le_bytes().to_vec();
    want.extend([2, fail::EQUIPPED_ITEM_CLASS]);
    want.extend(2i32.to_le_bytes());
    want.extend(0x4_000Ci32.to_le_bytes());
    want.extend(0i32.to_le_bytes());
    assert_eq!(got, want, "a ranged weapon, of Shoot's kinds");
}

const SPIRIT_HEALER: u32 = 98_010;
const TRAP_OBJECT: u32 = 98_011;

fn sight_world() -> GameData {
    let mut d = data();
    let mut healer = crate::game::tests_p3_combat::template(SPIRIT_HEALER, 7);
    healer.npc_flags = crate::game::map::npc::NPC_FLAG_SPIRITHEALER;
    healer.faction = crate::game::tests_p3_combat::HUMANS;
    d.creature_templates.insert(SPIRIT_HEALER, healer);
    d.gameobject_templates
        .insert(TRAP_OBJECT, object(TRAP_OBJECT, 5, "A Crate", &[]));
    d
}

fn ghost(m: &mut Map, g: Guid) {
    let now = m.now;
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).expect("the player");
    let corpse = Corpse::new(&data, p, p.position(), 0.0);
    death::become_ghost(p, now);
    m.add_corpse(corpse);
}

/// Thistle Tea's `Visibility.Filter` for a living viewer and a ghost
/// ("does not see ghost players", "sees all players", "does not see dead
/// mobs near own corpse", "sees corpses and game objects", "nobody sees a
/// concealed creature, ghost or not"), as mangos-classic's
/// `isVisibleForInState` and `IsVisibleInGridForPlayer`: the living don't
/// see a ghost, a ghost sees every player, a corpse and an object, and of
/// the creatures by its corpse only the living; a creature a script
/// conceals no one sees, a spirit healer as little.
#[tokio::test]
async fn who_sees_whom_alive_and_as_a_ghost() {
    let (realm, _a, ga, map) = setup_with(sight_world(), Class::Warrior).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    let gb = b.enter("Bea", Race::Human, Class::Warrior).await;
    let out = on(&map, move |m| {
        let near = spawn(m, ga, BRUTE, 20, 10.0);
        let dead = spawn(m, ga, BRUTE, 20, 12.0);
        crate::game::map::combat::deal_damage(m, ga, dead, 1_000_000);
        let hidden = spawn(m, ga, BRUTE, 20, 8.0);
        let healer = spawn(m, ga, SPIRIT_HEALER, 20, 6.0);
        m.creature_mut(hidden)
            .unwrap()
            .scripted
            .get_or_insert_with(Default::default)
            .concealed = true;
        let at = m.players[&ga].position();
        let crate_ =
            crate::game::map::objects::summon(m, None, TRAP_OBJECT, at, 0.0, 0, 0, 0, None)
                .expect("the crate");
        let mut seen = Vec::new();
        let mut look = |m: &Map, what: &str| {
            seen.push((
                what.to_owned(),
                [near, dead, hidden, healer, gb, crate_].map(|o| death::can_see(m, ga, o)),
                death::can_see(m, gb, ga),
            ));
        };
        look(m, "alive");
        if let Some(p) = m.players.get_mut(&gb) {
            p.death = Death::Ghost { since: m.now };
        }
        look(m, "Bea a ghost");
        ghost(m, ga);
        let corpse = guid::corpse(guid::counter(ga));
        look(m, "both ghosts");
        let corpse_seen = (death::can_see(m, ga, corpse), death::can_see(m, gb, corpse));
        (seen, corpse_seen)
    })
    .await;
    let (seen, corpse_seen) = out;
    // near, dead, hidden, healer, Bea, the crate; and whether Bea sees Alice.
    assert_eq!(
        seen[0].1,
        [true, true, false, false, true, true],
        "{}",
        seen[0].0
    );
    assert!(seen[0].2);
    assert_eq!(
        seen[1].1,
        [true, true, false, false, false, true],
        "the living don't see a ghost"
    );
    assert!(seen[1].2, "a ghost sees the living");
    assert_eq!(
        seen[2].1,
        [true, false, false, true, true, true],
        "a ghost sees a ghost, a crate, the living by its corpse and a spirit healer; not the dead, nor the concealed"
    );
    assert!(seen[2].2);
    assert_eq!(
        corpse_seen,
        (true, true),
        "a corpse, by its ghost and another"
    );
}

const TOTEM: u32 = 98_020;
const TOTEM_SPELL: u32 = 98_021;
const BLIZZARD: u32 = 98_022;

/// Thistle Tea's "totems play their despawn animation as they leave" and
/// "dynamic objects play their despawn animation as they go", as
/// mangos-classic's `Totem::UnSummon` and `DynamicObject::Delete` send
/// `SMSG_GAMEOBJECT_DESPAWN_ANIM`: those who see a totem or a spell's
/// area watch it go, before they forget it.
#[tokio::test]
async fn a_totem_and_a_spells_area_fade_as_they_go() {
    let mut d = data();
    let mut totem = crate::game::tests_p3_combat::template(TOTEM, 11);
    totem.faction = crate::game::tests_p3_combat::HUMANS;
    d.creature_templates.insert(TOTEM, totem);
    d.spells.spells.insert(
        TOTEM_SPELL,
        spell(
            TOTEM_SPELL,
            &[crate::game::data::spells::SpellEffect {
                effect: crate::game::map::pets::EFFECT_SUMMON_TOTEM_SLOT1,
                misc_value: TOTEM as i32,
                base_points: 4,
                base_dice: 1,
                target_a: 1,
                ..Default::default()
            }],
        ),
    );
    d.spells.spells.insert(BLIZZARD, spell(BLIZZARD, &[]));
    let (realm, mut a, ga, map) = setup_with(d, Class::Warrior).await;
    let mut b = crate::game::harness::Client::connect(&realm, "B");
    b.enter("Bea", Race::Human, Class::Warrior).await;
    let totem = on(&map, move |m| {
        spells::cast_triggered(m, ga, TOTEM_SPELL, Some(ga));
        m.players[&ga].owned.totems[0].expect("the totem")
    })
    .await;
    let area = on(&map, move |m| {
        let at = m.players[&ga].position();
        crate::game::map::spells::dynamic::spawn(
            m,
            ga,
            BLIZZARD,
            crate::game::map::spells::dynamic::KIND_AREA_SPELL,
            at,
            8.0,
            None,
        )
    })
    .await;
    for c in [&mut a, &mut b] {
        c.sees(totem).await;
        c.sees(area).await;
    }
    let fades = |of: Guid| {
        move |m: &wow_world_messages::vanilla::opcodes::ServerOpcodeMessage| {
            match m {
            wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_GAMEOBJECT_DESPAWN_ANIM(
                d,
            ) if d.guid == of => Some(()),
            _ => None,
        }
        }
    };
    on(&map, move |m| {
        crate::game::map::pets::dismiss(m, totem);
        crate::game::map::spells::dynamic::remove(m, area);
    })
    .await;
    for c in [&mut a, &mut b] {
        c.wait_for("the totem's fade", fades(totem)).await;
        c.wait_for("the area's fade", fades(area)).await;
        c.forgets(totem).await;
        c.forgets(area).await;
    }
}
