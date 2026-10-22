//! A ranged attack's damage waits for its arrow: the go goes as the shot
//! is loosed, and the client draws the arrow from it; the damage log and
//! the target's health change come when it lands, the distance over the
//! spell's speed later (mangos-classic's `Spell::prepare`/`cast`:
//! `m_delayMoment` = distance / speed, at least five yards' worth, the
//! effects at `DoAllEffectOnTarget` after it). Auto Shot at 40 yards a
//! second, a wand's Shoot at 20, and a shot cast as a spell (Arcane
//! Shot's kind); the next auto shot's timer as it was.

use std::time::{Duration, Instant};

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_pets::cast;

/// A target of a lot of health that stands where it is (and hurts little).
const TARGET: u32 = 3_710;
const BOW: u32 = 50_103;
const ARROW: u32 = 50_104;
const WAND: u32 = 50_105;
const AUTO_SHOT: u32 = 75;
const SHOOT: u32 = 5019;
/// A shot cast as a spell: Arcane Shot's shape.
const ARCANE_SHOT: u32 = 95_710;
/// The map's update period: a shot lands at the first update past its
/// time.
const TICK: Duration = Duration::from_millis(50);

fn item(entry: u32, class: u32, subclass: u32, inventory_type: u32) -> ItemTemplate {
    ItemTemplate {
        entry,
        class,
        subclass,
        name: format!("Item {entry}"),
        display_id: 1000,
        inventory_type,
        stackable: 1,
        ..Default::default()
    }
}

fn world() -> GameData {
    use crate::game::tests_p3_combat::{data, template};
    let mut d = data();
    d.class_level_stats.insert(
        (1, 1),
        ClassLevelStats {
            health: 100_000,
            melee_damage: 1.0,
            ..Default::default()
        },
    );
    d.creature_templates.insert(TARGET, template(TARGET, 1));
    let mut bow = item(BOW, 2, 2, 15);
    bow.delay = 2000;
    bow.damages[0] = (10.0, 12.0, 0);
    bow.ammo_type = 2;
    d.items.insert(BOW, bow);
    let mut arrow = item(ARROW, 6, 2, 24);
    arrow.stackable = 200;
    arrow.damages[0] = (2.0, 2.0, 0);
    d.items.insert(ARROW, arrow);
    let mut wand = item(WAND, 2, 19, 26);
    wand.delay = 1500;
    wand.damages[0] = (10.0, 12.0, 6);
    d.items.insert(WAND, wand);
    // From five yards (no dead zone, to see the five-yard floor) to 35.
    d.spells.ranges.insert(114, (0.0, 35.0));
    let weapon = SpellEffect {
        effect: 58,
        base_points: -1,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    let mut auto = SpellTemplate {
        id: AUTO_SHOT,
        name: "Auto Shot".into(),
        range_index: 114,
        speed: 40.0,
        dmg_class: 3,
        equipped_item_class: 2,
        ..Default::default()
    };
    auto.attributes[0] = 0x2 | 0x10;
    auto.attributes[2] = 0x20;
    auto.effects[0] = weapon;
    let mut shoot = auto.clone();
    shoot.id = SHOOT;
    shoot.name = "Shoot".into();
    shoot.speed = 20.0;
    shoot.dmg_class = 1;
    shoot.equipped_item_subclass_mask = 1 << 19;
    let mut arcane = auto.clone();
    arcane.id = ARCANE_SHOT;
    arcane.name = "Arcane Shot".into();
    arcane.attributes[2] = 0;
    arcane.school = 6;
    arcane.effects[0] = SpellEffect {
        effect: 2,
        base_points: 19,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    for s in [auto, shoot, arcane] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

/// Put an item of `entry` in the equipment slot `slot`.
fn equip(m: &mut Map, g: Guid, entry: u32, slot: u8) {
    if let Some(p) = m.players.get_mut(&g) {
        p.items
            .retain(|i| !(i.bag == crate::game::store::BAG_PLAYER && i.slot == slot));
    }
    crate::game::map::inventory::add_item(m, g, entry, 1, false).expect("room for it");
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).expect("the player");
    let i = p
        .items
        .iter()
        .rposition(|i| i.entry == entry)
        .expect("the item");
    p.items[i].bag = crate::game::store::BAG_PLAYER;
    p.items[i].slot = slot;
    p.refresh_inventory(&data);
}

/// Alice with `weapon` (and arrows), shots that travel, and a target
/// `yards` before her.
async fn ready(
    weapon: u32,
    yards: f32,
) -> (
    std::sync::Arc<crate::game::Realm>,
    Client,
    Guid,
    MapHandle,
    Guid,
) {
    let (realm, mut a, ga, map) =
        crate::game::tests_p3_combat::setup_with(world(), Class::Warrior).await;
    let target = map
        .query(move |m| {
            m.spells.travel = Some(true);
            equip(m, ga, weapon, 17);
            crate::game::map::inventory::add_item(m, ga, ARROW, 50, false).expect("room");
            crate::game::map::ranged::set_ammo(m, ga, ARROW);
            if let Some(p) = m.players.get_mut(&ga) {
                p.spells.extend([AUTO_SHOT, SHOOT, ARCANE_SHOT]);
                p.fields.set_u32(UNIT_FIELD_MAXPOWER1, 1000);
                p.fields.set_u32(UNIT_FIELD_POWER1, 1000);
                p.derived.ranged_hit = 100.0;
            }
            let g = crate::game::tests_p3_combat::spawn(m, ga, TARGET, 1, yards);
            // Its blows can't kill her (and end its fight) while she shoots.
            if let Some(c) = m.creature_mut(g) {
                c.fields.set_f32(UNIT_FIELD_MINDAMAGE, 0.0);
                c.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 0.0);
            }
            if let Some(s) = crate::game::map::event_ai::scripted_mut(m, g) {
                s.no_combat_movement = true;
            }
            g
        })
        .await
        .unwrap();
    a.sees(target).await;
    a.clear();
    (realm, a, ga, map, target)
}

fn health(m: &Map, g: Guid) -> u32 {
    m.creature(g)
        .map_or(0, |c| c.fields.get_u32(UNIT_FIELD_HEALTH))
}

/// What one shot of `spell` at `target` told its shooter, and when: the
/// go that hit (when it came, and the target's health and the shots in
/// flight then), then its damage (when, how much, and the health after).
struct Flight {
    go_at: Instant,
    full: u32,
    at_go: (u32, usize),
    damage_at: Instant,
    damage: u32,
    after: u32,
}

async fn one_shot(a: &mut Client, ga: Guid, map: &MapHandle, spell: u32, target: Guid) -> Flight {
    let full = map.query(move |m| health(m, target)).await.unwrap();
    // A shot that misses (one in many) is passed over for the next.
    a.wait_for("a go that hits", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(g)
            if g.caster == ga && g.spell == spell && g.hits.contains(&target) =>
        {
            Some(())
        }
        _ => None,
    })
    .await;
    let go_at = Instant::now();
    let at_go = map
        .query(move |m| {
            (
                health(m, target),
                m.spells.shots.len() + m.spells.in_flight.len(),
            )
        })
        .await
        .unwrap();
    let damage = a
        .wait_for("its damage", |m| match m {
            ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(d)
                if d.attacker == ga && d.spell == spell && d.target == target =>
            {
                Some(d.damage)
            }
            _ => None,
        })
        .await;
    let damage_at = Instant::now();
    let after = map.query(move |m| health(m, target)).await.unwrap();
    Flight {
        go_at,
        full,
        at_go,
        damage_at,
        damage,
        after,
    }
}

