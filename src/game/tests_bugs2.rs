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
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
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

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
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

async fn group(leader: &mut Client, member: &mut Client, name: &str) {
    leader.send(CMSG_GROUP_INVITE { name: name.into() }).await;
    member
        .wait_for("SMSG_GROUP_INVITE", |m| {
            matches!(m, ServerOpcodeMessage::SMSG_GROUP_INVITE(_)).then_some(())
        })
        .await;
    member.send(CMSG_GROUP_ACCEPT {}).await;
    leader
        .wait_for("SMSG_GROUP_LIST", |m| match m {
            ServerOpcodeMessage::SMSG_GROUP_LIST(l) => (!l.members.is_empty()).then_some(()),
            _ => None,
        })
        .await;
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

#[tokio::test]
async fn hellfire_burns_about_the_warlock() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "HELLFIRE");
    let w = c.enter("Hellw", Race::Human, Class::Warlock).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let hurt = map
        .query(move |m| {
            level(m, w, 60, &[]);
            let near = spawn(m, w, 6, 60, 4.0);
            let far = spawn(m, w, 6, 60, 30.0);
            // The warlock has the far one selected.
            m.players
                .get_mut(&w)
                .unwrap()
                .fields
                .set_guid(UNIT_FIELD_TARGET, far);
            apply(m, w, w, 1949);
            m.now += Duration::from_millis(1100);
            auras::update(m);
            (health(m, near) < 1_000_000, health(m, far) < 1_000_000)
        })
        .await
        .unwrap();
    assert_eq!(hurt, (true, false), "the one beside the warlock burns");
}

#[tokio::test]
async fn plague_spreads_through_the_raid_it_is_on() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut a = Client::connect(&realm, "PLAGUEA");
    let ga = a.enter("Plaguea", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "PLAGUEB");
    let gb = b.enter("Plagueb", Race::Human, Class::Warrior).await;
    group(&mut a, &mut b, "Plagueb").await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let hurt = map
        .query(move |m| {
            level(m, ga, 60, &[]);
            level(m, gb, 60, &[]);
            let at = m.players[&ga].position();
            m.players.get_mut(&gb).unwrap().movement.position = Vector3d {
                x: at.x + 2.0,
                ..at
            };
            // An Anubisath Defender, well away, plagues the first.
            let anubisath = spawn(m, ga, 15277, 60, 25.0);
            apply(m, anubisath, ga, 26556);
            let full = (health(m, ga), health(m, gb));
            m.now += Duration::from_millis(3100);
            auras::update(m);
            (
                health(m, ga) < full.0,
                health(m, gb) < full.1,
                health(m, anubisath) == 1_000_000,
            )
        })
        .await
        .unwrap();
    assert_eq!(hurt, (true, true, true), "the plagued and the one beside");
}

#[tokio::test]
async fn a_teleport_within_a_copy_stays_in_it() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "BWLORB");
    let p = c.enter("Orbp", Race::Human, Class::Warrior).await;
    let from = realm.registry.online(p).unwrap().map();
    from.call(move |m| {
        m.teleport(
            p,
            MapKey::world(469),
            Vector3d {
                x: -7600.0,
                y: -1100.0,
                z: 400.0,
            },
            0.0,
        )
    });
    c.wait_for("SMSG_NEW_WORLD", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_NEW_WORLD(_)).then_some(())
    })
    .await;
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    c.sees(p).await;
    let copy = realm.registry.online(p).unwrap().map();
    assert_eq!(copy.key.map, 469);
    assert_ne!(copy.key.instance, 0, "in a copy");
    // The orb's teleport (23460) to the top of the lair.
    let at = copy
        .query(move |m| {
            spells::cast_triggered(m, p, 23460, Some(p));
            m.players.get(&p).map(|p| p.position())
        })
        .await
        .unwrap();
    let at = at.expect("still in the copy");
    assert!(
        (at.x + 7666.46).abs() < 0.1 && (at.y + 1103.08).abs() < 0.1,
        "at the orb's place: {at:?}"
    );
}

const RELENTLESS_STRIKES: u32 = 14179;

const EVISCERATE: u32 = 2098;

