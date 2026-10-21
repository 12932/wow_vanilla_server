//! Phase 3's combat tests, ported from Thistle Tea's `test/game/core/combat`
//! (docs/parity/matrix.csv): the white and special tables' steps and
//! boundaries, what an outcome makes of a blow's damage, blocks and their
//! values, damage against creature types, armor, rage from a swing, parry
//! haste and the reactive windows.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::items::ItemTemplate;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::data::*;
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::auras;
use crate::game::map::combat::{self, Hand, Outcome, Roll};
use crate::game::map::creature::Creature;
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::store::BAG_PLAYER;

pub(crate) const START: (f32, f32, f32) = (-8949.95, -132.493, 83.5312);

pub(crate) const HUMANS: u32 = 1;
/// Hostile to humans, and not quick to notice them.
pub(crate) const PACK: u32 = 40;

/// A humanoid warrior of level 20; a beast.
pub(crate) const BRUTE: u32 = 3700;
pub(crate) const BEAST: u32 = 3701;
/// A humanoid on the humans' side.
pub(crate) const HELPER: u32 = 3702;

pub(crate) const SHIELD: u32 = 63_001;
/// A next-swing strike for ten.
pub(crate) const STRIKE: u32 = 95_200;
/// Ten threat, ten less, and an attack-me (Taunt's matching effect).
pub(crate) const THREAT_UP: u32 = 95_201;
pub(crate) const THREAT_DOWN: u32 = 95_202;
pub(crate) const ATTACK_ME: u32 = 95_203;
pub(crate) const SWORD: u32 = 63_002;

pub(crate) const OFF_HAND: u8 = 16;

pub(crate) fn faction(id: u32, ours: u32, enemy_mask: u32) -> FactionTemplate {
    FactionTemplate {
        id,
        faction: id,
        ours,
        friend_mask: ours,
        enemy_mask,
        ..Default::default()
    }
}

pub(crate) fn template(entry: u32, creature_type: u32) -> CreatureTemplate {
    CreatureTemplate {
        entry,
        name: format!("Creature {entry}"),
        display_ids: [903, 0, 0, 0],
        display_probabilities: [1, 0, 0, 0],
        level_min: 20,
        level_max: 20,
        faction: PACK,
        unit_class: 1,
        creature_type,
        health_multiplier: 1.0,
        damage_multiplier: 1.0,
        base_attack_time: 2000,
        detection_range: 0.1,
        ..Default::default()
    }
}

pub(crate) fn data() -> GameData {
    let mut d = GameData::empty();
    d.faction_templates.extend([
        (HUMANS, faction(HUMANS, 0x1, 0x2)),
        (PACK, faction(PACK, 0x2, 0x1)),
    ]);
    // A level-20 warrior creature has 45 strength, as vmangos's has.
    d.class_level_stats.insert(
        (1, 20),
        ClassLevelStats {
            health: 5000,
            melee_damage: 30.0,
            strength: 45,
            ..Default::default()
        },
    );
    // A human warrior of 60 with 140 strength and no agility.
    d.player_level_stats
        .insert((1, 1, 60), [140, 0, 20, 20, 20]);
    d.player_class_stats.insert((1, 60), (1000, 0));
    d.creature_templates.insert(BRUTE, template(BRUTE, 7));
    d.creature_templates.insert(BEAST, template(BEAST, 1));
    let mut helper = template(HELPER, 7);
    helper.faction = HUMANS;
    d.creature_templates.insert(HELPER, helper);
    d.items.insert(
        SHIELD,
        ItemTemplate {
            entry: SHIELD,
            class: 4,
            subclass: 6,
            name: "Shield".into(),
            display_id: 1000,
            inventory_type: 14,
            block: 20,
            stackable: 1,
            ..Default::default()
        },
    );
    d.items.insert(
        SWORD,
        ItemTemplate {
            entry: SWORD,
            class: 2,
            subclass: 7,
            name: "Sword".into(),
            display_id: 1000,
            inventory_type: 13,
            delay: 2000,
            stackable: 1,
            ..Default::default()
        },
    );
    d.spells.ranges.insert(1, (0.0, 30.0));
    let mut strike = SpellTemplate {
        id: STRIKE,
        name: "Strike".into(),
        range_index: 1,
        equipped_item_class: -1,
        dmg_class: 2,
        ..Default::default()
    };
    strike.effects[0] = SpellEffect {
        effect: 2,
        base_points: 9,
        base_dice: 1,
        die_sides: 1,
        target_a: 6,
        ..Default::default()
    };
    d.spells.spells.insert(STRIKE, strike);
    for (id, effect, points) in [
        (THREAT_UP, 63, 10),
        (THREAT_DOWN, 63, -10),
        (ATTACK_ME, 114, 0),
    ] {
        let mut s = SpellTemplate {
            id,
            name: format!("Spell {id}"),
            range_index: 1,
            equipped_item_class: -1,
            ..Default::default()
        };
        s.effects[0] = SpellEffect {
            effect,
            base_points: points - 1,
            base_dice: 1,
            die_sides: 1,
            target_a: 6,
            ..Default::default()
        };
        d.spells.spells.insert(id, s);
    }
    d
}

pub(crate) async fn setup(
    class: Class,
) -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    setup_with(data(), class).await
}

/// [`setup`], in a world of `data`.
pub(crate) async fn setup_with(
    data: GameData,
    class: Class,
) -> (std::sync::Arc<crate::game::Realm>, Client, Guid, MapHandle) {
    let realm = harness::realm_with(data);
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Alice", Race::Human, class).await;
    a.heartbeat(START.0, START.1, START.2).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    (realm, a, ga, map)
}

