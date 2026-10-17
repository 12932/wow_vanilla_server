//! Bugs found while mapping wvs's tests against Thistle Tea's, the second
//! batch: spells paid in health, creatures' costs, dazes, triggered areas
//! about the unit the aura is on, teleports within a copy, Relentless
//! Strikes, health regeneration in combat, the global cooldown, ranks'
//! proc rules, wide family masks, 1.12's modifier operations, smaller
//! targeting and casting rules, who may attack whom, the melee and ranged
//! damage taken auras, Mana Shield's mana, a DoT's threat and a proc's
//! chance.

use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells::{self, Targets};
use crate::game::map::{Map, MapKey, auras};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| crate::game::harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

fn realm() -> std::sync::Arc<crate::game::Realm> {
    harness::realm_with(crate::game::data::load::load(DB.as_path()))
}

/// A creature of `entry` at `level`, `yards` in front of `g`, hardy and
/// without armor.
fn spawn(m: &mut Map, g: Guid, entry: u32, level: u8, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let template = data
        .creature_template(entry)
        .cloned()
        .expect("the creature");
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

/// `g` at `level`, knowing `learn`, with the powers a character of their
/// class has (none filled in for the test).
fn level(m: &mut Map, g: Guid, level: u8, learn: &[u32]) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, level);
    for s in learn {
        spells::learn(m, g, *s);
    }
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

fn apply(m: &mut Map, caster: Guid, target: Guid, id: u32) {
    let data = m.realm.data.clone();
    let s = data.spells.get(id).unwrap().clone();
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].effect == 6 && s.effects[*i].aura != 0)
        .map(|i| (i, spells::points(&s, i, 60)))
        .collect();
    let _ = auras::apply_spell(m, caster, target, &s, &effects, auras::Context::default());
}

/// The result of `spell`'s cast as the client hears it: none for a cast
/// that went, else the failure.
async fn cast_result(c: &mut Client, spell: u32) -> Option<u8> {
    let b = c
        .wait_frame(
            "SMSG_CAST_RESULT",
            crate::game::map::spells::SMSG_CAST_RESULT,
            |b| b[..4] == spell.to_le_bytes(),
        )
        .await;
    (b[4] != 0).then(|| b[5])
}

const BLOODRAGE: u32 = 2687;

const HEALTH_FUNNEL: u32 = 755;

#[tokio::test]
async fn a_health_cost_is_paid_in_health() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "BLOODRAGE");
    let w = c.enter("Ragew", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let before = map
        .query(move |m| {
            level(m, w, 60, &[BLOODRAGE]);
            let f = &m.players[&w].fields;
            (
                f.get_u32(UNIT_FIELD_HEALTH),
                f.get_u32(UNIT_FIELD_BASE_HEALTH),
                f.get_u32(UNIT_FIELD_POWER1 + 1),
            )
        })
        .await
        .unwrap();
    c.send(CMSG_CAST_SPELL {
        spell: BLOODRAGE,
        targets: spells::wire_targets(&Targets::default()),
    })
    .await;
    assert_eq!(cast_result(&mut c, BLOODRAGE).await, None, "Bloodrage goes");
    let after = map
        .query(move |m| {
            let f = &m.players[&w].fields;
            (
                f.get_u32(UNIT_FIELD_HEALTH),
                f.get_u32(UNIT_FIELD_POWER1 + 1),
            )
        })
        .await
        .unwrap();
    // A fifth of base health (less what came back since), and its rage.
    let paid = before.0 - after.0;
    assert!(
        paid <= before.1 / 5 && paid + 100 >= before.1 / 5,
        "paid in health: {paid} of {}",
        before.1
    );
    assert!(after.1 > before.2, "and the rage came");
    // Health Funnel needs more health than it costs.
    let refused = map
        .query(move |m| {
            let s = m.realm.data.spells.get(HEALTH_FUNNEL).unwrap().clone();
            let cost = spells::power_cost(m, w, &s);
            m.players
                .get_mut(&w)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, cost);
            let at_cost = spells::unpayable(m, w, &s, cost);
            m.players
                .get_mut(&w)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_HEALTH, cost + 1);
            (cost, at_cost, spells::unpayable(m, w, &s, cost))
        })
        .await
        .unwrap();
    assert_eq!(
        refused,
        (11, Some(spells::fail::CASTER_AURASTATE), None),
        "health above the cost"
    );
}

#[tokio::test]
async fn a_creature_casts_its_list_without_rage_or_energy() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "SHOUTING");
    let p = c.enter("Shoutee", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let shouted = map
        .query(move |m| {
            level(m, p, 30, &[]);
            // A Splinter Fist Warrior's list is Battle Shout, for rage.
            let orc = spawn(m, p, 212, 30, 5.0);
            spells::lists::tick(m, orc, p);
            m.now += Duration::from_secs(4);
            spells::lists::tick(m, orc, p);
            let shout = has(m, orc, 5242);
            // Nor does one without mana pay mana.
            let s = m.realm.data.spells.get(5242).unwrap().clone();
            let mut mana_spell = s.clone();
            mana_spell.power_type = 0;
            let without_mana = spells::unpayable(m, orc, &mana_spell, 100);
            (shout, without_mana)
        })
        .await
        .unwrap();
    assert_eq!(shouted, (true, None), "Battle Shout goes, for nothing");
}

#[tokio::test]
async fn dazed_is_no_exclusive_snare() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "DAZEDBUG");
    let p = c.enter("Dazedp", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let both = map
        .query(move |m| {
            level(m, p, 30, &[]);
            let kobold = spawn(m, p, 6, 30, 3.0);
            // Hamstring's snare, then a daze from the same unit.
            apply(m, kobold, p, 1715);
            apply(m, kobold, p, 1604);
            (has(m, p, 1715), has(m, p, 1604))
        })
        .await
        .unwrap();
    assert_eq!(both, (true, true), "the daze and the snare stand together");
}
