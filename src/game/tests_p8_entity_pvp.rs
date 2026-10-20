//! Round 8's entity tests of PvP flagging by spells, ported from Thistle
//! Tea's `test/game/world/entity/effect_resolver/pvp_test.exs`
//! (docs/parity/matrix.csv): a pet's spell flags its master as it lands, a
//! PvP-enabling spell flags both sides without a fight, and help in a
//! spell's second target slot flags its caster.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::data::GameData;
use crate::game::data::spells::{ATTR_EX_NO_THREAT, ATTR_EX3_PVP_ENABLING, SpellEffect};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness;
use crate::game::map::Map;
use crate::game::map::creature::Creature;
use crate::game::map::player::{UNIT_FLAG_IN_COMBAT, UNIT_FLAG_PVP};
use crate::game::map::pvp::{CONTESTED_FOR, PLAYER_FLAGS_CONTESTED_PVP};
use crate::game::map::spells::{self, Targets};
use crate::game::tests_pets::{WOLF, data, effect, enter, spell};

const CURSE: u32 = 9_901;
const BOLT: u32 = 9_902;
const ENABLING_MARK: u32 = 9_903;
const QUIET_MARK: u32 = 9_904;
const PARTY_WARD: u32 = 9_905;

fn world() -> GameData {
    let mut d = data();
    let none = SpellEffect::default();
    // A harmful aura, no damage, threat as it lands: a curse.
    let mut curse = effect(6, 0, 6);
    curse.aura = 4;
    let mut curse = spell(CURSE, "Curse", [curse, none, none]);
    curse.duration_index = 4;
    let bolt = spell(BOLT, "Bolt", [effect(2, 4, 6), none, none]);
    // Hunter's Mark's shape: harmful, no threat, PvP-enabling.
    let mut mark = effect(6, 0, 6);
    mark.aura = 4;
    let mut enabling = spell(ENABLING_MARK, "Mark", [mark, none, none]);
    enabling.attributes[1] |= ATTR_EX_NO_THREAT;
    enabling.attributes[3] |= ATTR_EX3_PVP_ENABLING;
    enabling.duration_index = 4;
    let mut quiet = enabling.clone();
    quiet.id = QUIET_MARK;
    quiet.attributes[3] &= !ATTR_EX3_PVP_ENABLING;
    // A ward on any unit, then the party around it (Thistle Tea's
    // `any_unit` and `party_around_target`, 25 and 37), of no threat.
    let mut ward = effect(6, 9, 25);
    ward.target_b = 37;
    ward.aura = 22;
    ward.misc_value = 4;
    let mut ward = spell(PARTY_WARD, "Party Ward", [ward, none, none]);
    ward.attributes[1] |= ATTR_EX_NO_THREAT;
    ward.duration_index = 4;
    for s in [curse, bolt, enabling, quiet, ward] {
        d.spells.spells.insert(s.id, s);
    }
    d
}

fn flagged(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP))
}

fn fighting(m: &Map, g: Guid) -> bool {
    m.unit_fields(g)
        .is_some_and(|f| f.has_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT))
}

fn contested(m: &Map, g: Guid) -> bool {
    m.players[&g]
        .fields
        .has_flag(PLAYER_FLAGS, PLAYER_FLAGS_CONTESTED_PVP)
}

/// Everyone unflagged, out of the fight and uncontested, their auras off.
fn reset(m: &mut Map, players: &[Guid]) {
    for &g in players {
        let p = m.players.get_mut(&g).unwrap();
        p.pvp = Default::default();
        p.combat = Default::default();
        p.fields.set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, false);
        p.fields
            .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_IN_COMBAT, false);
        p.fields
            .set_flag(PLAYER_FLAGS, PLAYER_FLAGS_CONTESTED_PVP, false);
        for s in [CURSE, ENABLING_MARK, QUIET_MARK, PARTY_WARD] {
            crate::game::map::auras::remove_spell(m, g, s, None);
        }
    }
}

/// `g` set down 3 yards before `by` (who faces east).
fn beside(m: &mut Map, g: Guid, by: Guid) {
    let mut at = m.players[&by].position();
    at.x += 3.0;
    m.players.get_mut(&g).unwrap().movement.position = at;
    m.players.get_mut(&by).unwrap().movement.orientation = 0.0;
}

fn flag(m: &mut Map, g: Guid) {
    m.unit_fields_mut(g)
        .unwrap()
        .set_flag(UNIT_FIELD_FLAGS, UNIT_FLAG_PVP, true);
}

/// A wolf of `owner`'s at their side.
fn pet_of(m: &mut Map, owner: Guid, counter: u32) -> Guid {
    let t = m.realm.data.creature_template(WOLF).cloned().unwrap();
    let at = m.players[&owner].position();
    let mut pet = Creature::new(guid::unit(WOLF, counter), &t, 1, at, 0.0);
    pet.owner = Some(owner);
    m.add_creature(pet)
}

