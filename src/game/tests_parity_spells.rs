//! Stage 2a's spells and auras, and 2h's silithyst, against the real data
//! when it is here: a dispel taking a passive aura, weapons' feral attack
//! power held to the forms, healing taken by the spell's coefficient and at
//! each tick, a killing spell that spares the gear, the Net-o-Matic's
//! backfire, Premeditation's points taken back, and the carrier's PvP
//! pulse.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells;
use crate::game::map::{Map, MapKey, auras};
use crate::game::store::BAG_PLAYER;

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

fn realm() -> std::sync::Arc<crate::game::Realm> {
    harness::realm_with(crate::game::data::load::load(DB.as_path()))
}

/// A Kobold Vermin at `level`, `yards` in front of `g`, hardy and without
/// armor.
fn dummy(m: &mut Map, g: Guid, level: u8, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let template = data.creature_template(6).cloned().expect("the kobold");
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
        guid::unit(6, counter | 0x80_0000),
        &template,
        level,
        at,
        o + std::f32::consts::PI,
    );
    c.fields.set_u32(UNIT_FIELD_MAXHEALTH, 1_000_000);
    c.fields.set_u32(UNIT_FIELD_HEALTH, 1_000_000);
    c.fields.set_u32(UNIT_FIELD_RESISTANCES, 0);
    let c = c.with_base();
    m.add_creature(c)
}

fn ready(m: &mut Map, g: Guid, level: u8) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, level);
    for i in 0..5 {
        p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + i, 100_000);
        p.fields.set_u32(UNIT_FIELD_POWER1 + i, 100_000);
    }
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// `id`'s auras from `caster` on `target`, as the data has them.
fn apply(m: &mut Map, caster: Guid, target: Guid, id: u32) {
    let data = m.realm.data.clone();
    let s = data.spells.get(id).unwrap().clone();
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].effect == 6 && s.effects[*i].aura != 0)
        .map(|i| (i, spells::points(&s, i, 60)))
        .collect();
    let _ = auras::apply_spell(m, caster, target, &s, &effects, auras::Context::default());
}

/// A test aura of `kind` and `misc`, holding `amount`, on `g`.
fn aura_of(m: &mut Map, g: Guid, id: u32, kind: u32, misc: i32, amount: i32) {
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
        name: "Test Aura".into(),
        equipped_item_class: -1,
        effects,
        ..Default::default()
    };
    let _ = auras::apply_spell(m, g, g, &s, &[(0, amount)], auras::Context::default());
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

fn set_health(m: &mut Map, g: Guid, h: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_HEALTH, h);
    }
}

/// Run `g`'s ticks of `spell` now: what they healed.
fn tick_now(m: &mut Map, g: Guid, spell: u32) -> u32 {
    let now = m.now;
    if let Some(a) = auras::auras_mut(m, g) {
        for h in a.holders.iter_mut().filter(|h| h.spell == spell) {
            for x in h.effects.iter_mut() {
                x.next_tick = Some(now);
            }
        }
    }
    let before = health(m, g);
    auras::update(m);
    health(m, g) - before
}

const LIGHTNING_HIDE: u32 = 7273;
const PURGE: u32 = 8012;

#[tokio::test]
async fn a_dispel_takes_a_passive_aura_of_its_kind() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "P2A1");
    let mage = c.enter("Parone", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, mage, 60);
            // A creature's passive Lightning Hide, of magic.
            let kobold = dummy(m, mage, 60, 5.0);
            apply(m, kobold, kobold, LIGHTNING_HIDE);
            let passive = auras::auras(m, kobold)
                .and_then(|a| a.holders.iter().find(|h| h.spell == LIGHTNING_HIDE))
                .map(|h| (h.passive, h.dispel));
            spells::cast_triggered(m, mage, PURGE, Some(kobold));
            (passive, has(m, kobold, LIGHTNING_HIDE))
        })
        .await
        .unwrap();
    assert_eq!(out.0, Some((true, 1)), "a passive aura of magic");
    assert!(!out.1, "purged, as Thistle Tea and mangos purge it");
    let dispelled = c
        .wait_for("SMSG_SPELLDISPELLOG", |m| match m {
            ServerOpcodeMessage::SMSG_SPELLDISPELLOG(x) => Some(x.spells.clone()),
            _ => None,
        })
        .await;
    assert_eq!(dispelled, vec![LIGHTNING_HIDE]);
}

const HAMMER_OF_BESTIAL_FURY: u32 = 20580;
const FERAL_ATTACK_POWER: u32 = 24994;
const CAT_FORM: u32 = 768;
const BEAR_FORM: u32 = 5487;

