//! Round 10's player on the real world: a talent reset's weapon skills,
//! Feline Swiftness's hidden passives, a skill step below the value, an
//! invalid lesson (Thistle Tea's `talents_dbc_test`, `form_passives_dbc_test`
//! and `teaching_dbc_test`).

use std::sync::LazyLock;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, auras, misc, skills, spells};
use crate::game::tests_p3_entity::on;

static DB: LazyLock<std::path::PathBuf> = LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

async fn player(race: Race, class: Class) -> Option<(Client, Guid, MapHandle)> {
    if !have_data() {
        return None;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let mut a = Client::connect(&realm, "A");
    let g = a.enter("Learner", race, class).await;
    let map = realm.registry.online(g).unwrap().map();
    Some((a, g, map))
}

fn skill(m: &Map, g: Guid, id: u32) -> Option<(u32, u32)> {
    let p = &m.players[&g];
    Some((skills::value(p, id)?, skills::max_value(p, id)?))
}

fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

/// Thistle Tea's "forgets shaman two-handed proficiencies while retaining
/// learned skill values for retraining", as mangos's 1.11 rule has it
/// (`UpdateSpellTrainedSkills`, `m_forgottenSkills`): a tauren shaman's
/// talent (16269) teaches Two-Handed Axes (197) and Maces (199) and their
/// skills (172, 160); a reset takes the spells and the skills, Staves (54)
/// staying, and the talent learned again brings the skills back at what
/// they had reached.
#[tokio::test]
#[ignore = "extended"]
async fn a_talent_reset_forgets_its_weapon_skills_and_a_retrain_brings_them_back() {
    let Some((_a, g, map)) = player(Race::Tauren, Class::Shaman).await else {
        return;
    };
    let seen = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, g, 60);
        for s in [16_269, 197, 199] {
            spells::learn(m, g, s);
        }
        let learned = (skill(m, g, 172), skill(m, g, 160));
        let p = m.players.get_mut(&g).unwrap();
        skills::set_value(p, 172, 245);
        skills::set_value(p, 160, 190);
        let staves = skill(m, g, 54);
        let data = m.realm.data.clone();
        let talents = misc::talent_spells(&m.players[&g], &data);
        misc::forget_spells(m, g, &talents);
        let p = &m.players[&g];
        let reset = (
            [16_269, 197, 199].map(|s| p.spells.contains(&s)),
            skill(m, g, 172),
            skill(m, g, 160),
            skill(m, g, 54),
        );
        for s in [16_269, 197, 199] {
            spells::learn(m, g, s);
        }
        (learned, staves, reset, (skill(m, g, 172), skill(m, g, 160)))
    })
    .await;
    let (learned, staves, reset, back) = seen;
    assert_eq!(learned, (Some((1, 300)), Some((1, 300))), "learned at 1");
    assert_eq!(reset.0, [false; 3], "the talent's spells go");
    assert_eq!((reset.1, reset.2), (None, None), "and their skills");
    assert_eq!(reset.3, staves, "Staves stays");
    assert_eq!(
        back,
        (Some((245, 300)), Some((190, 300))),
        "kept for a retrain"
    );
}

/// Thistle Tea's "hidden dodge survives indoors independently of speed and
/// follows rank and unlearning", as mangos's `addSpell` and `removeSpell`
/// take a talent rank's dependents: Feline Swiftness's first rank (17002)
/// brings its hidden dodge (24867), its second (24866) the next (24864),
/// which takes the first's place, in the spellbook and its auras; a reset
/// takes both.
#[tokio::test]
#[ignore = "extended"]
async fn feline_swiftness_swaps_its_hidden_passive_with_its_rank() {
    let Some((_a, g, map)) = player(Race::NightElf, Class::Druid).await else {
        return;
    };
    let seen = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, g, 60);
        spells::cast_triggered(m, g, 768, Some(g));
        let state = |m: &Map| {
            let p = &m.players[&g];
            (
                [17_002, 24_867, 24_866, 24_864].map(|s| p.spells.contains(&s)),
                [17_002, 24_867, 24_866, 24_864].map(|s| has(m, g, s)),
            )
        };
        spells::learn(m, g, 17_002);
        let first = state(m);
        spells::learn(m, g, 24_866);
        let second = state(m);
        let data = m.realm.data.clone();
        let talents = misc::talent_spells(&m.players[&g], &data);
        misc::forget_spells(m, g, &talents);
        (first, second, state(m))
    })
    .await;
    assert_eq!(seen.0.0, [true, true, false, false], "the first rank");
    assert!(seen.0.1[1], "its hidden dodge on");
    assert_eq!(
        seen.1.0,
        [false, false, true, true],
        "the second takes its place"
    );
    assert_eq!(&seen.1.1[..2], &[false; 2], "the first's auras gone");
    assert!(seen.1.1[3], "the second's hidden dodge on");
    assert_eq!(seen.2, ([false; 4], [false; 4]), "a reset takes both");
}