/// A creature of `entry` at `level`, `yards` in front of `g` and facing
/// them.
pub(crate) fn spawn(m: &mut Map, g: Guid, entry: u32, level: u8, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let t = data
        .creature_template(entry)
        .cloned()
        .expect("the template");
    let p = &m.players[&g];
    let (pos, o) = (p.position(), p.movement.orientation);
    let at = Vector3d {
        x: pos.x + yards * o.cos(),
        y: pos.y + yards * o.sin(),
        z: pos.z,
    };
    let counter = m.next_creature_counter;
    m.next_creature_counter += 1;
    let mut c = Creature::new(
        guid::unit(entry, counter | 0x80_0000),
        &t,
        level,
        at,
        o + std::f32::consts::PI,
    );
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 100_000);
    c.fields.set_u32(UNIT_FIELD_HEALTH, 100_000);
    c.fields.set_u32(UNIT_FIELD_RESISTANCES, 0);
    c.fields.set_f32(UNIT_FIELD_MINDAMAGE, 100.0);
    c.fields.set_f32(UNIT_FIELD_MAXDAMAGE, 100.0);
    let c = c.with_base();
    m.add_creature(c)
}

/// A test aura of `kind` and `misc`, holding `amount`, from `g` on itself.
pub(crate) fn aura_of(m: &mut Map, g: Guid, id: u32, kind: u32, misc: i32, amount: i32) {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect: 6,
        aura: kind,
        misc_value: misc,
        base_points: amount - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    let s = SpellTemplate {
        id,
        name: format!("Test Aura {id}"),
        equipped_item_class: -1,
        effects,
        ..Default::default()
    };
    let _ = auras::apply_spell(m, g, g, &s, &[(0, amount)], auras::Context::default());
}

/// Put an item of `entry` in the equipment slot `slot`.
pub(crate) fn equip(m: &mut Map, g: Guid, entry: u32, slot: u8) {
    if let Some(p) = m.players.get_mut(&g) {
        p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot == slot));
    }
    crate::game::map::inventory::add_item(m, g, entry, 1, false).expect("room for it");
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).expect("the player");
    let i = p
        .items
        .iter()
        .rposition(|i| i.entry == entry)
        .expect("the item");
    p.items[i].bag = BAG_PLAYER;
    p.items[i].slot = slot;
    p.refresh_inventory(&data);
    p.apply_stats(&data);
}

pub(crate) fn level(m: &mut Map, g: Guid, to: u8) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, to);
}

/// Swing `attacker`'s main hand at `victim` now, with `outcome` in place of
/// the roll.
pub(crate) fn forced_swing(m: &mut Map, attacker: Guid, victim: Guid, outcome: Outcome) {
    combat::FORCED_OUTCOME.with(|f| f.set(Some(outcome)));
    let now = m.now;
    if let Some(c) = m.combat_mut(attacker) {
        c.victim = victim;
        c.attacking = true;
        c.next_swing = Some(now);
        c.next_offhand = Some(now + Duration::from_secs(60));
    }
    combat::try_swing(m, attacker);
    combat::FORCED_OUTCOME.with(|f| f.set(None));
}

/// An even fight at `level`: a player's swing at a creature with the
/// usual five percent of each.
pub(crate) fn even(level: i32) -> Roll {
    Roll {
        attacker_skill: level * 5,
        victim_defense: level * 5,
        attacker_is_player: true,
        victim_is_player: false,
        attacker_level: level,
        victim_level: level,
        crit: 5.0,
        dodge: 5.0,
        parry: 5.0,
        block: 5.0,
        standing: true,
        can_crit: true,
        ..Default::default()
    }
}

#[test]
fn the_white_table_walks_miss_dodge_parry_glancing_block_and_crit_against_a_creature() {
    let r = even(20);
    // Miss 5%, dodge 5, parry 5, glancing 10, block 5, crit 5.
    for (roll, outcome) in [
        (499, Outcome::Miss),
        (500, Outcome::Dodge),
        (1_000, Outcome::Parry),
        (1_500, Outcome::Glancing),
        (2_500, Outcome::Block),
        (3_000, Outcome::Crit),
        (3_500, Outcome::Hit),
        (9_999, Outcome::Hit),
    ] {
        assert_eq!(combat::roll_outcome(&r, roll), outcome, "{roll}");
    }
    // A creature that never blocks: block's band is the crit's.
    let no_block = Roll {
        no_block: true,
        ..r
    };
    assert_eq!(combat::roll_outcome(&no_block, 2_500), Outcome::Crit);
    // A next-swing spell's blow can't glance.
    let queued = Roll { special: true, ..r };
    assert_eq!(combat::roll_outcome(&queued, 1_500), Outcome::Block);
    assert_eq!(combat::roll_special(&r, 1_500, 9_999), Outcome::Block);
    // From behind a creature still dodges, but neither parries nor blocks.
    let behind = Roll {
        from_behind: true,
        ..r
    };
    assert_eq!(combat::roll_outcome(&behind, 700), Outcome::Dodge);
    assert_eq!(combat::roll_outcome(&behind, 1_200), Outcome::Glancing);
    // A defender casting can't dodge, parry or block.
    let casting = Roll {
        victim_is_player: true,
        avoidance_disabled: true,
        ..r
    };
    assert_eq!(combat::roll_outcome(&casting, 800), Outcome::Crit);
}

#[test]
fn players_never_crush_and_an_always_crushing_creature_always_does() {
    // A level-20 player on a level-10 mage: no crushing blow however high
    // the roll.
    let r = Roll {
        attacker_skill: 100,
        victim_defense: 50,
        attacker_is_player: true,
        victim_is_player: true,
        attacker_level: 20,
        victim_level: 10,
        crit: 5.0,
        standing: true,
        can_crit: true,
        ..Default::default()
    };
    assert_eq!(combat::roll_outcome(&r, 9_999), Outcome::Hit);
    let crusher = Roll {
        attacker_is_player: false,
        always_crush: true,
        attacker_level: 20,
        victim_level: 20,
        victim_defense: 100,
        ..r
    };
    assert_eq!(combat::roll_outcome(&crusher, 9_999), Outcome::Crushing);
    // An ability's swing never crushes.
    assert_eq!(combat::roll_special(&crusher, 9_999, 9_999), Outcome::Hit);
}

