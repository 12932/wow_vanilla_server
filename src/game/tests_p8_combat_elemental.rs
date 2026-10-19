//! Round 8's combat tests, an elemental's blows (Thistle Tea's
//! `elemental_combat_test.exs`): a blow of a magic school is resisted in
//! part before a ward of its school soaks it, a caster's penetration takes
//! the resistance off, and a flat cut to the school takes at most half.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::*;
use crate::game::fields::*;
use crate::game::harness::Client;
use crate::game::map::auras::{
    AURA_MOD_DAMAGE_TAKEN, AURA_MOD_TARGET_RESISTANCE, AURA_SCHOOL_ABSORB,
};
use crate::game::map::combat::{self, HITINFO_ABSORB, HITINFO_RESIST, Outcome};
use crate::game::map::{Map, MapHandle};
use crate::game::tests_p3_combat::{self as p3, BRUTE, HUMANS};

/// A creature whose blows are fire.
const FIRE_ELEMENTAL: u32 = 3760;
const FIRE: u32 = 4;
const FIRE_INDEX: u16 = 2;

fn data() -> GameData {
    let mut d = p3::data();
    let mut t = p3::template(FIRE_ELEMENTAL, 4);
    t.damage_school = 2;
    t.faction = HUMANS;
    d.creature_templates.insert(FIRE_ELEMENTAL, t);
    d
}

fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

/// A fire elemental and a brute of level 50 before Alice, the brute's
/// health back to full; the elemental's blow at the brute, its partial
/// resist rolled at `roll`: what it took off.
fn blow(m: &mut Map, elemental: Guid, brute: Guid, roll: i32) -> u32 {
    if let Some(f) = m.unit_fields_mut(brute) {
        f.set_u32(UNIT_FIELD_HEALTH, 100_000);
    }
    m.spells.rolls = Some(true);
    m.spells.queued_rolls.lock().unwrap().push_back(roll);
    p3::forced_swing(m, elemental, brute, Outcome::Hit);
    m.spells.rolls = None;
    m.spells.queued_rolls.lock().unwrap().clear();
    100_000 - health(m, brute)
}

/// The blow's hit info, damage, absorbed and resisted, as Alice is told
/// (`SMSG_ATTACKERSTATEUPDATE`).
async fn told(a: &mut Client) -> (u32, u32, u32, u32) {
    let b = a
        .wait_frame("the blow", combat::SMSG_ATTACKERSTATEUPDATE, |_| true)
        .await;
    let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let mut i = 4;
    for _ in 0..2 {
        let mask = b[i];
        i += 1 + mask.count_ones() as usize;
    }
    // Total, the count, then the school's: school, float, damage, absorbed,
    // resisted.
    let total = u32_at(i);
    let at = i + 5;
    (u32_at(0), total, u32_at(at + 12), u32_at(at + 16))
}

type Setup = (
    std::sync::Arc<crate::game::Realm>,
    Client,
    MapHandle,
    Guid,
    Guid,
);

async fn setup() -> Setup {
    let (realm, mut a, ga, map) = p3::setup_with(data(), Class::Warrior).await;
    let (elemental, brute) = map
        .query(move |m| {
            p3::level(m, ga, 60);
            let brute = p3::spawn(m, ga, BRUTE, 50, 3.0);
            let elemental = p3::spawn(m, ga, FIRE_ELEMENTAL, 50, 4.0);
            crate::game::map::visibility::refresh_dirty(m);
            (elemental, brute)
        })
        .await
        .unwrap();
    a.sees(brute).await;
    a.sees(elemental).await;
    a.clear();
    (realm, a, map, elemental, brute)
}

