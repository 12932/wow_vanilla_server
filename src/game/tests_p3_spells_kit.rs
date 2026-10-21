//! What Phase 3's spell and aura tests share: the real data, units to cast
//! at, spells made up for a test, and the map's clock moved by hand.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::data::GameData;
use crate::game::data::spells::{SpellEffect, SpellTemplate};
use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells;
use crate::game::map::{Map, MapHandle, MapKey, auras};

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

pub fn have_data() -> bool {
    DB.as_path().join("vmangos.sqlite").exists()
}

/// A realm on the real data.
pub fn realm() -> Arc<Realm> {
    realm_plus(|_| {})
}

/// A realm on the real data, with what `f` adds to it (spells made up for
/// a test).
pub fn realm_plus(f: impl FnOnce(&mut GameData)) -> Arc<Realm> {
    let mut data = crate::game::harness::load_real(DB.as_path());
    f(&mut data);
    harness::realm_with(data)
}

/// A player of `race` and `class` in the world, with their map.
pub async fn player(
    realm: &Arc<Realm>,
    account: &str,
    name: &str,
    race: Race,
    class: Class,
) -> (Client, Guid, MapHandle) {
    let mut c = Client::connect(realm, account);
    let g = c.enter(name, race, class).await;
    let map_id = if matches!(
        race,
        Race::NightElf | Race::Orc | Race::Tauren | Race::Troll
    ) {
        1
    } else {
        0
    };
    let map = realm.maps.get(realm, MapKey::world(map_id));
    (c, g, map)
}

/// A Kobold Vermin at `level`, `yards` in front of `g`, hardy and without
/// armor.
pub fn dummy(m: &mut Map, g: Guid, level: u8, yards: f32) -> Guid {
    spawn(m, g, 6, level, yards)
}

/// A creature of `entry` at `level`, `yards` in front of `g`, facing it,
/// hardy and without armor.
pub fn spawn(m: &mut Map, g: Guid, entry: u32, level: u8, yards: f32) -> Guid {
    let data = m.realm.data.clone();
    let template = data
        .creature_template(entry)
        .cloned()
        .expect("the creature");
    let (pos, o) = match m.players.get(&g) {
        Some(p) => (p.position(), p.movement.orientation),
        None => {
            let c = m.creature(g).expect("a unit");
            (c.position, c.orientation)
        }
    };
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

/// `g` at `level`, knowing `learn`, with plenty of every power.
pub fn ready(m: &mut Map, g: Guid, level: u8, learn: &[u32]) {
    let data = m.realm.data.clone();
    let p = m.players.get_mut(&g).unwrap();
    crate::game::map::stats::set_level(p, &data, level);
    for s in learn {
        spells::learn(m, g, *s);
    }
    let p = m.players.get_mut(&g).unwrap();
    for i in 0..5 {
        p.fields.set_u32(UNIT_FIELD_MAXPOWER1 + i, 100_000);
        p.fields.set_u32(UNIT_FIELD_POWER1 + i, 100_000);
    }
}

pub fn has(m: &Map, g: Guid, spell: u32) -> bool {
    auras::auras(m, g).is_some_and(|a| a.has_spell(spell))
}

pub fn holder(m: &Map, g: Guid, spell: u32) -> Option<auras::Holder> {
    auras::auras(m, g).and_then(|a| a.holders.iter().find(|h| h.spell == spell).cloned())
}

/// `id`'s auras from `caster` on `target`, as the data has them.
pub fn apply(m: &mut Map, caster: Guid, target: Guid, id: u32) {
    let data = m.realm.data.clone();
    let s = data.spells.get(id).unwrap().clone();
    apply_template(m, caster, target, &s);
}

/// A template's auras from `caster` on `target`.
pub fn apply_template(m: &mut Map, caster: Guid, target: Guid, s: &SpellTemplate) {
    let effects: Vec<(usize, i32)> = (0..3)
        .filter(|i| s.effects[*i].effect == 6 && s.effects[*i].aura != 0)
        .map(|i| (i, spells::points(s, i, 60)))
        .collect();
    let _ = auras::apply_spell(m, caster, target, s, &effects, auras::Context::default());
}

pub fn health(m: &Map, g: Guid) -> u32 {
    m.unit_fields(g).map_or(0, |f| f.get_u32(UNIT_FIELD_HEALTH))
}

pub fn set_health(m: &mut Map, g: Guid, h: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_HEALTH, h);
    }
}