#[test]
fn the_special_table_walks_miss_dodge_parry_and_block_then_rolls_its_crit_apart() {
    let r = even(20);
    for (roll, outcome) in [
        (499, Outcome::Miss),
        (500, Outcome::Dodge),
        (1_000, Outcome::Parry),
        (1_500, Outcome::Block),
        (2_000, Outcome::Hit),
    ] {
        assert_eq!(combat::roll_special(&r, roll, 9_999), outcome, "{roll}");
    }
    // An avoided special never crits, whatever its crit roll.
    assert_eq!(combat::roll_special(&r, 500, 0), Outcome::Dodge);
    // Where a white swing would glance, a special lands plainly.
    assert_eq!(combat::roll_special(&r, 2_000, 9_999), Outcome::Hit);
    // The crit is its own roll.
    assert_eq!(combat::roll_special(&r, 9_999, 0), Outcome::Crit);
    assert_eq!(combat::roll_special(&r, 9_999, 499), Outcome::Crit);
    assert_eq!(combat::roll_special(&r, 9_999, 500), Outcome::Hit);
}

#[test]
fn hit_takes_off_the_miss_after_a_low_creatures_scaling_and_past_ten_defense_the_first_point_is_lost()
 {
    // A level-5 creature's miss is half of five percent; then two points of
    // hit leave half a percent.
    let low = Roll {
        hit: 2.0,
        ..even(5)
    };
    assert_eq!(low.miss_bp(), 50);
    assert_eq!(combat::roll_outcome(&low, 49), Outcome::Miss);
    assert_eq!(combat::roll_outcome(&low, 50), Outcome::Dodge);
    // A level-23 creature against skill 100: 8%, and the first point of
    // hit does nothing.
    let high = Roll {
        victim_level: 23,
        victim_defense: 115,
        ..even(20)
    };
    for (hit, boundary) in [(0.0, 800), (1.0, 800), (8.0, 100), (9.0, 0), (-1.0, 900)] {
        let r = Roll { hit, ..high };
        assert_eq!(r.miss_bp(), boundary, "hit {hit}");
    }
    // Ten points apart, every point counts.
    let close = Roll {
        attacker_skill: 105,
        hit: 6.0,
        ..high
    };
    assert_eq!(close.miss_bp(), 0);
    // The defender's own aura on its attackers isn't the hit that's lost.
    let warded = Roll {
        attacker_hit: 1.0,
        ..high
    };
    assert_eq!(combat::roll_outcome(&warded, 699), Outcome::Miss);
    assert_ne!(combat::roll_outcome(&warded, 700), Outcome::Miss);
}

#[test]
fn skill_above_the_cap_glances_no_less_often_but_harder_and_takes_a_creatures_parry_down() {
    // A stunned level-23 creature: miss, then glancing to 40%.
    let stunned = Roll {
        victim_level: 23,
        victim_defense: 115,
        avoidance_disabled: true,
        ..even(20)
    };
    for (skill, boundary, kept) in [(100, 4_800, 550), (105, 4_600, 800)] {
        let r = Roll {
            attacker_skill: skill,
            ..stunned
        };
        assert_eq!(combat::roll_outcome(&r, boundary - 1), Outcome::Glancing);
        assert_ne!(combat::roll_outcome(&r, boundary), Outcome::Glancing);
        let factor = combat::glancing_factor(115 - skill, false, 0.0);
        let (info, _, damage, _) = combat::blow(Outcome::Glancing, 1000, 1000, 0, 2.0, factor);
        // Thistle Tea's 550 and 800; single precision, as mangos's, may
        // lose the last point.
        assert!(damage.abs_diff(kept) <= 1, "skill {skill}: {damage}");
        assert_eq!(
            info,
            combat::HITINFO_NORMALSWING2 | combat::HITINFO_GLANCING
        );
    }
    // Skill 105 against a level-20 creature: miss and dodge 4.5% each, and
    // parry 4.8%, the whole skill counted at 0.04% a point (mangos's
    // `CalculateEffectiveParryChance`; vmangos capped it, for 5%).
    let skilled = Roll {
        attacker_skill: 105,
        ..even(20)
    };
    assert_eq!(combat::roll_outcome(&skilled, 1_379), Outcome::Parry);
    assert_eq!(combat::roll_outcome(&skilled, 1_380), Outcome::Glancing);
    // Wand users glance harder: three fifths kept at the least.
    let factor = combat::glancing_factor(0, true, 0.0);
    let (_, _, damage, _) = combat::blow(Outcome::Glancing, 1000, 1000, 0, 2.0, factor);
    assert!(damage.abs_diff(600) <= 1, "{damage}");
}

#[test]
fn a_creatures_level_caps_the_crit_skill_can_take_off() {
    // Against a level-23 creature, skill past the attacker's level cap
    // takes no more off the crit: 5% - 3% for every skill.
    let high = Roll {
        victim_level: 23,
        victim_defense: 115,
        ..even(20)
    };
    for skill in [100, 105, 110] {
        let r = Roll {
            attacker_skill: skill,
            ..high
        };
        assert_eq!(
            combat::roll_special(&r, 9_999, 199),
            Outcome::Crit,
            "{skill}"
        );
        assert_eq!(
            combat::roll_special(&r, 9_999, 200),
            Outcome::Hit,
            "{skill}"
        );
    }
}

#[test]
fn an_outcome_makes_its_damage_crits_double_blocks_take_off_and_crushing_adds_half() {
    let crit = combat::blow(Outcome::Crit, 100, 100, 0, 2.0, 1.0);
    assert_eq!(
        crit,
        (
            combat::HITINFO_NORMALSWING2 | combat::HITINFO_CRITICALHIT,
            combat::VICTIMSTATE_NORMAL,
            200,
            0
        )
    );
    // A creature's 12 blocked of 100.
    let block = combat::blow(Outcome::Block, 100, 100, 12, 2.0, 1.0);
    assert_eq!(
        block,
        (
            combat::HITINFO_NORMALSWING2,
            combat::VICTIMSTATE_NORMAL,
            88,
            12
        )
    );
    // A block takes off what armor left, at most: all of it is a full
    // block.
    let full = combat::blow(Outcome::Block, 100, 25, 72, 2.0, 1.0);
    assert_eq!(
        full,
        (
            combat::HITINFO_NORMALSWING2,
            combat::VICTIMSTATE_BLOCKS,
            0,
            25
        )
    );
    let crushing = combat::blow(Outcome::Crushing, 100, 100, 0, 2.0, 1.0);
    assert_eq!(
        crushing,
        (
            combat::HITINFO_NORMALSWING2 | combat::HITINFO_CRUSHING,
            combat::VICTIMSTATE_NORMAL,
            150,
            0
        )
    );
    // A crit against a type its attacker slays: half again.
    let slaying = combat::blow(Outcome::Crit, 100, 100, 0, 3.0, 1.0);
    assert_eq!(slaying.2, 300);
}

