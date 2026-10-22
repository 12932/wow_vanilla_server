//! A spellbook across a far teleport out of a dungeon: a level 60 player
//! taught their class's spells in the Deadmines, a spell of theirs on a
//! bar, out through the exit to Westfall, still has every spell in the
//! client's spellbook and the spell on the bar. On the real world, in the
//! extended suite.
//!
//! The 1.12 client builds its spellbook, and its bars, as its own player is
//! created (at a login and after every far teleport) from the last
//! `SMSG_INITIAL_SPELLS` and `SMSG_ACTION_BUTTONS` it was sent; a spell
//! learned in the world, or a button set, changes only what it shows
//! (`tools/client_check/initial_spells.py`). [`Book`] follows the frames as
//! the client does, and the tests hold it to what the server knows.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};
use crate::game::tests_p3_entity::on;

static DB: std::sync::LazyLock<std::path::PathBuf> =
    std::sync::LazyLock::new(|| harness::real_data("db"));

/// The Deadmines, and its exit to Westfall.
const DEADMINES: u32 = 36;
const EXIT: u32 = 119;
/// Mind Control's third rank, a level 58 priest's; Frostbolt's ninth, a
/// level 56 mage's (the tenth is a book's); Execute's fifth, a level 56
/// warrior's.
const MIND_CONTROL: u32 = 10912;
const FROSTBOLT: u32 = 10181;
const EXECUTE: u32 = 20662;

const SMSG_LEARNED_SPELL: u16 = 0x12B;
const SMSG_SUPERCEDED_SPELL: u16 = 0x12C;
const SMSG_REMOVED_SPELL: u16 = 0x203;

/// The client's spellbook and bars, as the 1.12 client keeps them: what the
/// last `SMSG_INITIAL_SPELLS` and `SMSG_ACTION_BUTTONS` said (`sent_*`),
/// and what it shows, built from those as its own player is created and
/// changed in the world by what it learns and sets.
#[derive(Default)]
struct Book {
    sent_spells: Vec<u32>,
    sent_buttons: Vec<u32>,
    spells: BTreeSet<u32>,
    buttons: Vec<u32>,
    /// Whether the client has a player of its own (between its create and
    /// a far teleport).
    in_world: bool,
}

impl Book {
    /// Follow `frames`, as the client gets them; `own` is its player.
    fn follow(&mut self, frames: &[Arc<[u8]>], own: Guid) {
        for f in frames {
            let opcode = u16::from_le_bytes([f[2], f[3]]);
            let body = &f[4..];
            let u16_at = |i: usize| u32::from(u16::from_le_bytes([body[i], body[i + 1]]));
            match opcode {
                SMSG_LEARNED_SPELL => self.learned(u16_at(0)),
                SMSG_SUPERCEDED_SPELL => self.superseded(u16_at(0), u16_at(2)),
                SMSG_REMOVED_SPELL => self.removed(u16_at(0)),
                _ => {}
            }
            let Ok(m) = ServerOpcodeMessage::read_unencrypted(&f[..]) else {
                continue;
            };
            match &m {
                ServerOpcodeMessage::SMSG_INITIAL_SPELLS(s) => {
                    self.sent_spells = s
                        .initial_spells
                        .iter()
                        .map(|s| u32::from(s.spell_id))
                        .collect();
                }
                ServerOpcodeMessage::SMSG_ACTION_BUTTONS(b) => self.sent_buttons = b.data.to_vec(),
                ServerOpcodeMessage::SMSG_NEW_WORLD(_) => self.in_world = false,
                _ if harness::creates(&m).contains(&own) => self.created(),
                _ => {}
            }
        }
    }

    /// Its own player created: the book emptied and filled from the list
    /// sent, the bars set from the buttons sent but for a spell not in it.
    fn created(&mut self) {
        self.in_world = true;
        self.spells = self.sent_spells.iter().copied().collect();
        self.buttons = self
            .sent_buttons
            .iter()
            .map(|&b| {
                let spell = b & 0xF000_0000 == 0;
                if spell && b != 0 && !self.spells.contains(&(b & 0xFF_FFFF)) {
                    0
                } else {
                    b
                }
            })
            .collect();
    }

    fn learned(&mut self, spell: u32) {
        if self.in_world {
            self.spells.insert(spell);
        } else {
            self.sent_spells.push(spell);
        }
    }

    fn superseded(&mut self, old: u32, new: u32) {
        if self.in_world {
            self.spells.remove(&old);
            self.spells.insert(new);
        } else if let Some(s) = self.sent_spells.iter_mut().find(|s| **s == old) {
            *s = new;
        } else {
            self.sent_spells.push(new);
        }
    }