/// `g`'s power of `index` (0 mana, 1 rage, 3 energy).
pub fn power(m: &Map, g: Guid, index: u16) -> u32 {
    m.unit_fields(g)
        .map_or(0, |f| f.get_u32(UNIT_FIELD_POWER1 + index))
}

pub fn set_power(m: &mut Map, g: Guid, index: u16, v: u32) {
    if let Some(f) = m.unit_fields_mut(g) {
        f.set_u32(UNIT_FIELD_POWER1 + index, v);
    }
}

pub fn at(unit: Guid) -> spells::Targets {
    spells::Targets {
        unit: Some(unit),
        ..Default::default()
    }
}

/// `g`'s cast under way: its spell.
pub fn casting(m: &Map, g: Guid) -> Option<u32> {
    m.caster(g)
        .and_then(|c| c.casting.as_ref())
        .map(|c| c.spell)
}

/// Move the map's clock on by `ms` and run what time drives in spells and
/// auras.
pub fn pass(m: &mut Map, ms: u64) {
    m.now += Duration::from_millis(ms);
    spells::update(m);
    auras::update(m);
}

/// A spell made up for a test: one effect, its aura of `aura` with `misc`
/// and `amount`, on its target.
pub fn made_up(id: u32, effect: u32, aura: u32, misc: i32, amount: i32) -> SpellTemplate {
    let mut effects = [SpellEffect::default(); 3];
    effects[0] = SpellEffect {
        effect,
        aura,
        misc_value: misc,
        base_points: amount - 1,
        base_dice: 1,
        target_a: 1,
        ..Default::default()
    };
    SpellTemplate {
        id,
        name: "Test Spell".into(),
        equipped_item_class: -1,
        range_index: 1,
        effects,
        ..Default::default()
    }
}

/// `g` free to cast again: no cast under way, no global cooldown, no
/// cooldowns.
pub fn fresh(m: &mut Map, g: Guid) {
    if let Some(c) = m.caster_mut(g) {
        c.casting = None;
        c.gcd_until = None;
        c.cooldowns.clear();
        c.category_cooldowns.clear();
    }
}

/// `spell` cast by `caster` at `t`, from a fresh start: whether it began.
pub fn cast_fresh(m: &mut Map, caster: Guid, spell: u32, t: spells::Targets) -> bool {
    fresh(m, caster);
    spells::cast(m, caster, spell, t, None)
}

/// Make `g`'s ticks of `spell` due now and run the auras: a tick of each.
pub fn tick_now(m: &mut Map, g: Guid, spell: u32) {
    let now = m.now;
    if let Some(a) = auras::auras_mut(m, g) {
        for h in a.holders.iter_mut().filter(|h| h.spell == spell) {
            for x in h.effects.iter_mut().filter(|x| x.period.is_some()) {
                x.next_tick = Some(now);
            }
        }
    }
    auras::update(m);
}

/// Whether `c` was told `spell` went off.
pub async fn saw_go(c: &mut Client, spell: u32) -> bool {
    use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
    c.gets(Duration::from_millis(300), |m| match m {
        ServerOpcodeMessage::SMSG_SPELL_GO(g) if g.spell == spell => Some(()),
        _ => None,
    })
    .await
    .is_some()
}

/// How `c` was told `spell` missed its first target: the reason (7 immune,
/// 2 resist, 11 reflect) in the go's misses, or in `SMSG_SPELLLOGMISS` for
/// a miss that came about as it landed.
pub async fn missed(c: &mut Client, spell: u32) -> u8 {
    c.told_miss(spell).await.1
}