#[tokio::test]
async fn a_creature_blocks_half_its_level_and_a_twentieth_of_its_strength_and_a_player_their_shield()
 {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (creature, values, shieldless) = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            // Its auras don't change a creature's.
            aura_of(m, brute, 90_001, auras::AURA_MOD_SHIELD_BLOCKVALUE, 0, 235);
            aura_of(
                m,
                brute,
                90_002,
                auras::AURA_MOD_SHIELD_BLOCKVALUE_PCT,
                0,
                30,
            );
            let creature = combat::block_value(m, brute);
            level(m, ga, 60);
            let p = m.players.get_mut(&ga).unwrap();
            p.spells.push(crate::game::map::derive::SPELL_BLOCK);
            let shieldless = (p.derived.block, p.derived.block_value);
            equip(m, ga, SHIELD, OFF_HAND);
            let mut values = vec![combat::block_value(m, ga)];
            // (20 + 30 + 140 / 20 - 1) * 1.3.
            aura_of(m, ga, 90_003, auras::AURA_MOD_SHIELD_BLOCKVALUE, 0, 30);
            aura_of(m, ga, 90_004, auras::AURA_MOD_SHIELD_BLOCKVALUE_PCT, 0, 30);
            values.push(combat::block_value(m, ga));
            // Another fifty percent multiplies, not adds.
            aura_of(m, ga, 90_005, auras::AURA_MOD_SHIELD_BLOCKVALUE_PCT, 0, 50);
            values.push(combat::block_value(m, ga));
            // A cut past all of it is none.
            aura_of(m, ga, 90_006, auras::AURA_MOD_SHIELD_BLOCKVALUE, 0, -1000);
            values.push(combat::block_value(m, ga));
            auras::remove_spell(m, ga, 90_006, None);
            aura_of(
                m,
                ga,
                90_007,
                auras::AURA_MOD_SHIELD_BLOCKVALUE_PCT,
                0,
                -150,
            );
            values.push(combat::block_value(m, ga));
            (creature, values, shieldless)
        })
        .await
        .unwrap();
    // 20 / 2 + 45 / 20.
    assert_eq!(creature, 12);
    assert_eq!(shieldless, (0.0, 0), "no shield, no block");
    // 20 + 140 / 20 - 1; then with the auras; then another half.
    assert_eq!(values, vec![26, 72, 109, 0, 0]);
}

#[tokio::test]
async fn a_shieldless_player_never_blocks_and_parry_auras_widen_the_parry() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (bare, shielded, parry) = map
        .query(move |m| {
            level(m, ga, 60);
            let brute = spawn(m, ga, BRUTE, 60, 3.0);
            let p = m.players.get_mut(&ga).unwrap();
            p.spells.push(crate::game::map::derive::SPELL_BLOCK);
            p.spells.push(crate::game::map::derive::SPELL_PARRY);
            // A parry needs a weapon in hand (mangos's GetUnitParryChance).
            equip(m, ga, SWORD, 15);
            let data = m.realm.data.clone();
            m.players.get_mut(&ga).unwrap().apply_stats(&data);
            aura_of(m, ga, 90_010, auras::AURA_MOD_PARRY_PERCENT, 0, 25);
            let roll = |m: &Map| combat::roll_for(m, brute, ga, Hand::Main).unwrap();
            let bare = roll(m);
            equip(m, ga, SHIELD, OFF_HAND);
            let shielded = roll(m);
            (bare, shielded, m.players[&ga].derived.parry)
        })
        .await
        .unwrap();
    assert_eq!(bare.block, 0.0);
    for r in [1_200, 5_000] {
        assert_ne!(combat::roll_outcome(&bare, r), Outcome::Block);
    }
    assert_eq!(parry, 30.0, "five and the aura's 25");
    // Miss 5%, no dodge without agility, parry 30%, then the block.
    let r = Roll {
        sheathed: false,
        ..shielded
    };
    assert_eq!(combat::roll_outcome(&r, 2_000), Outcome::Parry);
    assert_eq!(combat::roll_outcome(&r, 3_600), Outcome::Block);
}

#[tokio::test]
async fn damage_against_a_creature_type_scales_its_blows_and_its_crits() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (humanoid, beast, crit_humanoid, crit_beast) = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            let wolf = spawn(m, ga, BEAST, 20, 3.0);
            // Ten percent more against humanoids, and a crit's damage
            // doubled again against them.
            aura_of(m, ga, 90_020, auras::AURA_MOD_DAMAGE_DONE_VERSUS, 0x40, 10);
            aura_of(
                m,
                ga,
                90_021,
                auras::AURA_MOD_CRIT_PERCENT_VERSUS,
                0x40,
                100,
            );
            if let Some(f) = m.unit_fields_mut(ga) {
                f.set_f32(UNIT_FIELD_MINDAMAGE, 100.0);
                f.set_f32(UNIT_FIELD_MAXDAMAGE, 100.0);
            }
            let mut rng = rand::rng();
            let humanoid = combat::weapon_damage(m, ga, brute, Hand::Main, &mut rng).0;
            let beast = combat::weapon_damage(m, ga, wolf, Hand::Main, &mut rng).0;
            (
                humanoid,
                beast,
                combat::crit_multiplier(m, ga, brute),
                combat::crit_multiplier(m, ga, wolf),
            )
        })
        .await
        .unwrap();
    assert_eq!(humanoid, 110);
    assert_eq!(beast, 100);
    assert_eq!(crit_humanoid, 3.0);
    assert_eq!(crit_beast, 2.0);
}