/// Thistle Tea's "skill-only effects set exact caps and survive later
/// unrelated learning", as mangos's `SetSkillStep` keeps the value it has:
/// Apprentice Cooking's step (1) on one at 125 sets the cap to 75 and
/// keeps the 125; learning Unending Breath after changes nothing of it.
#[tokio::test]
#[ignore = "extended"]
async fn a_lower_skill_step_keeps_the_value_over_its_cap() {
    let Some((_a, g, map)) = player(Race::Human, Class::Warlock).await else {
        return;
    };
    let seen = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, g, 35);
        spells::learn(m, g, 2550);
        spells::learn(m, g, 3102);
        skills::set_value(m.players.get_mut(&g).unwrap(), 185, 125);
        let step = |m: &Map| {
            let p = &m.players[&g];
            let slot = (0..128u16)
                .find(|i| {
                    p.fields
                        .get_u32(crate::game::fields::PLAYER_SKILL_INFO_1_1 + i * 3)
                        & 0xFFFF
                        == 185
                })
                .unwrap();
            p.fields
                .get_u32(crate::game::fields::PLAYER_SKILL_INFO_1_1 + slot * 3)
                >> 16
        };
        let before = (skill(m, g, 185), step(m));
        skills::learn_skill(m.players.get_mut(&g).unwrap(), 185, 1);
        let after = (skill(m, g, 185), step(m));
        spells::learn(m, g, 5697);
        (before, after, (skill(m, g, 185), step(m)))
    })
    .await;
    assert_eq!(seen.0, (Some((125, 150)), 2));
    assert_eq!(seen.1, (Some((125, 75)), 1), "the value kept over the cap");
    assert_eq!(seen.2, seen.1, "unrelated learning changes nothing");
}

/// Thistle Tea's "cancellation preserves the book and teaches nothing":
/// the Expert Cookbook's cast (19886) cancelled keeps the book and
/// teaches nothing; let finish, it teaches Expert Cooking and the book
/// goes.
#[tokio::test]
#[ignore = "extended"]
async fn a_cancelled_book_keeps_its_book_and_teaches_nothing() {
    let Some((_a, g, map)) = player(Race::Human, Class::Warrior).await else {
        return;
    };
    let seen = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, g, 35);
        spells::learn(m, g, 2550);
        spells::learn(m, g, 3102);
        skills::set_value(m.players.get_mut(&g).unwrap(), 185, 125);
        crate::game::map::inventory::add_item(m, g, 16_072, 1, false).unwrap();
        let book = m.players[&g]
            .items
            .iter()
            .find(|i| i.entry == 16_072)
            .unwrap()
            .guid;
        spells::cast(m, g, 19_886, spells::Targets::default(), Some(book));
        let began = m.players[&g].caster.casting.as_ref().map(|c| c.spell);
        spells::cancel(m, g);
        let cancelled = (
            m.players[&g].item_by_guid(book).is_some(),
            m.players[&g].spells.contains(&3413),
        );
        crate::game::tests_p3_spells_kit::fresh(m, g);
        let again = spells::cast(m, g, 19_886, spells::Targets::default(), Some(book));
        assert!(again, "cast again");
        // Its cast time (vmangos's 20 s), passed.
        crate::game::tests_p3_spells_kit::pass(m, 30_000);
        let done = (
            m.players[&g].item_by_guid(book).is_some(),
            m.players[&g].spells.contains(&3413),
        );
        (began, cancelled, done)
    })
    .await;
    assert_eq!(seen.0, Some(19_886), "a cast with a time");
    assert_eq!(
        seen.1,
        (true, false),
        "cancelled: the book kept, nothing taught"
    );
    assert_eq!(seen.2, (false, true), "finished: taught, the book gone");
}