/// A pet's spell that lands on another player's flagged side flags its
/// master and contests him, as his own would (mangos's
/// `CasterHitTargetWithSpell`: `SetInCombatWithVictim` by the pet's
/// `GetBeneficiaryPlayer`; Thistle Tea's "launch flags only the attacking
/// owner"): a curse on a flagged player, a bolt on a flagged player's
/// pet. The one cursed keeps his flag, uncontested. A pet's spell on one
/// unflagged flags no one. As mangos has it, the master is flagged as the
/// spell lands, not as it leaves, and is in his pet's fight.
#[tokio::test]
async fn a_pets_spell_flags_and_contests_its_master_as_it_lands() {
    let realm = harness::realm_with(world());
    let (_a, alice, map) = enter(&realm, "A", "Alice", Race::Dwarf, Class::Hunter).await;
    let (_g, grom, _) = enter(&realm, "G", "Grom", Race::Undead, Class::Warrior).await;
    let seen = map
        .query(move |m| {
            beside(m, grom, alice);
            let pet = pet_of(m, alice, 91);
            let his = pet_of(m, grom, 92);
            let cast = |m: &mut Map, spell: u32, at: Guid| {
                spells::cast_now(
                    m,
                    pet,
                    spell,
                    Targets {
                        unit: Some(at),
                        ..Default::default()
                    },
                );
            };
            let mut seen = Vec::new();
            reset(m, &[alice, grom]);
            cast(m, CURSE, grom);
            seen.push(("unflagged", flagged(m, alice), contested(m, alice)));
            reset(m, &[alice, grom]);
            flag(m, grom);
            cast(m, CURSE, grom);
            seen.push(("curse", flagged(m, alice), contested(m, alice)));
            seen.push(("in the fight", fighting(m, alice), fighting(m, grom)));
            seen.push(("him", flagged(m, grom), contested(m, grom)));
            reset(m, &[alice, grom]);
            flag(m, grom);
            cast(m, BOLT, his);
            seen.push(("his pet", flagged(m, alice), contested(m, alice)));
            seen
        })
        .await
        .unwrap();
    assert_eq!(
        seen,
        vec![
            ("unflagged", false, false),
            ("curse", true, true),
            ("in the fight", true, true),
            ("him", true, false),
            ("his pet", true, true),
        ]
    );
}

/// A PvP-enabling spell of no threat (Hunter's Mark, Earthbind) landing on
/// a flagged player flags and contests its caster, and the one it lands on
/// has his flag kept up by a flagged caster's, with no fight for either
/// (mangos's `SetOutOfCombatWithAggressor` and `SetOutOfCombatWithVictim`,
/// touch only; Thistle Tea's "combat-free hostility flags both players
/// without starting combat"). The contest is the usual 30 seconds. The
/// same spell without the attribute flags its caster (mangos's
/// `FillTargetMap`) and contests no one.
#[tokio::test]
async fn a_pvp_enabling_spell_flags_both_sides_without_a_fight() {
    let realm = harness::realm_with(world());
    let (_a, alice, map) = enter(&realm, "A", "Alice", Race::Dwarf, Class::Hunter).await;
    let (_g, grom, _) = enter(&realm, "G", "Grom", Race::Undead, Class::Warrior).await;
    let seen = map
        .query(move |m| {
            let cast = |m: &mut Map, from: Guid, spell: u32, at: Guid| {
                spells::cast_now(
                    m,
                    from,
                    spell,
                    Targets {
                        unit: Some(at),
                        ..Default::default()
                    },
                );
            };
            beside(m, grom, alice);
            let now = m.now;
            let mut seen = Vec::new();
            reset(m, &[alice, grom]);
            flag(m, grom);
            cast(m, alice, ENABLING_MARK, grom);
            seen.push((
                "her",
                flagged(m, alice),
                contested(m, alice),
                fighting(m, alice),
            ));
            seen.push((
                "for 30 s",
                m.players[&alice].pvp.contested_until == Some(now + CONTESTED_FOR),
                false,
                false,
            ));
            // His flag is kept up by hers, which the mark raised first.
            seen.push((
                "him",
                flagged(m, grom),
                contested(m, grom),
                fighting(m, grom),
            ));
            seen.push((
                "his linger",
                m.players[&grom].combat.last_pvp == Some(now),
                false,
                false,
            ));
            reset(m, &[alice, grom]);
            flag(m, grom);
            cast(m, alice, QUIET_MARK, grom);
            seen.push((
                "without",
                flagged(m, alice),
                contested(m, alice),
                fighting(m, alice),
            ));
            seen
        })
        .await
        .unwrap();
    assert_eq!(
        seen,
        vec![
            ("her", true, true, false),
            ("for 30 s", true, false, false),
            ("him", true, false, false),
            ("his linger", true, false, false),
            ("without", true, false, false),
        ]
    );
}

/// Help chosen by a spell's second target slot (any unit, then the party
/// around it) flags its caster when it lands on a flagged player, as the
/// first slot's would: every landed target counts (mangos's
/// `FillTargetMap`; Thistle Tea's "party assistance in the second implicit
/// target slot flags a PvP caster"). Of no threat, it puts her in no fight.
#[tokio::test]
async fn help_through_a_spells_second_target_slot_flags_its_caster() {
    let realm = harness::realm_with(world());
    let (_a, alice, map) = enter(&realm, "A", "Alice", Race::Human, Class::Priest).await;
    let (_b, bob, _) = enter(&realm, "B", "Bob", Race::Human, Class::Warrior).await;
    let seen = map
        .query(move |m| {
            reset(m, &[alice, bob]);
            flag(m, bob);
            spells::cast_now(
                m,
                alice,
                PARTY_WARD,
                Targets {
                    unit: Some(bob),
                    ..Default::default()
                },
            );
            (
                m.players[&bob].auras.has_spell(PARTY_WARD),
                flagged(m, alice),
                fighting(m, alice),
                contested(m, alice),
            )
        })
        .await
        .unwrap();
    assert_eq!(seen, (true, true, false, false));
}
