//! Readying a character with GM commands, as peon's raid does each bot:
//! `.character level` and `.gear` leave it at full health and mana, and
//! `.character spells` teaches it what its class's trainers would. On a
//! small world of a trainer's spells, and on the real data in the
//! extended suite.

use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::spells::{SkillAbility, SpellEffect, SpellRank, SpellTemplate};
use crate::game::data::{CreatureTemplate, GameData, ItemTemplate};
use crate::game::fields::*;
use crate::game::harness::{self, Client};
use crate::game::map::spells::EFFECT_LEARN_SPELL;
use crate::game::map::{MapHandle, MapKey};
use crate::game::tests_p3_entity::on;

/// A cloth chest of level 58: forty stamina and thirty intellect.
const ROBE: u32 = 973_001;
/// A warrior's class trainer, a hunter's pet trainer and a weapon master.
const TRAINER: u32 = 973_010;
const PET_TRAINER: u32 = 973_011;
const WEAPON_MASTER: u32 = 973_012;
/// A strike's three ranks, at levels 4, 10 and 30.
const STRIKE: [u32; 3] = [973_101, 973_102, 973_103];
/// A shout that wants a stance known, the trainer teaching both, the
/// stance at a later level than the shout.
const SHOUT: u32 = 973_104;
const STANCE: u32 = 973_105;
/// A talent's first rank (no trainer's) and its second (the trainer's).
const TALENT: [u32; 2] = [973_106, 973_107];
/// A mage's spell on the warrior's trainer.
const MAGES: u32 = 973_108;
/// The pet trainer's and the weapon master's.
const PETS: u32 = 973_109;
const AXES: u32 = 973_110;
/// The spells the trainers cast to teach those: each learns one.
const TEACHES: u32 = 10_000;

fn world() -> GameData {
    use crate::game::data::npc::TrainerSpell;
    let mut d = crate::game::tests_p3_combat::data();
    d.items.insert(
        ROBE,
        ItemTemplate {
            entry: ROBE,
            class: 4,
            subclass: 1,
            name: "Robe".into(),
            display_id: 1000,
            inventory_type: 5,
            quality: 3,
            required_level: 58,
            allowable_class: -1,
            allowable_race: -1,
            stats: [
                (7, 40),
                (5, 30),
                (0, 0),
                (0, 0),
                (0, 0),
                (0, 0),
                (0, 0),
                (0, 0),
                (0, 0),
                (0, 0),
            ],
            stackable: 1,
            ..Default::default()
        },
    );
    let trainer = |entry, trainer_type, trainer_class| CreatureTemplate {
        trainer_type,
        trainer_class,
        npc_flags: 0x10,
        ..crate::game::tests_p3_combat::template(entry, 7)
    };
    d.creature_templates.insert(TRAINER, trainer(TRAINER, 0, 1));
    d.creature_templates
        .insert(PET_TRAINER, trainer(PET_TRAINER, 3, 1));
    d.creature_templates
        .insert(WEAPON_MASTER, trainer(WEAPON_MASTER, 2, 0));
    let learned = [
        STRIKE[0], STRIKE[1], STRIKE[2], SHOUT, STANCE, TALENT[0], TALENT[1], MAGES, PETS, AXES,
    ];
    for id in learned {
        let spell = |id: u32, name: String| SpellTemplate {
            id,
            name,
            equipped_item_class: -1,
            ..Default::default()
        };
        d.spells.spells.insert(id, spell(id, format!("Spell {id}")));
        let mut teach = spell(TEACHES + id, format!("Teach {id}"));
        teach.effects[0] = SpellEffect {
            effect: EFFECT_LEARN_SPELL,
            trigger_spell: id,
            ..Default::default()
        };
        d.spells.spells.insert(TEACHES + id, teach);
    }
    let rank = |first, rank, prev, req| SpellRank {
        prev,
        first,
        rank,
        req,
    };
    d.spells.ranks.extend([
        (STRIKE[0], rank(STRIKE[0], 1, 0, 0)),
        (STRIKE[1], rank(STRIKE[0], 2, STRIKE[0], 0)),
        (STRIKE[2], rank(STRIKE[0], 3, STRIKE[1], 0)),
        (SHOUT, rank(SHOUT, 1, 0, STANCE)),
        (TALENT[1], rank(TALENT[0], 2, TALENT[0], 0)),
    ]);
    d.spells.skill_abilities.insert(
        MAGES,
        vec![SkillAbility {
            skill: 6,
            spell: MAGES,
            class_mask: 0x80,
            ..Default::default()
        }],
    );
    let teach = |spell: u32, required_level| TrainerSpell {
        spell: TEACHES + spell,
        cost: 100,
        required_skill: 0,
        required_skill_value: 0,
        required_level,
    };
    d.npc.trainers.insert(
        TRAINER,
        vec![
            teach(STRIKE[2], 30),
            teach(SHOUT, 6),
            teach(STRIKE[1], 10),
            teach(STANCE, 8),
            teach(STRIKE[0], 4),
            teach(TALENT[1], 10),
            teach(MAGES, 1),
        ],
    );
    d.npc.trainers.insert(PET_TRAINER, vec![teach(PETS, 1)]);
    d.npc.trainers.insert(WEAPON_MASTER, vec![teach(AXES, 1)]);
    d
}