    fn removed(&mut self, spell: u32) {
        if self.in_world {
            self.spells.remove(&spell);
        } else {
            self.sent_spells.retain(|s| *s != spell);
        }
    }

    /// A button the player sets: what it shows alone.
    fn set(&mut self, button: usize, action: u32) {
        if self.buttons.len() <= button {
            self.buttons.resize(button + 1, 0);
        }
        self.buttons[button] = action;
    }
}

/// Wait for the far teleport's `SMSG_NEW_WORLD` to `map`, acknowledge it,
/// and wait for the player's own create on it.
async fn far_teleported(c: &mut Client, map: u32) {
    let to = c
        .wait_for("SMSG_NEW_WORLD", |m| match m {
            ServerOpcodeMessage::SMSG_NEW_WORLD(w) => Some(w.map.as_int()),
            _ => None,
        })
        .await;
    assert_eq!(to, map, "to map {map}");
    let from = c.captured().len();
    c.send(MSG_MOVE_WORLDPORT_ACK {}).await;
    // The create after the acknowledgement (one before it may not have
    // been waited for).
    let own = c.guid;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let frames = c.captured();
        let created = frames[from..].iter().any(|f| {
            ServerOpcodeMessage::read_unencrypted(&f[..])
                .is_ok_and(|m| harness::creates(&m).contains(&own))
        });
        if created {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "never created on map {map}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// The spells the server keeps in `g`'s spellbook: those known, but a rank
/// a known higher one has taken the place of.
async fn in_book(realm: &Arc<Realm>, g: Guid) -> BTreeSet<u32> {
    let map = realm.registry.online(g).expect("in the world").map();
    on(&map, move |m| {
        let p = &m.players[&g];
        p.spells
            .iter()
            .copied()
            .filter(|s| !crate::game::map::spells::superseded_in_book(&m.realm.data, &p.spells, *s))
            .collect()
    })
    .await
}

/// A level 60 `class` taught their class's spells in the Deadmines, `spell`
/// on their first button, out through the exit to Westfall: the client's
/// spellbook and bars after are what the server knows. The frames of the
/// map change are run through the client's own parsers (`tools/client_check`)
/// when it is there.
async fn out_of_the_deadmines(account: &str, class: Class, spell: u32) {
    let realm = harness::realm_with(harness::load_real(DB.as_path()));
    let mut c = Client::connect(&realm, account);
    let g = c.enter(account, Race::Human, class).await;
    c.send(CMSG_COMPLETE_CINEMATIC {}).await;
    let mut book = Book::default();
    c.say(".character level 60").await;
    c.system_message("Level 60").await;
    c.say(".tele deadmines").await;
    far_teleported(&mut c, DEADMINES).await;
    c.say(".character spells").await;
    c.system_message("Learned").await;
    let known = in_book(&realm, g).await;
    assert!(known.contains(&spell), "{class:?} was taught {spell}");
    c.send(CMSG_SET_ACTION_BUTTON {
        button: 0,
        action: spell as u16,
        misc: (spell >> 16) as u8,
        action_type: 0,
    })
    .await;
    let map = realm.registry.online(g).unwrap().map();
    let mut set = false;
    for _ in 0..200 {
        set = on(&map, move |m| {
            let p = &m.players[&g];
            p.row
                .actions
                .iter()
                .any(|a| a.button == 0 && u32::from(a.action) == spell)
        })
        .await;
        if set {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(set, "the button is kept");
    let before = c.captured();
    book.follow(&before, g);
    book.set(0, spell);
    assert_eq!(book.spells, known, "the client's book in the Deadmines");

    // Into the exit's trigger, and through it.
    let t = realm
        .data
        .area_triggers
        .get(&EXIT)
        .expect("the exit")
        .clone();
    c.heartbeat(t.x, t.y, t.z).await;
    c.send(CMSG_AREATRIGGER { trigger_id: EXIT }).await;
    far_teleported(&mut c, 0).await;
    let all = c.captured();
    let change = &all[before.len()..];
    book.follow(change, g);

    let after = in_book(&realm, g).await;
    assert_eq!(after, known, "the server's spellbook across the teleport");
    assert_eq!(
        book.spells, after,
        "the client's spellbook in Westfall is the server's"
    );
    assert_eq!(
        book.buttons.first().copied(),
        Some(spell),
        "{spell} still on the first button"
    );
    // Told before the player's own create, which builds the book from them.
    let at = |opcode: u16| {
        change
            .iter()
            .position(|f| u16::from_le_bytes([f[2], f[3]]) == opcode)
    };
    let initial = at(0x12A).expect("SMSG_INITIAL_SPELLS at the map change");
    let buttons = at(0x129).expect("SMSG_ACTION_BUTTONS at the map change");
    let created = change
        .iter()
        .position(|f| {
            ServerOpcodeMessage::read_unencrypted(&f[..])
                .is_ok_and(|m| harness::creates(&m).contains(&g))
        })
        .expect("the player's own create");
    assert!(
        initial < created && buttons < created,
        "the spells ({initial}) and buttons ({buttons}) before the create ({created})"
    );

    match crate::game::tests_client_emu::checker() {
        Some((check_py, wow_exe)) => {
            let mut capture = Vec::new();
            for f in change {
                if [0x12A, 0x129, 0x0A9, 0x1F6].contains(&u16::from_le_bytes([f[2], f[3]])) {
                    capture.extend_from_slice(f);
                }
            }
            crate::game::tests_client_emu::run_checker(&check_py, &wow_exe, &capture);
        }
        None => eprintln!("no client check here: tools/client_check/WoW.exe and `uv` wanted"),
    }
}

#[tokio::test]
#[ignore = "extended"]
async fn a_priest_keeps_their_spellbook_out_of_the_deadmines() {
    out_of_the_deadmines("Shadowpriest", Class::Priest, MIND_CONTROL).await;
}

#[tokio::test]
#[ignore = "extended"]
async fn a_mage_keeps_their_spellbook_out_of_the_deadmines() {
    out_of_the_deadmines("Frostmage", Class::Mage, FROSTBOLT).await;
}

#[tokio::test]
#[ignore = "extended"]
async fn a_warrior_keeps_their_spellbook_out_of_the_deadmines() {
    out_of_the_deadmines("Axewarrior", Class::Warrior, EXECUTE).await;
}

/// What a far teleport keeps, on the small world: a spell learned in the
/// world and a button set after the login are in the `SMSG_INITIAL_SPELLS`
/// and `SMSG_ACTION_BUTTONS` sent at the worldport, before the player's own
/// create, so the client's book and bars ([`Book`]) are the server's after;
/// a near teleport sends neither.
#[tokio::test]
async fn a_far_teleport_tells_the_spellbook_and_bars_again() {
    const LEARNED: u32 = 133;
    let realm = harness::realm();
    let mut c = Client::connect(&realm, "Porter");
    let g = c.enter("Porter", Race::Human, Class::Mage).await;
    let map = realm.registry.online(g).unwrap().map();
    on(&map, move |m| {
        crate::game::map::spells::learn(m, g, LEARNED)
    })
    .await;
    c.send(CMSG_SET_ACTION_BUTTON {
        button: 7,
        action: LEARNED as u16,
        misc: 0,
        action_type: 0,
    })
    .await;
    let mut set = false;
    for _ in 0..200 {
        set = on(&map, move |m| {
            m.players[&g].row.actions.iter().any(|a| a.button == 7)
        })
        .await;
        if set {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(set, "the button is kept");
    let mut book = Book::default();
    let before = c.captured();
    book.follow(&before, g);
    book.set(7, LEARNED);
    assert!(book.spells.contains(&LEARNED), "learned in the world");

    // A near teleport: neither is sent again.
    let at = |x: f32| Vector3d { x, y: 0.0, z: 0.0 };
    on(&map, move |m| {
        crate::game::map::movement::teleport_within(m, g, at(10.0), 0.0)
    })
    .await;
    c.wait_for("MSG_MOVE_TELEPORT_ACK", |m| {
        matches!(m, ServerOpcodeMessage::MSG_MOVE_TELEPORT_ACK(_)).then_some(())
    })
    .await;
    let near = c.captured();
    let told = |frames: &[Arc<[u8]>], opcode: u16| {
        frames
            .iter()
            .filter(|f| u16::from_le_bytes([f[2], f[3]]) == opcode)
            .count()
    };
    assert_eq!(
        (
            told(&near[before.len()..], 0x12A),
            told(&near[before.len()..], 0x129)
        ),
        (0, 0),
        "a near teleport keeps the client's own player"
    );

    // A far one, to Kalimdor.
    map.call(move |m| m.teleport(g, crate::game::map::MapKey::world(1), at(0.0), 0.0));
    far_teleported(&mut c, 1).await;
    let all = c.captured();
    let change = &all[near.len()..];
    assert_eq!((told(change, 0x12A), told(change, 0x129)), (1, 1));
    book.follow(change, g);
    let known = in_book(&realm, g).await;
    assert!(known.contains(&LEARNED));
    assert_eq!(
        book.spells, known,
        "the client's book after the far teleport"
    );
    assert_eq!(
        book.buttons.get(7).copied(),
        Some(LEARNED),
        "the button kept"
    );
}