#[tokio::test]
async fn a_weapons_feral_attack_power_holds_only_in_the_forms_it_names() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "P2A2");
    let druid = c.enter("Partwo", Race::NightElf, Class::Druid).await;
    let map = realm.maps.get(&realm, MapKey::world(1));
    let out = map
        .query(move |m| {
            ready(m, druid, 60);
            let data = m.realm.data.clone();
            let p = m.players.get_mut(&druid).unwrap();
            p.items.retain(|i| !(i.bag == BAG_PLAYER && i.slot == 15));
            crate::game::map::inventory::add_item(m, druid, HAMMER_OF_BESTIAL_FURY, 1, false)
                .expect("room for it");
            let p = m.players.get_mut(&druid).unwrap();
            let i = p
                .items
                .iter()
                .rposition(|i| i.entry == HAMMER_OF_BESTIAL_FURY)
                .unwrap();
            p.items[i].bag = BAG_PLAYER;
            p.items[i].slot = 15;
            p.refresh_inventory(&data);
            crate::game::map::inventory::sync_gear(m, druid);
            let caster = has(m, druid, FERAL_ATTACK_POWER);
            spells::cast_triggered(m, druid, CAT_FORM, Some(druid));
            let cat = has(m, druid, FERAL_ATTACK_POWER);
            auras::remove_spell(m, druid, CAT_FORM, None);
            let left = has(m, druid, FERAL_ATTACK_POWER);
            spells::cast_triggered(m, druid, BEAR_FORM, Some(druid));
            let bear = has(m, druid, FERAL_ATTACK_POWER);
            // A change of gear in a form keeps it.
            crate::game::map::inventory::sync_gear(m, druid);
            let synced = has(m, druid, FERAL_ATTACK_POWER);
            (caster, cat, left, bear, synced)
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (false, true, false, true, true),
        "(caster form, cat, cat left, bear, bear synced)"
    );
}

const FLASH_HEAL: u32 = 2061;
const RENEW: u32 = 6078;

#[tokio::test]
async fn flat_healing_taken_goes_by_the_spells_coefficient() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "P2A3");
    let priest = c.enter("Parthree", Race::Human, Class::Priest).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, priest, 60);
            // +100 healing taken, all schools (Amplify Magic's kind).
            aura_of(m, priest, 990_901, auras::AURA_MOD_HEALING, 0x7e, 100);
            // Flash Heal: 193 to 237, and 0.429 of the hundred.
            set_health(m, priest, 1);
            spells::cast_triggered(m, priest, FLASH_HEAL, Some(priest));
            let flash = health(m, priest) - 1;
            // Renew: 80 a tick, and 0.2 of the hundred.
            set_health(m, priest, 1);
            spells::cast_triggered(m, priest, RENEW, Some(priest));
            (flash, tick_now(m, priest, RENEW))
        })
        .await
        .unwrap();
    let (flash, tick) = out;
    assert!(
        (193 + 42..=237 + 43).contains(&flash),
        "the flat bonus by Flash Heal's coefficient: {flash}"
    );
    assert_eq!(tick, 100, "by Renew's tick's");
}

#[tokio::test]
async fn a_heal_over_time_takes_healing_taken_at_each_tick() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "P2A8");
    let priest = c.enter("Pareight", Race::Human, Class::Priest).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let ticks = map
        .query(move |m| {
            ready(m, priest, 60);
            // Renew, 80 a tick; a half cut after it lands halves the next
            // tick, and the cut gone, the one after is whole.
            set_health(m, priest, 1);
            spells::cast_triggered(m, priest, RENEW, Some(priest));
            let first = tick_now(m, priest, RENEW);
            aura_of(m, priest, 990_902, auras::AURA_MOD_HEALING_PCT, 0, -50);
            let cut = tick_now(m, priest, RENEW);
            auras::remove_spell(m, priest, 990_902, None);
            let again = tick_now(m, priest, RENEW);
            [first, cut, again]
        })
        .await
        .unwrap();
    assert_eq!(ticks, [80, 40, 80]);
}

const RITUAL_OF_DOOM_SACRIFICE: u32 = 20625;
const DEATH_TOUCH: u32 = 5;

#[tokio::test]
async fn a_killing_spell_without_durability_loss_spares_the_gear() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "P2A4");
    let warrior = c.enter("Parfour", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, warrior, 60);
            let kobold = dummy(m, warrior, 60, 5.0);
            let worn = |m: &Map| -> Vec<u32> {
                m.players[&warrior]
                    .items
                    .iter()
                    .filter(|i| {
                        i.bag == BAG_PLAYER
                            && i.slot < 19
                            && i.fields.get_u32(ITEM_FIELD_MAXDURABILITY) > 0
                    })
                    .map(|i| i.fields.get_u32(ITEM_FIELD_DURABILITY))
                    .collect()
            };
            let fresh = worn(m);
            // A Ritual of Doom's sacrifice spares it.
            spells::cast_triggered(m, kobold, RITUAL_OF_DOOM_SACRIFICE, Some(warrior));
            let sacrificed = (health(m, warrior), worn(m));
            crate::game::map::death::resurrect(m, warrior, 1.0);
            // Death Touch, an instant kill with no such word, wears it.
            spells::cast_triggered(m, kobold, DEATH_TOUCH, Some(warrior));
            let touched = (health(m, warrior), worn(m));
            (fresh, sacrificed, touched)
        })
        .await
        .unwrap();
    let (fresh, (dead, sacrificed), (dead_again, touched)) = out;
    // The starting sword, shield and trousers: 20 and 25 points, so dying
    // costs each two; a blow, one point of one, one time in two hundred.
    assert!(
        fresh.len() >= 2 && fresh.iter().all(|d| *d >= 20),
        "{fresh:?}"
    );
    let lost = |after: &[u32], before: &[u32]| -> Vec<u32> {
        after.iter().zip(before).map(|(a, b)| b - a).collect()
    };
    let spared = lost(&sacrificed, &fresh);
    assert_eq!(dead, 0, "sacrificed");
    assert!(
        spared.iter().sum::<u32>() <= 1,
        "the gear spared: {spared:?}"
    );
    let worn_down = lost(&touched, &sacrificed);
    assert_eq!(dead_again, 0, "touched");
    assert!(worn_down.iter().all(|l| *l >= 2), "and worn: {worn_down:?}");
}

