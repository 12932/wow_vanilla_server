//! SRP6 client auth + realm list fetch.
//!
//! Speaks login protocol 3, as the real 1.12.1 client does (it uses 2 only to
//! reconnect). Protocol 3 adds a security flag to the challenge reply and the
//! proof; a server that asks for a PIN or another second factor is refused,
//! since a bot has none.

use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::timeout;
use wow_login_messages::ClientMessage as _;
use wow_login_messages::all::{
    CMD_AUTH_LOGON_CHALLENGE_Client, Locale, Os, Platform, ProtocolVersion, Version,
};
use wow_login_messages::helper::tokio_expect_server_message;
use wow_login_messages::version_3::{
    CMD_AUTH_LOGON_CHALLENGE_Server, CMD_AUTH_LOGON_CHALLENGE_Server_LoginResult,
    CMD_AUTH_LOGON_CHALLENGE_Server_SecurityFlag, CMD_AUTH_LOGON_PROOF_Client,
    CMD_AUTH_LOGON_PROOF_Client_SecurityFlag, CMD_AUTH_LOGON_PROOF_Server,
    CMD_AUTH_LOGON_PROOF_Server_LoginResult, CMD_REALM_LIST_Client, CMD_REALM_LIST_Server,
};
use wow_srp::client::SrpClientUser;
use wow_srp::normalized_string::NormalizedString;

/// The 1.12.1 client's build.
const CLIENT_BUILD: u16 = 5875;
const CLIENT_VERSION: Version = Version {
    major: 1,
    minor: 12,
    patch: 1,
    build: CLIENT_BUILD,
};

/// Each step (connect, every read and write) must finish within this, so a
/// server that stalls mid-login fails the bot instead of hanging it.
const STEP_TIMEOUT: Duration = Duration::from_secs(10);

/// Run one auth step under [`STEP_TIMEOUT`].
async fn step<T, E: std::fmt::Debug>(
    what: &str,
    fut: impl std::future::Future<Output = Result<T, E>>,
) -> std::io::Result<T> {
    match timeout(STEP_TIMEOUT, fut).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(std::io::Error::other(format!("auth: {what}: {e:?}"))),
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("auth: {what}: timed out"),
        )),
    }
}

pub struct AuthOutcome {
    pub session_key: [u8; 40],
    pub realm_address: String,
}

pub async fn perform(
    auth_addr: &str,
    username: &str,
    password: &str,
) -> std::io::Result<AuthOutcome> {
    let mut stream = step("connect", TcpStream::connect(auth_addr)).await?;

    let challenge = CMD_AUTH_LOGON_CHALLENGE_Client {
        protocol_version: ProtocolVersion::Three,
        version: CLIENT_VERSION,
        platform: Platform::X86,
        os: Os::Windows,
        locale: Locale::EnGb,
        utc_timezone_offset: 0,
        client_ip_address: Ipv4Addr::new(127, 0, 0, 1),
        account_name: username.to_string(),
    };
    step("write challenge", challenge.tokio_write(&mut stream)).await?;

    let resp: CMD_AUTH_LOGON_CHALLENGE_Server = step(
        "read challenge reply",
        tokio_expect_server_message::<CMD_AUTH_LOGON_CHALLENGE_Server, _>(&mut stream),
    )
    .await?;
    let (server_public_key, generator, large_safe_prime, salt) = match resp.result {
        CMD_AUTH_LOGON_CHALLENGE_Server_LoginResult::Success {
            server_public_key,
            generator,
            large_safe_prime,
            salt,
            security_flag,
            ..
        } => {
            if !matches!(security_flag, CMD_AUTH_LOGON_CHALLENGE_Server_SecurityFlag::None) {
                return Err(std::io::Error::other(format!(
                    "auth: server asks for a second factor ({security_flag:?}); bots have none"
                )));
            }
            (server_public_key, generator, large_safe_prime, salt)
        }
        other => {
            return Err(std::io::Error::other(format!(
                "auth: server rejected logon challenge: {other:?}"
            )));
        }
    };

    let generator = *generator.first().ok_or_else(|| {
        std::io::Error::other("auth: empty generator in challenge response")
    })?;

    let large_safe_prime: [u8; 32] = large_safe_prime.as_slice().try_into().map_err(|_| {
        std::io::Error::other(format!(
            "auth: expected 32-byte large safe prime, got {} bytes",
            large_safe_prime.len()
        ))
    })?;

    let username_norm = NormalizedString::new(username.to_string()).map_err(|e| {
        std::io::Error::other(format!("auth: username not normalizable: {e:?}"))
    })?;
    let password_norm = NormalizedString::new(password.to_string()).map_err(|e| {
        std::io::Error::other(format!("auth: password not normalizable: {e:?}"))
    })?;

    let client_public_key = wow_srp::PublicKey::from_le_bytes(server_public_key)
        .map_err(|e| std::io::Error::other(format!("auth: bad server pubkey: {e:?}")))?;

    let challenge_state = SrpClientUser::new(username_norm, password_norm).into_challenge(
        generator,
        large_safe_prime,
        client_public_key,
        salt,
    );

    let proof_client = CMD_AUTH_LOGON_PROOF_Client {
        client_public_key: *challenge_state.client_public_key(),
        client_proof: *challenge_state.client_proof(),
        crc_hash: [0u8; 20],
        telemetry_keys: Vec::new(),
        security_flag: CMD_AUTH_LOGON_PROOF_Client_SecurityFlag::None,
    };
    step("write proof", proof_client.tokio_write(&mut stream)).await?;

    let proof_server: CMD_AUTH_LOGON_PROOF_Server = step(
        "read proof reply",
        tokio_expect_server_message::<CMD_AUTH_LOGON_PROOF_Server, _>(&mut stream),
    )
    .await?;
    let server_proof = match proof_server.result {
        CMD_AUTH_LOGON_PROOF_Server_LoginResult::Success { server_proof, .. } => server_proof,
        other => {
            return Err(std::io::Error::other(format!(
                "auth: server rejected proof: {other:?}"
            )));
        }
    };

    let client = challenge_state
        .verify_server_proof(server_proof)
        .map_err(|e| std::io::Error::other(format!("auth: server proof mismatch: {e:?}")))?;
    let session_key = client.session_key();

    let realm_list_req = CMD_REALM_LIST_Client {};
    step("write realm list request", realm_list_req.tokio_write(&mut stream)).await?;

    let realm_list: CMD_REALM_LIST_Server = step(
        "read realm list",
        tokio_expect_server_message::<CMD_REALM_LIST_Server, _>(&mut stream),
    )
    .await?;

    let realm = realm_list
        .realms
        .into_iter()
        .next()
        .ok_or_else(|| std::io::Error::other("auth: empty realm list"))?;

    // Auth socket no longer needed.
    drop(stream);

    Ok(AuthOutcome {
        session_key,
        realm_address: realm.address,
    })
}