async fn player(realm: &Arc<Realm>, account: &str, class: Class) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(account, Race::Human, class).await;
    let map = realm.maps.get(realm, MapKey::world(0));
    (c, g, map)
}

/// Health and mana, and their maximums.
async fn whole(map: &MapHandle, g: Guid) -> [u32; 4] {
    on(map, move |m| {
        let f = &m.players[&g].fields;
        [
            f.get_u32(UNIT_FIELD_HEALTH),
            f.get_u32(UNIT_FIELD_MAXHEALTH),
            f.get_u32(UNIT_FIELD_POWER1),
            f.get_u32(UNIT_FIELD_MAXPOWER1),
        ]
    })
    .await
}

/// mangos's `Player::GiveLevel`: a level set fills health and mana.
#[tokio::test]
async fn a_level_set_leaves_a_character_at_full_health_and_mana() {
    let realm = harness::realm_with(world());
    let (mut c, g, map) = player(&realm, "Ready", Class::Mage).await;
    // Hurt and drained, then levelled.
    on(&map, move |m| {
        let f = &mut m.players.get_mut(&g).unwrap().fields;
        f.set_u32(UNIT_FIELD_HEALTH, 1);
        f.set_u32(UNIT_FIELD_POWER1, 0);
    })
    .await;
    c.say(".character level 60").await;
    c.system_message("Level 60").await;
    let [health, max_health, mana, max_mana] = whole(&map, g).await;
    assert!(max_health > 100 && max_mana > 100, "a level-60 mage's");
    assert_eq!((health, mana), (max_health, max_mana));
}

/// The user's raiders came at half health: `.gear`'s stamina and intellect
/// raised their maximums and left them where they were. A GM's
/// convenience: it leaves them full, and says so.
#[tokio::test]
async fn gear_leaves_a_character_at_full_health_and_mana_and_says_so() {
    let realm = harness::realm_with(world());
    let (mut c, g, map) = player(&realm, "Geared", Class::Mage).await;
    c.say(".character level 60").await;
    c.system_message("Level 60").await;
    let before = whole(&map, g).await;
    c.say(".gear").await;
    let reply = c.system_message("Equipped").await;
    let [health, max_health, mana, max_mana] = whole(&map, g).await;
    let worn = on(&map, move |m| m.players[&g].count_of(ROBE)).await;
    assert_eq!(worn, 1, "the robe is worn");
    assert!(
        max_health >= before[1] + 400 && max_mana >= before[3] + 450,
        "the robe's stamina and intellect: {before:?} to {max_health}, {max_mana}"
    );
    assert_eq!((health, mana), (max_health, max_mana));
    assert!(reply.contains("full health and mana"), "{reply}");
}