#[tokio::test]
async fn relentless_strikes_goes_by_the_combo_points() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "RELENTLESS");
    let r = c.enter("Relentr", Race::Human, Class::Rogue).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let energy = map
        .query(move |m| {
            level(m, r, 60, &[RELENTLESS_STRIKES, EVISCERATE]);
            let kobold = spawn(m, r, 6, 60, 2.0);
            if let Some(c) = m.caster_mut(r) {
                c.combo_points = 5;
                c.combo_target = kobold;
            }
            m.players
                .get_mut(&r)
                .unwrap()
                .fields
                .set_u32(UNIT_FIELD_POWER1 + 3, 100);
            let went = spells::cast(
                m,
                r,
                EVISCERATE,
                Targets {
                    unit: Some(kobold),
                    ..Default::default()
                },
                None,
            );
            (went, m.players[&r].fields.get_u32(UNIT_FIELD_POWER1 + 3))
        })
        .await
        .unwrap();
    // Five points are a sure 25 energy back, of the 35 Eviscerate cost.
    assert_eq!(energy, (true, 90));
}

#[tokio::test]
async fn health_regeneration_auras_give_their_health_in_and_out_of_combat() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "TROLLBLOOD");
    let p = c.enter("Trollb", Race::Human, Class::Warrior).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let gained = map
        .query(move |m| {
            level(m, p, 60, &[]);
            // Major Troll's Blood: 12 health every five seconds.
            apply(m, p, p, 3223);
            let mut gained = Vec::new();
            for fighting in [true, false] {
                let pl = m.players.get_mut(&p).unwrap();
                pl.fields.set_flag(
                    UNIT_FIELD_FLAGS,
                    crate::game::map::player::UNIT_FLAG_IN_COMBAT,
                    fighting,
                );
                // No spirit, for none of its health.
                pl.fields.set_u32(UNIT_FIELD_STAT0 + 4, 0);
                pl.fields.set_u32(UNIT_FIELD_HEALTH, 100);
                for _ in 0..10 {
                    m.now += crate::game::map::stats::REGEN_INTERVAL;
                    crate::game::map::stats::regen(m);
                }
                gained.push(health(m, p) - 100);
            }
            gained
        })
        .await
        .unwrap();
    // Twenty seconds: four times 12, in a fight or out of one.
    for g in &gained {
        assert!((47..=48).contains(g), "gained {gained:?}");
    }
}

#[tokio::test]
async fn a_modifier_touches_only_its_mask_past_32_bits() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "SHADOWMASTERY");
    let w = c.enter("Shadowm", Race::Human, Class::Warlock).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let pct = map
        .query(move |m| {
            level(m, w, 60, &[18275]);
            let data = m.realm.data.clone();
            let all = |m: &Map, id: u32| {
                spells::mods::for_spell(m, w, data.spells.get(id).unwrap()).pct
                    [spells::mods::OP_ALL_EFFECTS]
            };
            // Shadow Mastery's 10%: its shadow spells' (Siphon Life's past
            // the 32nd bit), not Immolate's.
            (all(m, 18265), all(m, 348))
        })
        .await
        .unwrap();
    assert_eq!(pct, (10, 0));
}

#[tokio::test]
async fn permafrost_slows_frostbolt_more() {
    if !have_data() {
        return;
    }
    let realm = realm();
    let mut c = Client::connect(&realm, "PERMAFROST");
    let mage = c.enter("Permaf", Race::Human, Class::Mage).await;
    let map = realm.maps.get(&realm, MapKey::world(0));
    let slow = map
        .query(move |m| {
            level(m, mage, 60, &[11175]);
            let kobold = spawn(m, mage, 6, 60, 10.0);
            spells::cast_triggered(m, mage, 116, Some(kobold));
            auras::auras(m, kobold).and_then(|a| {
                a.holders
                    .iter()
                    .find(|h| h.spell == 116)
                    .and_then(|h| h.effects.iter().find(|x| x.kind == 33))
                    .map(|x| x.amount)
            })
        })
        .await
        .unwrap();
    // Frostbolt's 40%, and Permafrost's 4 more.
    assert_eq!(slow, Some(-44));
}