const NET_O_MATIC_BACKFIRE: u32 = 13119;
const NET_BACKFIRE_MARKER: u32 = 13139;
const NET_ON_THROWER: u32 = 13138;

#[tokio::test]
async fn the_net_o_matics_backfire_nets_its_thrower_and_leaves_no_marker() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "P2A5");
    let gnome = c.enter("Parfive", Race::Gnome, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, gnome, 60);
            let kobold = dummy(m, gnome, 60, 8.0);
            spells::cast_triggered(m, gnome, NET_O_MATIC_BACKFIRE, Some(kobold));
            (
                has(m, kobold, NET_O_MATIC_BACKFIRE),
                has(m, gnome, NET_ON_THROWER),
                has(m, gnome, NET_BACKFIRE_MARKER),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        out,
        (true, true, false),
        "(the target netted, the thrower netted, the marker left)"
    );
}

const PREMEDITATION: u32 = 14183;
/// A builder's point (Ruthlessness's).
const RUTHLESSNESS: u32 = 14157;

#[tokio::test]
async fn premeditations_points_come_off_when_it_runs_out_unless_a_builder_kept_them() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "P2A6");
    let rogue = c.enter("Parsix", Race::Human, Class::Rogue).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, rogue, 60);
            let points = |m: &Map| {
                let p = &m.players[&rogue];
                (
                    p.caster.combo_points,
                    p.fields.get_byte(PLAYER_FIELD_BYTES, 1),
                )
            };
            let kobold = dummy(m, rogue, 60, 3.0);
            // One point of a builder's, then Premeditation's two.
            spells::cast_triggered(m, rogue, RUTHLESSNESS, Some(kobold));
            spells::cast_triggered(m, rogue, PREMEDITATION, Some(kobold));
            let held = (points(m), has(m, rogue, PREMEDITATION));
            m.now += Duration::from_secs(11);
            auras::update(m);
            let ran_out = (points(m), has(m, rogue, PREMEDITATION));
            // Premeditation, then a builder: the points are kept.
            spells::cast_triggered(m, rogue, PREMEDITATION, Some(kobold));
            spells::cast_triggered(m, rogue, RUTHLESSNESS, Some(kobold));
            let built = (points(m), has(m, rogue, PREMEDITATION));
            m.now += Duration::from_secs(11);
            auras::update(m);
            let kept = points(m);
            (held, ran_out, built, kept)
        })
        .await
        .unwrap();
    let (held, ran_out, built, kept) = out;
    assert_eq!(held, ((3, 3), true), "three points, two of them held");
    assert_eq!(ran_out, ((1, 1), false), "the two held go with the hold");
    assert_eq!(built, ((4, 4), false), "a builder keeps them for good");
    assert_eq!(kept, (4, 4));
}

const SILITHYST: u32 = 29519;

#[tokio::test]
async fn the_silithyst_carriers_pulse_keeps_them_contested() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "P2A7");
    let carrier = c.enter("Parseven", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let out = map
        .query(move |m| {
            ready(m, carrier, 60);
            spells::cast_triggered(m, carrier, SILITHYST, Some(carrier));
            let carrying = has(m, carrier, SILITHYST);
            // The contest from the pickup has lapsed.
            let p = m.players.get_mut(&carrier).unwrap();
            p.pvp.contested_until = None;
            p.fields.set_flag(
                PLAYER_FLAGS,
                crate::game::map::pvp::PLAYER_FLAGS_CONTESTED_PVP,
                false,
            );
            m.now += Duration::from_secs(15);
            auras::update(m);
            let now = m.now;
            let p = &m.players[&carrier];
            (
                carrying,
                p.fields.has_flag(
                    PLAYER_FLAGS,
                    crate::game::map::pvp::PLAYER_FLAGS_CONTESTED_PVP,
                ),
                p.pvp.contested_until.is_some_and(|t| t > now),
            )
        })
        .await
        .unwrap();
    assert_eq!(out, (true, true, true), "(carrying, flagged, contested)");
}