/// `.character spells`: what the class's trainers would teach the player
/// now, free, as each teaches it (`trainer_buy`): a rank once the one
/// before it is had, a spell once the one it needs is, the learn effect's
/// spell. Not a rank above the level, a talent's rank to one without the
/// talent, another class's spell, nor a pet trainer's or a weapon
/// master's. Asked again, nothing more.
#[tokio::test]
async fn character_spells_teaches_what_the_class_trainers_would() {
    let realm = harness::realm_with(world());
    let (mut c, g, map) = player(&realm, "Learner", Class::Warrior).await;
    c.say(".character level 20").await;
    c.system_message("Level 20").await;
    c.say(".character spells").await;
    assert_eq!(c.system_message("Learned").await, "Learned 4 spells.");
    let knows = |spells: &[u32]| {
        let spells = spells.to_vec();
        let map = map.clone();
        async move {
            on(&map, move |m| {
                let p = &m.players[&g];
                spells
                    .iter()
                    .map(|s| p.spells.contains(s))
                    .collect::<Vec<_>>()
            })
            .await
        }
    };
    assert_eq!(
        knows(&[STRIKE[0], STRIKE[1], SHOUT, STANCE]).await,
        [true; 4],
        "the ranks up to the level, and the shout after its stance"
    );
    assert_eq!(
        knows(&[STRIKE[2], TALENT[1], MAGES, PETS, AXES, TEACHES + STRIKE[0]]).await,
        [false; 6],
        "nothing above the level, of a talent, another class's, a pet or weapon trainer's, \
         nor the teaching spell itself"
    );
    c.say(".character spells").await;
    assert_eq!(c.system_message("Learned").await, "Learned 0 spells.");

    let (mut c, g, map) = player(&realm, "Novice", Class::Warrior).await;
    let before = on(&map, move |m| m.players[&g].spells.clone()).await;
    c.say(".character spells").await;
    assert_eq!(c.system_message("Learned").await, "Learned 0 spells.");
    let after = on(&map, move |m| m.players[&g].spells.clone()).await;
    assert_eq!(before, after, "a level-1 warrior learns nothing new");
}

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

/// A `race` `class` of `level` on the real data, given `.character
/// spells`: the reply's count and what they know.
async fn trained(
    realm: &Arc<Realm>,
    account: &str,
    race: Race,
    class: Class,
    level: u8,
) -> (String, Vec<u32>) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(account, race, class).await;
    // Where the race starts: an orc in Kalimdor.
    let map = realm.registry.online(g).expect("in the world").map();
    if level > 1 {
        c.say(&format!(".character level {level}")).await;
        c.system_message(&format!("Level {level}")).await;
    }
    c.say(".character spells").await;
    let reply = c.system_message("Learned").await;
    let spells = on(&map, move |m| m.players[&g].spells.clone()).await;
    (reply, spells)
}

/// The real trainers' spells, the highest rank for the level of each, as
/// the trainers teach them: a level-60 mage's Frostbolt (rank 10: the
/// eleventh is a tome's), Arcane Explosion and Blizzard; their side's
/// teleports, not the other's; no talent's ranks.
#[tokio::test]
#[ignore = "extended"]
async fn character_spells_gives_a_level_60_mage_the_top_ranks() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let (reply, spells) = trained(&realm, "Mage", Race::Undead, Class::Mage, 60).await;
    let n: usize = reply
        .trim_start_matches("Learned ")
        .trim_end_matches(" spells.")
        .parse()
        .unwrap_or_else(|_| panic!("{reply}"));
    assert!(n > 100, "{reply}");
    // Frostbolt 10, Arcane Explosion 6, Blizzard 6, and the ranks below.
    for spell in [10_181, 10_202, 10_187, 116, 1449, 10] {
        assert!(spells.contains(&spell), "knows {spell}");
    }
    // Frostbolt 11 is a tome's; Pyroblast's ranks follow its talent; the
    // Alliance's teleport isn't an undead's.
    for spell in [25_304, 12_505, 3561] {
        assert!(!spells.contains(&spell), "doesn't know {spell}");
    }
    assert!(spells.contains(&3563), "Teleport: Undercity");
}

/// A level-55 warrior: Heroic Strike's seventh rank (the eighth is at 56),
/// Whirlwind; Mortal Strike's trained ranks follow its talent. A level-1
/// warrior has nothing new but Battle Shout, which the trainers teach at
/// level 1 (what it starts with it knows already).
#[tokio::test]
#[ignore = "extended"]
async fn character_spells_gives_a_warrior_the_ranks_for_its_level() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let (_, spells) = trained(&realm, "Warrior", Race::Orc, Class::Warrior, 55).await;
    assert!(spells.contains(&11_566), "Heroic Strike 7");
    assert!(!spells.contains(&11_567), "Heroic Strike 8 is at 56");
    assert!(spells.contains(&1680), "Whirlwind");
    assert!(!spells.contains(&21_551), "Mortal Strike 2, the talent's");
    let (reply, spells) = trained(&realm, "Novice", Race::Orc, Class::Warrior, 1).await;
    assert_eq!(reply, "Learned 1 spells.");
    assert!(spells.contains(&6673), "Battle Shout");
    assert!(!spells.contains(&100), "nor Charge, at 4");
}