/// Thistle Tea's "the registered packet removes spells and tracking, frees
/// the slot, and persists", as mangos's `HandleUnlearnSkillOpcode`
/// (`SetSkillStep(0)`, `removeSpell` of the skill's spells): Mining
/// unlearned takes its spells (each told), Find Minerals' tracking with
/// them, and the skill; learned again, it starts at 1.
#[tokio::test]
#[ignore = "extended"]
async fn mining_unlearned_takes_its_spells_and_tracking_and_starts_again_at_one() {
    let Some((mut a, g, map)) = player(Race::Dwarf, Class::Warrior).await else {
        return;
    };
    let learned = on(&map, move |m| {
        crate::game::tests_p3_combat::level(m, g, 20);
        spells::learn(m, g, 2575);
        spells::cast_triggered(m, g, 2580, Some(g));
        skills::set_value(m.players.get_mut(&g).unwrap(), 186, 40);
        (
            [2575, 2580, 2656].map(|s| m.players[&g].spells.contains(&s)),
            has(m, g, 2580),
            skill(m, g, 186),
        )
    })
    .await;
    assert_eq!(learned, ([true; 3], true, Some((40, 75))));
    a.clear();
    a.send(CMSG_UNLEARN_SKILL {
        skill: wow_world_messages::vanilla::Skill::Mining,
    })
    .await;
    let mut told = Vec::new();
    while told.len() < 3 {
        let s = a
            .wait_for("SMSG_REMOVED_SPELL", |m| match m {
                wow_world_messages::vanilla::opcodes::ServerOpcodeMessage::SMSG_REMOVED_SPELL(
                    r,
                ) => Some(u32::from(r.spell)),
                _ => None,
            })
            .await;
        told.push(s);
    }
    told.sort_unstable();
    assert_eq!(told, vec![2575, 2580, 2656], "each told");
    let after = on(&map, move |m| {
        let gone = (
            [2575, 2580, 2656].map(|s| m.players[&g].spells.contains(&s)),
            has(m, g, 2580),
            skill(m, g, 186),
        );
        spells::learn(m, g, 2575);
        (gone, skill(m, g, 186))
    })
    .await;
    assert_eq!(
        after.0,
        ([false; 3], false, None),
        "spells, tracking, skill"
    );
    assert_eq!(after.1, Some((1, 75)), "from 1 again");
}

/// Thistle Tea's "a recently bandaged player keeps the item and never
/// starts a channel" and "the recipient's live immunity prevents another
/// player's item consumption", as mangos's `CheckCast` refuses a helpful
/// spell to a target immune to its mechanic (TARGET_AURASTATE): a Linen
/// Bandage (1251) used on oneself, or on a friend, under Recently Bandaged
/// (11196) is refused before it channels, the bandage kept.
#[tokio::test]
#[ignore = "extended"]
async fn a_bandage_on_one_recently_bandaged_is_refused_and_kept() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let mut a = Client::connect(&realm, "A");
    let ga = a.enter("Medic", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "B");
    let gb = b.enter("Patient", Race::Human, Class::Warrior).await;
    let map = realm.registry.online(ga).unwrap().map();
    let at = on(&map, move |m| {
        let here = m.players[&ga].position();
        m.players.get_mut(&gb).unwrap().movement.position = here;
        spells::learn(m, ga, 3273);
        crate::game::map::inventory::add_item(m, ga, 1251, 3, false).unwrap();
        for g in [ga, gb] {
            spells::cast_triggered(m, g, 11_196, Some(g));
        }
        let p = &m.players[&ga];
        let i = p.items.iter().find(|i| i.entry == 1251).unwrap();
        (i.bag, i.slot)
    })
    .await;
    for target in [ga, gb] {
        a.clear();
        a.send(CMSG_USE_ITEM {
            bag_index: at.0,
            bag_slot: at.1,
            spell_index: 0,
            targets: SpellCastTargets {
                target_flags: SpellCastTargets_SpellCastTargetFlags::empty().set_unit(
                    SpellCastTargets_SpellCastTargetFlags_Unit::Unit {
                        unit_target: target,
                    },
                ),
            },
        })
        .await;
        let refused = a
            .wait_frame("SMSG_CAST_RESULT", spells::SMSG_CAST_RESULT, |f| {
                u32::from_le_bytes(f[0..4].try_into().unwrap()) == 746
            })
            .await;
        assert_eq!(
            refused[4..6],
            [2, spells::checks::fail::TARGET_AURASTATE],
            "{target:?}"
        );
        let after = on(&map, move |m| {
            (
                m.players[&ga].count_of(1251),
                m.players[&ga].caster.casting.is_some(),
            )
        })
        .await;
        assert_eq!(after, (3, false), "kept, no channel");
    }
}
