//! A client held in one of the update's steps before its visibility (a
//! stealthed pet striking from its prowl in the pets' step, seen by a crowd
//! at once) is let go at that step's end if it has frames waiting
//! (`Map::let_go_waiting`): its forgets and creates first, then what
//! waited, in order; one with nothing waiting stays held, its blocks for
//! the update's batch; and none goes while something put in the map waits
//! for the update's visibility.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::harness::{self, Client};
use crate::game::map::{Map, MapKey, visibility};

/// What `c` was told since the last look, in order: `forget` and `create`
/// for update blocks about `of`, `emote` for an emote of `of`'s.
fn told(c: &mut Client, of: Guid) -> Vec<&'static str> {
    let mut out = Vec::new();
    for m in std::mem::take(&mut c.backlog) {
        match &m {
            ServerOpcodeMessage::SMSG_EMOTE(e) if e.guid == of => out.push("emote"),
            ServerOpcodeMessage::SMSG_UPDATE_OBJECT(_)
            | ServerOpcodeMessage::SMSG_COMPRESSED_UPDATE_OBJECT(_) => {
                if harness::forgotten(&m).contains(&of) {
                    out.push("forget");
                }
                if harness::creates(&m).contains(&of) {
                    out.push("create");
                }
            }
            _ => {}
        }
    }
    out
}

/// As a step before visibility would: `observer` forgets `object` and is
/// told of it afresh (a stealthed one seen again), held for its create,
/// and is sent an emote of the object's meanwhile.
fn seen_again(m: &mut Map, observer: Guid, object: Guid, emote: bool) {
    visibility::part(m, observer, object);
    visibility::introduce(m, observer, object);
    if emote
        && let Some(frame) = crate::game::conn::encode(&SMSG_EMOTE {
            emote: Emote::OneshotWave,
            guid: object,
        })
    {
        m.send_to_observers(object, &frame, false);
    }
}

#[tokio::test]
async fn a_client_held_in_a_step_is_let_go_at_its_end_creates_first() {
    let realm = harness::realm();
    let map = realm.maps.get(&realm, MapKey::world(0));
    let mut a = Client::connect(&realm, "helda");
    let ga = a.enter("Helda", Race::Human, Class::Warrior).await;
    let mut b = Client::connect(&realm, "heldb");
    let gb = b.enter("Heldb", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "heldc");
    let gc = c.enter("Heldc", Race::Human, Class::Warrior).await;
    a.sees(gb).await;
    c.sees(gb).await;
    harness::an_update(&map).await;
    a.clear();
    c.clear();
    // Alice sees Bob again with an emote of his waiting; Carol sees him
    // again with nothing waiting.
    let (released, kept) = map
        .query(move |m| {
            m.let_go_after_steps = true;
            seen_again(m, ga, gb, true);
            seen_again(m, gc, gb, false);
            m.let_go_waiting();
            m.let_go_after_steps = false;
            let held: Vec<Guid> = m.holding.iter().map(|(g, _)| *g).collect();
            (!held.contains(&ga), held.contains(&gc))
        })
        .await
        .unwrap();
    assert!(released, "Alice, with a frame waiting, is let go");
    assert!(
        kept,
        "Carol, with nothing waiting, stays held for the update"
    );
    let _ = a
        .gets(std::time::Duration::from_millis(300), |_| None::<()>)
        .await;
    assert_eq!(told(&mut a, gb), ["forget", "create", "emote"], "Alice");
    // Carol's come with the update, still held until then (the emote came
    // while she still knew him).
    harness::an_update(&map).await;
    let _ = c
        .gets(std::time::Duration::from_millis(300), |_| None::<()>)
        .await;
    assert_eq!(told(&mut c, gb), ["emote", "forget", "create"], "Carol");

    // Something put in the map, waiting for the update's visibility: no
    // one is let go before it.
    let waits = map
        .query(move |m| {
            m.let_go_after_steps = true;
            seen_again(m, ga, gb, true);
            m.fresh.push(gb);
            m.let_go_waiting();
            m.let_go_after_steps = false;
            let held = m.holding.iter().any(|(g, _)| *g == ga);
            m.fresh.clear();
            held
        })
        .await
        .unwrap();
    assert!(
        waits,
        "held while something waits for the update's visibility"
    );
    harness::an_update(&map).await;
    let _ = a
        .gets(std::time::Duration::from_millis(300), |_| None::<()>)
        .await;
    assert_eq!(
        told(&mut a, gb),
        ["forget", "create", "emote"],
        "Alice, at the update"
    );
}
