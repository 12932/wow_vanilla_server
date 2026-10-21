//! Round 12, stage 12k: a character flagged for a new name takes one at
//! the character screen, as mangos-classic's `.character rename`
//! (`AT_LOGIN_RENAME`) and `HandleCharRenameOpcode`. `CMSG_CHAR_RENAME`
//! wasn't handled: the only fix for a bad name was to delete the
//! character.

use wow_world_messages::Guid;
use wow_world_messages::vanilla::opcodes::ServerOpcodeMessage;
use wow_world_messages::vanilla::*;

use super::harness::{self, Client};

async fn logout(c: &mut Client) {
    c.send(CMSG_LOGOUT_REQUEST {}).await;
    c.wait_for("SMSG_LOGOUT_COMPLETE", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_LOGOUT_COMPLETE).then_some(())
    })
    .await;
}

/// The character list's entry for `g`: its name and flags.
async fn listed(c: &mut Client, g: Guid) -> (String, u32) {
    c.clear();
    c.send(CMSG_CHAR_ENUM {}).await;
    c.wait_for("SMSG_CHAR_ENUM", |m| match m {
        ServerOpcodeMessage::SMSG_CHAR_ENUM(e) => e
            .characters
            .iter()
            .find(|x| x.guid == g)
            .map(|x| (x.name.clone(), x.flags.as_int())),
        _ => None,
    })
    .await
}

async fn rename(c: &mut Client, g: Guid, name: &str) -> SMSG_CHAR_RENAME_WorldResult {
    c.clear();
    c.send(CMSG_CHAR_RENAME {
        character: g,
        new_name: name.into(),
    })
    .await;
    c.wait_for("SMSG_CHAR_RENAME", |m| match m {
        ServerOpcodeMessage::SMSG_CHAR_RENAME(r) => Some(r.result.clone()),
        _ => None,
    })
    .await
}

/// A GM's `.rename` flags an offline character: the list shows it to be
/// renamed, a login is refused, and a new name is taken (a bad one, or
/// one in use, refused); then it logs in under it, and the GM, online, is
/// told to forget the old.
#[tokio::test]
async fn a_character_flagged_by_a_gm_takes_a_new_name_at_the_character_screen() {
    let realm = harness::realm_with(crate::game::data::GameData::empty());
    let mut gm = Client::connect(&realm, "P12KGM");
    gm.enter("Warden", Race::Human, Class::Warrior).await;
    let mut c = Client::connect(&realm, "P12KBAD");
    let g = c.enter("Rudename", Race::Human, Class::Warrior).await;
    logout(&mut c).await;
    assert_eq!(listed(&mut c, g).await.1 & CharacterFlags::RENAME, 0);
    gm.say(".rename Rudename").await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if listed(&mut c, g).await.1 & CharacterFlags::RENAME != 0 {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "never flagged");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    c.clear();
    c.send(CMSG_PLAYER_LOGIN { guid: g }).await;
    c.wait_for("SMSG_CHARACTER_LOGIN_FAILED", |m| {
        matches!(m, ServerOpcodeMessage::SMSG_CHARACTER_LOGIN_FAILED(_)).then_some(())
    })
    .await;
    assert_eq!(
        rename(&mut c, g, "X").await,
        SMSG_CHAR_RENAME_WorldResult::CharNameTooShort
    );
    assert_eq!(
        rename(&mut c, g, "Warden").await,
        SMSG_CHAR_RENAME_WorldResult::CharCreateNameInUse
    );
    gm.clear();
    assert_eq!(
        rename(&mut c, g, "kindname").await,
        SMSG_CHAR_RENAME_WorldResult::ResponseSuccess {
            character: g,
            new_name: "Kindname".into(),
        }
    );
    gm.wait_for("SMSG_INVALIDATE_PLAYER", |m| match m {
        ServerOpcodeMessage::SMSG_INVALIDATE_PLAYER(i) if i.guid == g => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(listed(&mut c, g).await, ("Kindname".to_owned(), 0));
    assert_eq!(realm.registry.guid_by_name("Kindname"), Some(g));
    assert_eq!(realm.registry.guid_by_name("Rudename"), None);
    // Renamed once: no second time without a GM's say.
    assert_eq!(
        rename(&mut c, g, "Othername").await,
        SMSG_CHAR_RENAME_WorldResult::CharCreateError
    );
    c.clear();
    c.login(g).await;
    let row = {
        logout(&mut c).await;
        realm.store.load(crate::game::guid::counter(g)).await.unwrap()
    };
    assert_eq!(row.name, "Kindname");
}

/// Another account's character isn't renamed, flagged or not.
#[tokio::test]
async fn another_accounts_character_isnt_renamed() {
    let realm = harness::realm_with(crate::game::data::GameData::empty());
    let mut owner = Client::connect(&realm, "P12KOWNER");
    let g = owner.enter("Ownedname", Race::Human, Class::Warrior).await;
    logout(&mut owner).await;
    let mut gm = Client::connect(&realm, "P12KGM2");
    gm.enter("Warder", Race::Human, Class::Warrior).await;
    gm.say(".rename Ownedname").await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while listed(&mut owner, g).await.1 & CharacterFlags::RENAME == 0 {
        assert!(tokio::time::Instant::now() < deadline, "never flagged");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let mut other = Client::connect(&realm, "P12KTHIEF");
    other.enter("Thief", Race::Human, Class::Warrior).await;
    logout(&mut other).await;
    assert_eq!(
        rename(&mut other, g, "Stolenname").await,
        SMSG_CHAR_RENAME_WorldResult::CharCreateError
    );
    assert_eq!(realm.registry.guid_by_name("Ownedname"), Some(g));
}