/// The damage came no sooner than `yards` at `speed` after the go (within
/// a tick), and not long after; the target's health untouched while the
/// shot flew, and down by the damage once it landed.
fn assert_flew(f: &Flight, yards: f32, speed: f32, what: &str) {
    let flight = Duration::from_secs_f32(yards.max(5.0) / speed);
    let took = f.damage_at - f.go_at;
    assert!(
        took + TICK >= flight,
        "{what}: the damage {took:?} after the go, before the arrow's {flight:?}"
    );
    assert!(
        took <= flight + Duration::from_millis(750),
        "{what}: the damage {took:?} after the go, long after the arrow's {flight:?}"
    );
    let (health_at_go, flying) = f.at_go;
    if flying > 0 {
        assert_eq!(health_at_go, f.full, "{what}: no damage while it flies");
    } else {
        // Seen only once it had landed: a five-yard flight is short.
        assert!(
            flight < Duration::from_millis(500),
            "{what}: seen in flight"
        );
    }
    assert!(f.damage > 0, "{what}: it hurt");
    assert_eq!(
        f.after,
        f.full.saturating_sub(f.damage),
        "{what}: the health by its damage"
    );
}

async fn auto_shot_at(yards: f32) {
    let (_realm, mut a, ga, map, target) = ready(BOW, yards).await;
    cast(&a, AUTO_SHOT, Some(target)).await;
    let f = one_shot(&mut a, ga, &map, AUTO_SHOT, target).await;
    assert_flew(&f, yards, 40.0, &format!("Auto Shot at {yards} yards"));
}

#[tokio::test]
async fn an_auto_shot_at_five_yards_lands_after_its_flight() {
    auto_shot_at(5.0).await;
}

#[tokio::test]
async fn an_auto_shot_at_twenty_yards_lands_after_its_flight() {
    auto_shot_at(20.0).await;
}

#[tokio::test]
async fn an_auto_shot_at_thirty_five_yards_lands_after_its_flight() {
    auto_shot_at(35.0).await;
}

/// A wand's Shoot at 20 yards: a second's flight at its 20 yards a second.
#[tokio::test]
async fn a_wands_shoot_lands_after_its_flight() {
    let (_realm, mut a, ga, map, target) = ready(WAND, 20.0).await;
    cast(&a, SHOOT, Some(target)).await;
    let f = one_shot(&mut a, ga, &map, SHOOT, target).await;
    assert_flew(&f, 20.0, 20.0, "Shoot at 20 yards");
}