/// Thistle Tea's "resistance applies before the matching ward and reports
/// both amounts", as mangos's `CalculateMeleeDamage` has it
/// (`CalculateDamageAbsorbAndResist` for each of a blow's schools): an
/// elemental's blow of a hundred fire at a brute of 250 fire resistance,
/// its level, is resisted three quarters at the luckiest roll, and a fire
/// ward of ten soaks ten of the rest; the log says both.
#[tokio::test]
async fn an_elemental_blow_is_resisted_in_part_before_its_schools_ward() {
    let (_realm, mut a, map, elemental, brute) = setup().await;
    let plain = map
        .query(move |m| blow(m, elemental, brute, 0))
        .await
        .unwrap();
    assert_eq!(plain, 100, "a hundred, no resistance");
    let (info, total, absorbed, resisted) = told(&mut a).await;
    assert_eq!(
        (
            info & (HITINFO_ABSORB | HITINFO_RESIST),
            total,
            absorbed,
            resisted
        ),
        (0, 100, 0, 0)
    );
    a.clear();
    let warded = map
        .query(move |m| {
            p3::aura_of(m, brute, 96_400, AURA_SCHOOL_ABSORB, FIRE as i32, 10);
            m.unit_fields_mut(brute)
                .unwrap()
                .set_u32(UNIT_FIELD_RESISTANCES + FIRE_INDEX, 250);
            blow(m, elemental, brute, 0)
        })
        .await
        .unwrap();
    assert_eq!(warded, 15, "75 resisted, 10 soaked");
    let (info, total, absorbed, resisted) = told(&mut a).await;
    assert_eq!(
        info & (HITINFO_ABSORB | HITINFO_RESIST),
        HITINFO_ABSORB | HITINFO_RESIST
    );
    assert_eq!((total, absorbed, resisted), (15, 10, 75));
}

/// Thistle Tea's "penetration removes school resistance without changing
/// the target's stat": the elemental's 250 fire penetration (its
/// `SPELL_AURA_MOD_TARGET_RESISTANCE`) leaves the brute nothing to resist
/// with, and the brute's 250 as they were.
#[tokio::test]
async fn an_elementals_penetration_takes_off_the_resistance_its_blow_meets() {
    let (_realm, _a, map, elemental, brute) = setup().await;
    let r = map
        .query(move |m| {
            m.unit_fields_mut(brute)
                .unwrap()
                .set_u32(UNIT_FIELD_RESISTANCES + FIRE_INDEX, 250);
            let resisted = blow(m, elemental, brute, 0);
            p3::aura_of(
                m,
                elemental,
                96_401,
                AURA_MOD_TARGET_RESISTANCE,
                FIRE as i32,
                -250,
            );
            let dealt = blow(m, elemental, brute, 0);
            let kept = m
                .unit_fields(brute)
                .unwrap()
                .get_u32(UNIT_FIELD_RESISTANCES + FIRE_INDEX);
            (resisted, dealt, kept)
        })
        .await
        .unwrap();
    assert_eq!(
        r,
        (25, 100, 250),
        "(taken unpierced, taken pierced, the brute's resistance)"
    );
}

/// Thistle Tea's "Dampen Magic cannot reduce an elemental swing below
/// half": a cut of a thousand to every magic school takes only half of
/// the elemental's hundred, and a twenty more to fire adds twenty.
#[tokio::test]
async fn a_flat_cut_to_a_magic_school_takes_at_most_half_an_elementals_blow() {
    let (_realm, _a, map, elemental, brute) = setup().await;
    let r = map
        .query(move |m| {
            p3::aura_of(m, brute, 96_402, AURA_MOD_DAMAGE_TAKEN, 126, -1000);
            let dampened = blow(m, elemental, brute, 9_999);
            crate::game::map::auras::remove_spell(m, brute, 96_402, None);
            p3::aura_of(m, brute, 96_403, AURA_MOD_DAMAGE_TAKEN, FIRE as i32, 20);
            (dampened, blow(m, elemental, brute, 9_999))
        })
        .await
        .unwrap();
    assert_eq!(r, (50, 120), "(dampened, amplified)");
}