#[tokio::test]
async fn armor_takes_only_physical_blows_down_and_never_below_one() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (physical, fire, floor) = map
        .query(move |m| {
            level(m, ga, 20);
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            m.unit_fields_mut(brute)
                .unwrap()
                .set_u32(UNIT_FIELD_RESISTANCES, 1000);
            let physical = combat::mitigate(m, ga, brute, 100, 1);
            let fire = combat::mitigate(m, ga, brute, 100, 4);
            m.unit_fields_mut(brute)
                .unwrap()
                .set_u32(UNIT_FIELD_RESISTANCES, 5000);
            let floor = combat::mitigate(m, ga, brute, 1, 1);
            (physical, fire, floor)
        })
        .await
        .unwrap();
    // 1000 / (1000 + 85 * 20 + 400) stopped.
    assert_eq!(physical, 68);
    assert_eq!(fire, 100);
    assert_eq!(floor, 1);
}

#[test]
fn rage_for_a_swing_follows_its_damage_and_level() {
    // 200 dealt at 60: six and a half rage; three quarters of it for a
    // dodge or parry.
    assert_eq!(combat::rage_for(200, 60, false), 65);
    assert_eq!(combat::rage_for(150, 60, false), 48);
}

#[tokio::test]
async fn a_swing_gives_rage_when_it_lands_three_quarters_when_avoided_and_none_on_a_miss() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let rage = map
        .query(move |m| {
            level(m, ga, 60);
            let brute = spawn(m, ga, BRUTE, 60, 1.0);
            if let Some(f) = m.unit_fields_mut(ga) {
                f.set_f32(UNIT_FIELD_MINDAMAGE, 200.0);
                f.set_f32(UNIT_FIELD_MAXDAMAGE, 200.0);
            }
            let mut out = Vec::new();
            for o in [Outcome::Hit, Outcome::Dodge, Outcome::Parry, Outcome::Miss] {
                m.unit_fields_mut(ga)
                    .unwrap()
                    .set_u32(UNIT_FIELD_POWER1 + 1, 0);
                forced_swing(m, ga, brute, o);
                out.push(m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_POWER1 + 1));
            }
            out
        })
        .await
        .unwrap();
    assert_eq!(rage, vec![65, 48, 48, 0]);
}

#[tokio::test]
async fn a_mana_user_gets_no_rage_from_a_swing() {
    let (_realm, _a, ga, map) = setup(Class::Paladin).await;
    let (mana, rage) = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            let before = m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_POWER1);
            forced_swing(m, ga, brute, Outcome::Hit);
            let f = m.unit_fields(ga).unwrap();
            (
                f.get_u32(UNIT_FIELD_POWER1) == before,
                f.get_u32(UNIT_FIELD_POWER1 + 1),
            )
        })
        .await
        .unwrap();
    assert!(mana);
    assert_eq!(rage, 0);
}

/// A creature in a fight, its next swing `main` milliseconds off and its
/// off hand's `off` (if it has one).
pub(crate) fn parrier(m: &mut Map, g: Guid, main: i64, off: Option<i64>) -> Guid {
    let brute = spawn(m, g, BRUTE, 20, 1.0);
    let now = m.now;
    let at = |ms: i64| {
        if ms >= 0 {
            now + Duration::from_millis(ms as u64)
        } else {
            now - Duration::from_millis((-ms) as u64)
        }
    };
    if let Some(off) = off {
        let c = m.creature_mut(brute).unwrap();
        c.fields.set_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + 1, 1);
        c.fields.set_u32(UNIT_VIRTUAL_ITEM_INFO + 2, 2);
        c.fields.set_u32(UNIT_FIELD_BASEATTACKTIME + 1, 1000);
        let _ = off;
    }
    let c = m.combat_mut(brute).unwrap();
    c.attacking = true;
    c.victim = g;
    c.next_swing = Some(at(main));
    c.next_offhand = off.map(at);
    brute
}

/// What's left of each hand's wait, in milliseconds.
pub(crate) fn waits(m: &Map, g: Guid) -> (Option<u64>, Option<u64>) {
    let now = m.now;
    let c = m.combat(g).unwrap();
    let left = |t: Option<std::time::Instant>| {
        t.map(|t| t.saturating_duration_since(now).as_millis() as u64)
    };
    (left(c.next_swing), left(c.next_offhand))
}

#[tokio::test]
async fn a_parry_hastens_the_earlier_hand_by_two_fifths_to_no_less_than_a_fifth() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let results = map
        .query(move |m| {
            let mut out = Vec::new();
            // Two seconds left of 2000: 1200 after.
            let g = parrier(m, ga, 2_000, None);
            combat::parry_haste(m, g);
            out.push(waits(m, g).0);
            // Shortened past the floor: a fifth.
            for left in [401, 700, 1_200] {
                let g = parrier(m, ga, left, None);
                combat::parry_haste(m, g);
                out.push(waits(m, g).0);
            }
            // Imminent or overdue: left alone.
            for left in [100, 400] {
                let g = parrier(m, ga, left, None);
                combat::parry_haste(m, g);
                out.push(waits(m, g).0);
            }
            // Again and again: never past the floor.
            let g = parrier(m, ga, 2_000, None);
            for _ in 0..5 {
                combat::parry_haste(m, g);
            }
            out.push(waits(m, g).0);
            out
        })
        .await
        .unwrap();
    let close = |got: Option<u64>, want: u64| got.is_some_and(|g| g.abs_diff(want) <= 2);
    assert!(close(results[0], 1_200), "{results:?}");
    for r in &results[1..4] {
        assert!(close(*r, 400), "{results:?}");
    }
    assert!(close(results[4], 100), "{results:?}");
    assert!(close(results[5], 400), "{results:?}");
    assert!(close(results[6], 400), "{results:?}");
}

