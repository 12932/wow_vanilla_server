//! Stage 2a's spells and auras, against the real data when it is here: a
//! dispel taking a passive aura.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::fields::*;
use crate::game::guid;
use crate::game::harness::{self, Client};
use crate::game::map::creature::Creature;
use crate::game::map::spells;
use crate::game::map::{Map, MapKey, auras};

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
