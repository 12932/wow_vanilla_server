//! `.gear` for a tank, over the real items: a warrior, paladin or shaman
//! tank's shield goes with a sword, mace or axe (as the class may), never
//! a fist weapon or a dagger; a druid's two-hander is a staff or a mace;
//! a tank's stats first, a caster's never. Many draws of each quality, as
//! peon's raids ask them.

use std::collections::BTreeMap;
use std::sync::Arc;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapHandle, MapKey};
use crate::game::store::BAG_PLAYER;

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

fn have_data() -> bool {
    let db = DB.join("vmangos.sqlite");
    db.exists() && !crate::game::data::load::lfs_pointer(&db)
}

async fn on<T: Send + 'static>(
    map: &MapHandle,
    f: impl FnOnce(&mut Map) -> T + Send + 'static,
) -> T {
    map.query(f).await.expect("the map is up")
}

/// A weapon subclass's name.
fn kind(subclass: u32) -> &'static str {
    match subclass {
        0 => "axe",
        1 => "two-handed axe",
        4 => "mace",
        5 => "two-handed mace",
        6 => "polearm",
        7 => "sword",
        8 => "two-handed sword",
        10 => "staff",
        13 => "fist weapon",
        15 => "dagger",
        _ => "other",
    }
}

/// What a tank was given in its hands, `draws` times over for each of
/// `commands`: the main hand's kind, its stats (strength, agility,
/// stamina, intellect, spirit) and name, and whether the off hand is a
/// shield.
type Draw = (u32, [i32; 5], String, bool);

async fn draw(
    realm: &Arc<Realm>,
    (account, name): (&str, &str),
    race: Race,
    class: Class,
    level: u8,
    commands: &[(&str, usize)],
) -> Vec<(String, Vec<Draw>)> {
    let mut c = Client::connect(realm, account);
    let g: Guid = c.enter(name, race, class).await;
    let continent = u32::from(matches!(
        race,
        Race::Orc | Race::Tauren | Race::Troll | Race::NightElf
    ));
    let map = realm.maps.get(realm, MapKey::world(continent));
    c.say(&format!(".character level {level}")).await;
    c.system_message(&format!("Level {level}")).await;
    let mut out = Vec::new();
    for (command, draws) in commands {
        let mut got = Vec::new();
        for _ in 0..*draws {
            c.say(command).await;
            c.system_message("Equipped").await;
            let hands = on(&map, move |m| {
                let data = m.realm.data.clone();
                let p = &m.players[&g];
                let at = |slot: u8| {
                    p.items
                        .iter()
                        .find(|i| i.bag == BAG_PLAYER && i.slot == slot)
                        .and_then(|i| data.item(i.entry))
                        .cloned()
                };
                let main = at(15)?;
                let shield = at(16).is_some_and(|t| t.class == 4 && t.subclass == 6);
                let stat = |k: u32| {
                    main.stats
                        .iter()
                        .filter(|(s, _)| *s == k)
                        .map(|(_, v)| *v)
                        .sum::<i32>()
                };
                Some((
                    main.subclass,
                    [stat(4), stat(3), stat(7), stat(5), stat(6)],
                    main.name.clone(),
                    shield,
                ))
            })
            .await;
            got.extend(hands);
        }
        out.push(((*command).to_owned(), got));
    }
    out
}

/// What each kind of draw came to: its weapon kinds counted.
fn tally(draws: &[Draw]) -> String {
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    for (sub, ..) in draws {
        *by.entry(kind(*sub)).or_default() += 1;
    }
    by.iter()
        .map(|(k, n)| format!("{k} {n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A few of the weapons drawn, each once.
fn some(draws: &[Draw]) -> String {
    let mut names: Vec<&str> = Vec::new();
    for (.., name, _) in draws {
        if !names.contains(&name.as_str()) && names.len() < 4 {
            names.push(name);
        }
    }
    names.join(", ")
}

/// A tank's weapon stats: strength, agility or stamina, and no intellect
/// or spirit.
fn tanks_stats(stats: &[i32; 5]) -> bool {
    stats[..3].iter().any(|v| *v > 0) && !casters(stats)
}

/// A caster's weapon stats: intellect or spirit.
fn casters(stats: &[i32; 5]) -> bool {
    stats[3] > 0 || stats[4] > 0
}

/// A warrior, paladin and shaman tank, `.gear [quality] tank [shield]` as
/// peon asks it (`.gear green tank shield`) and bare, many times at 60 and
/// at 45: always a shield, and in the main hand a sword, mace or axe, as
/// the class may hold; never a fist weapon or a dagger, nor a caster's
/// (intellect or spirit). A druid tank's two-hander is a staff or a mace,
/// not a caster's. The tallies are printed: what each tank gets.
#[tokio::test]
#[ignore = "extended"]
async fn gear_gives_a_tank_a_tanks_weapon() {
    if !have_data() {
        return;
    }
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let random = |n| {
        vec![
            (".gear tank", n),
            (".gear green tank shield", n),
            (".gear tank shield", n),
            (".gear blue tank shield", n),
            (".gear raid tank shield", 2),
        ]
    };
    let tanks = [
        (Race::Human, Class::Warrior, "TANKGW", "Tankgwar"),
        (Race::Human, Class::Paladin, "TANKGP", "Tankgpal"),
        (Race::Orc, Class::Shaman, "TANKGS", "Tankgsha"),
    ];
    let mut report = Vec::new();
    let mut wrong = Vec::new();
    for level in [60u8, 45] {
        for (race, class, account, name) in tanks {
            let account = format!("{account}{level}");
            let name = format!("{name}{}", if level == 60 { "a" } else { "b" });
            for (command, draws) in
                draw(&realm, (&account, &name), race, class, level, &random(25)).await
            {
                let stats = draws.iter().filter(|(_, s, ..)| tanks_stats(s)).count();
                report.push(format!(
                    "{class:?} {level} `{command}`: {}; a tank's stats {stats} of {}; e.g. {}",
                    tally(&draws),
                    draws.len(),
                    some(&draws)
                ));
                for (sub, s, item, shield) in &draws {
                    if !matches!(sub, 0 | 4 | 7) || !shield || casters(s) {
                        wrong.push(format!(
                            "{class:?} {level} `{command}`: {item} ({}), stats {s:?}, shield {shield}",
                            kind(*sub)
                        ));
                    }
                }
            }
        }
        // A druid tank: a staff or a two-handed mace, a tank's stats where
        // there are any of the quality.
        let (account, name) = (
            format!("TANKGD{level}"),
            format!("Tankgdru{}", if level == 60 { "a" } else { "b" }),
        );
        let commands = [
            (".gear tank", 25),
            (".gear green tank", 25),
            (".gear blue tank", 25),
            (".gear raid tank", 2),
        ];
        for (command, draws) in draw(
            &realm,
            (&account, &name),
            Race::NightElf,
            Class::Druid,
            level,
            &commands,
        )
        .await
        {
            let stats = draws.iter().filter(|(_, s, ..)| tanks_stats(s)).count();
            report.push(format!(
                "Druid {level} `{command}`: {}; a tank's stats {stats} of {}; e.g. {}",
                tally(&draws),
                draws.len(),
                some(&draws)
            ));
            for (sub, s, item, _) in &draws {
                if !matches!(sub, 5 | 10) || casters(s) {
                    wrong.push(format!(
                        "Druid {level} `{command}`: {item} ({}), stats {s:?}",
                        kind(*sub)
                    ));
                }
            }
        }
    }
    eprintln!("{}", report.join("\n"));
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