/// A shot cast as a spell (Arcane Shot's kind) travels as a bolt does.
#[tokio::test]
async fn an_arcane_shot_lands_after_its_flight() {
    let (_realm, mut a, ga, map, target) = ready(BOW, 30.0).await;
    cast(&a, ARCANE_SHOT, Some(target)).await;
    let f = one_shot(&mut a, ga, &map, ARCANE_SHOT, target).await;
    assert_flew(&f, 30.0, 40.0, "Arcane Shot at 30 yards");
}

/// The flight delays the damage, not the next shot: Auto Shot's go comes
/// every bow's two seconds, whatever its arrows' flights (35 yards: 875
/// ms each).
#[tokio::test]
async fn the_next_auto_shot_waits_for_the_bow_not_the_arrow() {
    let (_realm, mut a, ga, map, target) = ready(BOW, 35.0).await;
    cast(&a, AUTO_SHOT, Some(target)).await;
    let mut gos = Vec::new();
    for _ in 0..2 {
        a.wait_for("a go", |m| match m {
            ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.caster == ga && g.spell == AUTO_SHOT => {
                Some(())
            }
            _ => None,
        })
        .await;
        gos.push(Instant::now());
    }
    let between = gos[1] - gos[0];
    assert!(
        between + TICK >= Duration::from_millis(2000) && between <= Duration::from_millis(2600),
        "the second shot {between:?} after the first"
    );
    // The first arrow landed in between.
    let hits = map
        .query(move |m| m.spells.shots.iter().filter(|s| s.shooter == ga).count())
        .await
        .unwrap();
    assert!(hits <= 1, "one arrow in the air at most: {hits}");
}

/// A shot whose target dies in flight is lost; one from a shooter gone
/// is too: neither is applied, and none is left in the air.
#[tokio::test]
async fn a_shot_at_a_target_dead_in_flight_is_lost() {
    let (_realm, mut a, ga, map, target) = ready(BOW, 35.0).await;
    cast(&a, AUTO_SHOT, Some(target)).await;
    a.wait_for("a go", |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.caster == ga && g.spell == AUTO_SHOT => Some(()),
        _ => None,
    })
    .await;
    let flying = map
        .query(move |m| {
            let flying = m.spells.shots.len();
            if flying > 0 {
                crate::game::map::combat::deal_damage(m, ga, target, 1_000_000);
            }
            flying
        })
        .await
        .unwrap();
    if flying == 0 {
        return;
    }
    let landed = a
        .gets(Duration::from_millis(1200), |m| match m {
            ServerOpcodeMessage::SMSG_SPELLNONMELEEDAMAGELOG(d)
                if d.attacker == ga && d.spell == AUTO_SHOT =>
            {
                Some(())
            }
            _ => None,
        })
        .await;
    assert!(landed.is_none(), "no damage for a dead target");
    let left = map.query(|m| m.spells.shots.len()).await.unwrap();
    assert_eq!(left, 0, "no shot left in the air");
}

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

/// A Young Nightsaber, of Shadowglen.
const YOUNG_NIGHTSABER: u32 = 2031;

/// The user's case: a level 1 night elf hunter, as she starts (her bow,
/// her arrows), auto-shoots a Young Nightsaber 20 yards off, in the real
/// world: the go first, the damage half a second on (Auto Shot's 40 yards
/// a second), the tiger's health dropping only then; what was sent passes
/// the client check.
#[tokio::test]
#[ignore = "extended"]
async fn a_night_elf_hunters_auto_shot_hits_a_tiger_when_the_arrow_lands() {
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let (mut a, ga, map) =
        crate::game::tests_pets::enter(&realm, "ARROW", "Hunter", Race::NightElf, Class::Hunter)
            .await;
    a.send(CMSG_COMPLETE_CINEMATIC {}).await;
    map.query(|m| m.spells.travel = Some(true)).await.unwrap();
    let tiger = crate::game::tests_pets::place(&map, ga, YOUNG_NIGHTSABER, 0x70_0A01, 20.0).await;
    // It stands to be shot again should the first arrow miss.
    map.query(move |m| {
        if let Some(s) = crate::game::map::event_ai::scripted_mut(m, tiger) {
            s.no_combat_movement = true;
        }
    })
    .await
    .unwrap();
    a.sees_on(&map, tiger).await;
    a.clear();
    cast(&a, AUTO_SHOT, Some(tiger)).await;
    let f = one_shot(&mut a, ga, &map, AUTO_SHOT, tiger).await;
    assert_flew(&f, 20.0, 40.0, "the hunter's Auto Shot");
    a.send(CMSG_CANCEL_AUTO_REPEAT_SPELL {}).await;
    let (check_py, wow_exe) = crate::game::tests_client_emu::checker()
        .expect("the client-emulation check: see docs/TESTING.md");
    let (capture, _) = crate::game::tests_client_emu::capture(vec![&mut a]);
    crate::game::tests_client_emu::run_checker(&check_py, &wow_exe, &capture);
}