#[tokio::test]
async fn a_parry_hastens_the_off_hand_only_when_it_is_earlier_and_armed() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (earlier_off, equal, imminent_off, unarmed_off, idle) = map
        .query(move |m| {
            // The off hand first, of a 1000 period: 600 after.
            let g = parrier(m, ga, 2_000, Some(1_000));
            combat::parry_haste(m, g);
            let earlier_off = waits(m, g);
            // Even: the main hand.
            let g = parrier(m, ga, 1_000, Some(1_000));
            combat::parry_haste(m, g);
            let equal = waits(m, g);
            // An imminent off hand gives its haste to neither.
            let g = parrier(m, ga, 2_000, Some(100));
            combat::parry_haste(m, g);
            let imminent_off = waits(m, g);
            // Without an off-hand weapon its timer doesn't count.
            let g = parrier(m, ga, 2_000, None);
            let now = m.now;
            m.combat_mut(g).unwrap().next_offhand = Some(now + Duration::from_millis(500));
            combat::parry_haste(m, g);
            let unarmed_off = waits(m, g);
            // Out of a fight, no timers are made.
            let g = parrier(m, ga, 2_000, None);
            let c = m.combat_mut(g).unwrap();
            c.attacking = false;
            c.next_swing = None;
            c.next_offhand = None;
            combat::parry_haste(m, g);
            let c = m.combat(g).unwrap();
            let idle = (c.next_swing, c.next_offhand);
            (earlier_off, equal, imminent_off, unarmed_off, idle)
        })
        .await
        .unwrap();
    let close = |got: Option<u64>, want: u64| got.is_some_and(|g| g.abs_diff(want) <= 2);
    assert!(
        close(earlier_off.1, 600) && close(earlier_off.0, 2_000),
        "{earlier_off:?}"
    );
    assert!(close(equal.0, 400) && close(equal.1, 1_000), "{equal:?}");
    assert!(
        close(imminent_off.0, 2_000) && close(imminent_off.1, 100),
        "{imminent_off:?}"
    );
    assert!(
        close(unarmed_off.0, 1_200) && close(unarmed_off.1, 500),
        "{unarmed_off:?}"
    );
    assert_eq!(idle, (None, None));
}

#[tokio::test]
async fn a_parry_hastens_by_the_weapons_own_period_not_the_hasted_one() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let left = map
        .query(move |m| {
            let g = parrier(m, ga, 1_000, None);
            // Twice as fast: a 1000 period of a 2000 weapon.
            aura_of(m, g, 90_030, auras::AURA_MOD_ATTACKSPEED, 0, 100);
            assert_eq!(
                m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_BASEATTACKTIME),
                1000
            );
            let now = m.now;
            m.combat_mut(g).unwrap().next_swing = Some(now + Duration::from_millis(1_000));
            combat::parry_haste(m, g);
            waits(m, g).0
        })
        .await
        .unwrap();
    // Two fifths of 2000 off, to a fifth of 2000.
    assert!(left.is_some_and(|l| l.abs_diff(400) <= 2), "{left:?}");
}

#[tokio::test]
async fn a_parried_swing_hastens_the_parrier_and_deals_nothing() {
    let (_realm, mut a, ga, map) = setup(Class::Warrior).await;
    let (brute, left, health) = map
        .query(move |m| {
            let brute = parrier(m, ga, 2_000, None);
            let before = m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH);
            forced_swing(m, ga, brute, Outcome::Parry);
            let after = m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH);
            (brute, waits(m, brute).0, before == after)
        })
        .await
        .unwrap();
    assert!(left.is_some_and(|l| l.abs_diff(1_200) <= 2), "{left:?}");
    assert!(health);
    let frame = a
        .wait_frame(
            "the parried swing",
            combat::SMSG_ATTACKERSTATEUPDATE,
            |_| true,
        )
        .await;
    let (_, damage, state, _) = swing_log(&frame);
    let _ = brute;
    assert_eq!((damage, state), (0, combat::VICTIMSTATE_PARRY));
}

/// A swing's log: its hit info, damage, the victim's state and what was
/// blocked.
pub(crate) fn swing_log(b: &[u8]) -> (u32, u32, u32, u32) {
    let u = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let mut at = 4;
    for _ in 0..2 {
        let mask = b[at];
        at += 1 + mask.count_ones() as usize;
    }
    let n = b.len();
    (u(0), u(at), u(n - 16), u(n - 4))
}

#[test]
fn a_ranged_attack_is_never_parried() {
    let r = Roll {
        ranged: true,
        parry: 100.0,
        ..even(20)
    };
    for roll in [500, 1_000, 5_000] {
        assert_ne!(combat::roll_outcome(&r, roll), Outcome::Parry);
    }
}

#[tokio::test]
async fn avoidance_opens_each_classs_own_window_and_the_bits_close_after_four_seconds() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let bits = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            let mut out = Vec::new();
            let state = |m: &Map| {
                m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_AURASTATE)
                    & (combat::AURA_STATE_DEFENSE | combat::AURA_STATE_HUNTER_PARRY)
            };
            let reset = |m: &mut Map, class: u8| {
                let p = m.players.get_mut(&ga).unwrap();
                p.row.class = class;
                p.combat.defense_window = None;
                p.combat.hunter_parry_window = None;
                combat::sync_aura_state(m, ga);
            };
            // A hunter's dodge is the defense window, its parry its own.
            for (class, outcome) in [
                (3, Outcome::Dodge),
                (3, Outcome::Parry),
                (4, Outcome::Dodge),
                (4, Outcome::Parry),
                (1, Outcome::Dodge),
                (1, Outcome::Parry),
                (1, Outcome::Block),
            ] {
                reset(m, class);
                combat::after_outcome(m, brute, ga, outcome);
                out.push(state(m));
            }
            // A hunter's dodge then parry keep apart.
            reset(m, 3);
            combat::after_outcome(m, brute, ga, Outcome::Dodge);
            combat::after_outcome(m, brute, ga, Outcome::Parry);
            out.push(state(m));
            // The window's time runs out: the bit goes.
            let now = m.now;
            m.players.get_mut(&ga).unwrap().combat.defense_window =
                Some((brute, now - Duration::from_millis(1)));
            combat::update_players(m);
            out.push(state(m));
            reset(m, 1);
            out
        })
        .await
        .unwrap();
    let (d, h) = (combat::AURA_STATE_DEFENSE, combat::AURA_STATE_HUNTER_PARRY);
    assert_eq!(bits, vec![d, h, 0, d, d, d, d, d | h, h]);
}

