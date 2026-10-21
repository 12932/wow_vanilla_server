//! Round 12's player commands, as mangos-classic's `SEC_PLAYER` table
//! (`PlayerCommands`, on by default): `.help`, `.server info|motd`, `.gm
//! ingame`, `.account` and `.account password <old> <new> <new>` for
//! anyone; a GM's commands are "no such command" to a player; a line of
//! dots is chat; off, a player's "." line is chat again.

use std::sync::Arc;
use std::time::Duration;

use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use crate::game::Realm;
use crate::game::harness::{self, Client};

fn realm_with(player_commands: bool) -> Arc<Realm> {
    let mut cfg = harness::test_config();
    cfg.everyone_gm = false;
    cfg.gm.player_commands = player_commands;
    let (store, startup) = crate::game::store::Store::open(":memory:").unwrap();
    Realm::new(cfg, store, startup, Default::default(), None)
}

async fn player(realm: &Arc<Realm>, account: &str, name: &str) -> Client {
    let mut c = Client::connect(realm, account);
    c.enter(name, Race::Human, Class::Warrior).await;
    c.clear();
    c
}

/// The next system line `c` is told, or none within a moment.
async fn told(c: &mut Client) -> Option<String> {
    c.gets(Duration::from_millis(500), |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(l)
            if matches!(l.chat_type, SMSG_MESSAGECHAT_ChatType::System { .. }) =>
        {
            Some(l.message.clone())
        }
        _ => None,
    })
    .await
}

async fn said(c: &mut Client) -> Option<String> {
    c.gets(Duration::from_millis(500), |m| match m {
        ServerOpcodeMessage::SMSG_MESSAGECHAT(l)
            if matches!(l.chat_type, SMSG_MESSAGECHAT_ChatType::Say { .. }) =>
        {
            Some(l.message.clone())
        }
        _ => None,
    })
    .await
}

#[tokio::test]
async fn a_player_has_mangos_player_commands() {
    let realm = realm_with(true);
    let mut a = player(&realm, "A", "Alice").await;
    a.say(".help").await;
    let help = told(&mut a).await.expect("the help");
    assert!(help.contains(".start") && help.contains(".gm ingame"), "{help}");
    assert!(!help.contains(".kick"), "no GM's command listed: {help}");
    a.say(".server info").await;
    let info = told(&mut a).await.expect("the server's info");
    assert!(info.contains("Players online: 1"), "{info}");
    a.say(".gm ingame").await;
    assert_eq!(
        told(&mut a).await.as_deref(),
        Some("There are no GMs currently logged in on this server.")
    );
    a.say(".account").await;
    assert_eq!(
        told(&mut a).await.as_deref(),
        Some("Your account level is: 0")
    );
    // A GM's command is none of a player's.
    a.say(".kick Bob").await;
    assert_eq!(
        told(&mut a).await.as_deref(),
        Some("There is no such command.")
    );
    a.say(".gm on").await;
    assert_eq!(
        told(&mut a).await.as_deref(),
        Some("There is no such command.")
    );
    // Dots are chat.
    a.say("...").await;
    assert_eq!(said(&mut a).await.as_deref(), Some("..."));
}

#[tokio::test]
async fn without_player_commands_a_players_dot_line_is_chat() {
    let realm = realm_with(false);
    let mut a = player(&realm, "A", "Alice").await;
    a.say(".help").await;
    assert_eq!(said(&mut a).await.as_deref(), Some(".help"));
}

/// mangos's `HandleAccountPasswordCommand`: the old password, then the
/// new one twice. A wrong old one, or two that differ, change nothing.
#[tokio::test]
async fn a_player_changes_their_own_password() {
    let accounts =
        crate::accounts::Accounts::in_memory(crate::accounts::Creation::Existing, false);
    accounts.set_password("ALICE", "old").unwrap();
    crate::accounts::install(accounts);
    let accounts = crate::accounts::get().unwrap();
    let realm = realm_with(true);
    let mut a = player(&realm, "ALICE", "Alice").await;
    a.say(".account password wrong new new").await;
    assert_eq!(
        told(&mut a).await.as_deref(),
        Some("The old password is wrong.")
    );
    a.say(".account password old new other").await;
    assert_eq!(
        told(&mut a).await.as_deref(),
        Some("The new passwords don't match.")
    );
    assert!(accounts.check_password("ALICE", "old"));
    a.say(".account password old new new").await;
    assert_eq!(
        told(&mut a).await.as_deref(),
        Some("Your password is changed.")
    );
    assert!(accounts.check_password("ALICE", "new"));
    assert!(!accounts.check_password("ALICE", "old"));
}
