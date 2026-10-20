//! The GUIDs of what a map makes itself (summons, pets, a script's
//! objects): never one that something on the map still has, though the
//! counter comes round (review 5's rev5-maps 7).

use wow_world_messages::vanilla::*;

use crate::game::data::{CreatureTemplate, GameData, GameObjectTemplate};
use crate::game::guid;
use crate::game::harness;
use crate::game::map::{MapKey, OWN_MARK, PET_MARK, scripts};

const IMP: u32 = 416;
const BOBBER: u32 = 35591;

fn data() -> GameData {
    let mut d = GameData::empty();
    d.creature_templates.insert(
        IMP,
        CreatureTemplate {
            entry: IMP,
            name: "Imp".into(),
            display_ids: [4449, 0, 0, 0],
            display_probabilities: [1, 0, 0, 0],
            level_min: 5,
            level_max: 5,
            faction: 14,
            health_multiplier: 1.0,
            ..Default::default()
        },
    );
    d.gameobject_templates.insert(
        BOBBER,
        GameObjectTemplate {
            entry: BOBBER,
            name: "Fishing Bobber".into(),
            ..Default::default()
        },
    );
    d
}

const AT: Vector3d = Vector3d {
    x: -8949.95,
    y: -132.493,
    z: 83.5312,
};

#[tokio::test]
async fn a_summon_never_takes_the_guid_of_one_still_there() {
    let realm = harness::realm_with(data());
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (first, second, creatures, indexed) = map
        .query(|m| {
            let first = scripts::summon(m, Guid::zero(), IMP, AT, 0.0, None).unwrap();
            // The counter all the way round, 2^23 summons on.
            m.next_creature_counter += OWN_MARK - 1;
            let second = scripts::summon(m, Guid::zero(), IMP, AT, 0.0, None).unwrap();
            let imps = m.creatures.iter().filter(|(_, c)| c.entry == IMP).count();
            let indexed = [first, second]
                .iter()
                .filter(|g| m.creature(**g).is_some())
                .count();
            (first, second, imps, indexed)
        })
        .await
        .unwrap();
    assert_ne!(first, second, "a second GUID");
    assert_eq!((creatures, indexed), (2, 2));
    assert_eq!(guid::counter(first) & OWN_MARK, OWN_MARK);
    assert_eq!(guid::counter(second) & OWN_MARK, OWN_MARK);

    // One gone, its counter is free again.
    let again = map
        .query(move |m| {
            m.remove_creature(first);
            m.next_creature_counter = guid::counter(first) & !OWN_MARK;
            m.creature_counter()
        })
        .await
        .unwrap();
    assert_eq!(again, guid::counter(first));
}

#[tokio::test]
async fn pets_and_objects_have_counts_of_their_own_and_skip_those_in_use() {
    let realm = harness::realm_with(data());
    let map = realm.maps.get(&realm, MapKey::world(0));
    let (pets, objects, creature) = map
        .query(|m| {
            let pets: Vec<u32> = (0..3).map(|_| m.pet_counter()).collect();
            let objects: Vec<u32> = (0..3).map(|_| m.object_counter()).collect();
            // A unit whose counter is a pet's, there: the pets' count skips
            // it when it comes round.
            let pet = pets[0];
            let creature = m.creature_counter();
            let imp = guid::unit(IMP, pet);
            let t = m.realm.data.creature_template(IMP).unwrap().clone();
            let c = crate::game::map::creature::Creature::new(imp, &t, 5, AT, 0.0);
            m.add_creature(c);
            m.next_creature_counter = pet & !PET_MARK;
            let next = m.pet_counter();
            (pets, objects, (creature, next, pet))
        })
        .await
        .unwrap();
    assert!(pets.iter().all(|c| c & PET_MARK == PET_MARK), "{pets:x?}");
    assert_eq!(
        objects,
        [OWN_MARK | 1, OWN_MARK | 2, OWN_MARK | 3],
        "the objects' own count, from 1"
    );
    let (_, next, pet) = creature;
    assert_ne!(next, pet, "the one in use is skipped");
    assert_eq!(next, pet + 1);
}