#[tokio::test]
async fn only_a_warrior_gets_a_dodging_targets_combo_point() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (warrior, rogue) = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            combat::after_outcome(m, ga, brute, Outcome::Dodge);
            let warrior = {
                let p = &m.players[&ga];
                (p.caster.combo_points, p.caster.combo_target == brute)
            };
            let p = m.players.get_mut(&ga).unwrap();
            p.caster.combo_points = 0;
            p.caster.combo_target = Guid::zero();
            p.row.class = 4;
            combat::after_outcome(m, ga, brute, Outcome::Dodge);
            let rogue = m.players[&ga].caster.combo_points;
            m.players.get_mut(&ga).unwrap().row.class = 1;
            (warrior, rogue)
        })
        .await
        .unwrap();
    assert_eq!(warrior, (1, true));
    assert_eq!(rogue, 0);
}

#[tokio::test]
async fn under_a_fifth_of_health_a_unit_is_healthless_for_execute_and_keeps_its_other_bits() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let states = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            let mut out = Vec::new();
            let mut set = |m: &mut Map, g: Guid, health: u32, extra: u32| {
                let f = m.unit_fields_mut(g).unwrap();
                let max = f.get_u32(UNIT_FIELD_MAXHEALTH);
                f.set_u32(UNIT_FIELD_HEALTH, max * health / 100);
                let bits = f.get_u32(UNIT_FIELD_AURASTATE);
                f.set_u32(UNIT_FIELD_AURASTATE, bits | extra);
                combat::sync_aura_state(m, g);
                out.push(
                    m.unit_fields(g).unwrap().get_u32(UNIT_FIELD_AURASTATE)
                        & (combat::AURA_STATE_HEALTHLESS_20 | combat::AURA_STATE_DEFENSE | 0x10),
                );
            };
            set(m, ga, 19, 0);
            set(m, ga, 50, 0);
            set(m, ga, 0, 0);
            set(m, brute, 15, 0);
            // A judgement's bit stays.
            set(m, ga, 50, 0x10);
            // As does a defense window's.
            combat::after_outcome(m, brute, ga, Outcome::Block);
            set(m, ga, 10, 0);
            out
        })
        .await
        .unwrap();
    let (low, def) = (combat::AURA_STATE_HEALTHLESS_20, combat::AURA_STATE_DEFENSE);
    assert_eq!(states, vec![low, 0, 0, low, 0x10, low | def | 0x10]);
}

#[tokio::test]
async fn hit_auras_move_the_miss_and_a_defenders_ward_is_melee_or_ranged() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (dazed, melee, ranged, unwarded) = map
        .query(move |m| {
            level(m, ga, 20);
            let brute = spawn(m, ga, BRUTE, 20, 3.0);
            // Two points less hit on the player's swings.
            aura_of(m, ga, 90_040, auras::AURA_MOD_HIT_CHANCE, 0, -2);
            let dazed = combat::roll_for(m, ga, brute, Hand::Main).unwrap();
            // The player's ward on those shooting at them.
            let unwarded = combat::roll_for(m, brute, ga, Hand::Ranged).unwrap();
            aura_of(
                m,
                ga,
                90_041,
                auras::AURA_MOD_ATTACKER_RANGED_HIT_CHANCE,
                0,
                -20,
            );
            let melee = combat::roll_for(m, brute, ga, Hand::Main).unwrap();
            let ranged = combat::roll_for(m, brute, ga, Hand::Ranged).unwrap();
            (dazed, melee, ranged, unwarded)
        })
        .await
        .unwrap();
    assert_eq!(dazed.hit, -2.0);
    assert_eq!(dazed.miss_bp(), 700);
    assert_eq!(combat::roll_outcome(&dazed, 600), Outcome::Miss);
    assert_eq!(melee.attacker_hit, 0.0);
    assert_eq!(ranged.attacker_hit, -20.0);
    assert_eq!(combat::roll_outcome(&ranged, 2_000), Outcome::Miss);
    assert_ne!(combat::roll_outcome(&unwarded, 2_000), Outcome::Miss);
}

#[tokio::test]
async fn two_weapons_pay_the_penalty_unless_a_strike_is_queued_and_a_creatures_shield_is_no_weapon()
{
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (armed, queued, consumed, unarmed, creature) = map
        .query(move |m| {
            let brute = spawn(m, ga, BRUTE, 20, 1.0);
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .push(crate::game::map::derive::SPELL_DUAL_WIELD);
            equip(m, ga, SWORD, 15);
            equip(m, ga, SWORD, OFF_HAND);
            let dual = |m: &Map| {
                combat::roll_for(m, ga, brute, Hand::Main)
                    .unwrap()
                    .dual_wield
            };
            let armed = dual(m);
            combat::queue_next_swing(m, ga, STRIKE);
            let queued = dual(m);
            m.combat_mut(ga).unwrap().queued = None;
            let consumed = dual(m);
            let p = m.players.get_mut(&ga).unwrap();
            p.items
                .retain(|i| !(i.bag == BAG_PLAYER && i.slot == OFF_HAND));
            let data = m.realm.data.clone();
            p.refresh_inventory(&data);
            p.apply_stats(&data);
            let unarmed = dual(m);
            // A creature's off hand: a weapon, a shield, a held thing.
            let mut creature = Vec::new();
            for class in [2, 4, 0] {
                let c = m.creature_mut(brute).unwrap();
                c.fields.set_u32(UNIT_VIRTUAL_ITEM_SLOT_DISPLAY + 1, 1);
                c.fields.set_u32(UNIT_VIRTUAL_ITEM_INFO + 2, class);
                creature.push(combat::dual_wields(m, brute));
            }
            (armed, queued, consumed, unarmed, creature)
        })
        .await
        .unwrap();
    assert!(armed);
    assert!(!queued, "a queued strike's swing pays no penalty");
    assert!(consumed, "and the next one does again");
    assert!(!unarmed);
    assert_eq!(creature, vec![true, false, false]);
}

#[tokio::test]
async fn a_shields_block_chance_takes_its_auras_and_goes_with_it_and_strength_counts_in_twentieths()
{
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (chances, fractional) = map
        .query(move |m| {
            level(m, ga, 60);
            m.players
                .get_mut(&ga)
                .unwrap()
                .spells
                .push(crate::game::map::derive::SPELL_BLOCK);
            equip(m, ga, SHIELD, OFF_HAND);
            let block = |m: &Map| m.players[&ga].derived.block;
            let mut chances = vec![block(m)];
            aura_of(m, ga, 90_050, auras::AURA_MOD_BLOCK_PERCENT, 0, 75);
            chances.push(block(m));
            auras::remove_spell(m, ga, 90_050, None);
            chances.push(block(m));
            aura_of(m, ga, 90_050, auras::AURA_MOD_BLOCK_PERCENT, 0, 75);
            let p = m.players.get_mut(&ga).unwrap();
            p.items
                .retain(|i| !(i.bag == BAG_PLAYER && i.slot == OFF_HAND));
            let data = m.realm.data.clone();
            p.refresh_inventory(&data);
            p.apply_stats(&data);
            chances.push(block(m));
            auras::remove_spell(m, ga, 90_050, None);
            // 39 strength: (20 + 1.95 - 1) * 1.3, the fraction kept to the
            // end.
            equip(m, ga, SHIELD, OFF_HAND);
            aura_of(m, ga, 90_051, auras::AURA_MOD_STAT, 0, -101);
            aura_of(m, ga, 90_052, auras::AURA_MOD_SHIELD_BLOCKVALUE_PCT, 0, 30);
            let strength = m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_STAT0);
            (chances, (strength, combat::block_value(m, ga)))
        })
        .await
        .unwrap();
    assert_eq!(chances, vec![5.0, 80.0, 5.0, 0.0]);
    assert_eq!(fractional, (39, 27));
}

#[tokio::test]
async fn a_queued_strike_takes_the_swing_and_gives_no_rage_and_only_a_parry_hastens() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (hurt, rage, untouched) = map
        .query(move |m| {
            level(m, ga, 60);
            let brute = parrier(m, ga, 2_000, None);
            let before = m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH);
            m.unit_fields_mut(ga)
                .unwrap()
                .set_u32(UNIT_FIELD_POWER1 + 1, 0);
            m.players.get_mut(&ga).unwrap().spells.push(STRIKE);
            combat::queue_next_swing(m, ga, STRIKE);
            forced_swing(m, ga, brute, Outcome::Hit);
            let after = m.unit_fields(brute).unwrap().get_u32(UNIT_FIELD_HEALTH);
            let rage = m.unit_fields(ga).unwrap().get_u32(UNIT_FIELD_POWER1 + 1);
            let now = m.now;
            m.combat_mut(brute).unwrap().next_swing = Some(now + Duration::from_millis(2_000));
            let mut untouched = Vec::new();
            for o in [
                Outcome::Dodge,
                Outcome::Block,
                Outcome::Miss,
                Outcome::Hit,
                Outcome::Crit,
            ] {
                combat::after_outcome(m, ga, brute, o);
                untouched.push(waits(m, brute).0);
            }
            (before - after, rage, untouched)
        })
        .await
        .unwrap();
    assert_eq!(hurt, 10, "the strike's ten, not a swing's hundred");
    assert_eq!(rage, 0);
    for left in untouched {
        assert!(left.is_some_and(|l| l.abs_diff(2_000) <= 2), "{left:?}");
    }
}

#[tokio::test]
async fn reactive_windows_last_four_seconds_for_their_own_target_and_a_creature_opens_none() {
    let (_realm, _a, ga, map) = setup(Class::Warrior).await;
    let (deadline, targets, refreshed, rogue, creature) = map
        .query(move |m| {
            let one = spawn(m, ga, BRUTE, 20, 3.0);
            let two = spawn(m, ga, BRUTE, 20, 3.0);
            let three = spawn(m, ga, BRUTE, 20, 3.0);
            let now = m.now;
            combat::after_outcome(m, one, ga, Outcome::Block);
            let deadline =
                m.players[&ga].combat.defense_window == Some((one, now + combat::REACTIVE_WINDOW));
            // A hunter: the dodge's window for one, the parry's for two.
            let p = m.players.get_mut(&ga).unwrap();
            p.row.class = 3;
            p.combat.defense_window = None;
            combat::after_outcome(m, one, ga, Outcome::Dodge);
            combat::after_outcome(m, two, ga, Outcome::Parry);
            let c = &m.players[&ga].combat;
            let targets = (
                c.defense_window.map(|w| w.0) == Some(one),
                c.hunter_parry_window.map(|w| w.0) == Some(two),
            );
            // Another dodge moves the defense window to three, and leaves
            // the parry's.
            let parry_before = c.hunter_parry_window;
            combat::after_outcome(m, three, ga, Outcome::Dodge);
            let c = &m.players[&ga].combat;
            let refreshed = (
                c.defense_window.map(|w| w.0) == Some(three),
                c.hunter_parry_window == parry_before,
            );
            // A rogue's dodge doesn't move its Riposte window.
            let p = m.players.get_mut(&ga).unwrap();
            p.row.class = 4;
            p.combat.defense_window = None;
            combat::after_outcome(m, one, ga, Outcome::Parry);
            let riposte = m.players[&ga].combat.defense_window;
            combat::after_outcome(m, two, ga, Outcome::Dodge);
            let rogue = m.players[&ga].combat.defense_window == riposte && riposte.is_some();
            m.players.get_mut(&ga).unwrap().row.class = 1;
            // A creature that blocks opens nothing.
            combat::after_outcome(m, ga, one, Outcome::Block);
            let c = m.combat(one).unwrap();
            let creature = c.defense_window.is_none()
                && m.unit_fields(one).unwrap().get_u32(UNIT_FIELD_AURASTATE)
                    & combat::AURA_STATE_DEFENSE
                    == 0;
            (deadline, targets, refreshed, rogue, creature)
        })
        .await
        .unwrap();
    assert!(deadline);
    assert_eq!(targets, (true, true));
    assert_eq!(refreshed, (true, true));
    assert!(rogue);
    assert!(creature);
}
