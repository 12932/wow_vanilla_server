//! SRP6 client auth + realm list fetch.

use std::net::Ipv4Addr;

use tokio::net::TcpStream;
use wow_login_messages::ClientMessage as _;
use wow_login_messages::all::{
    CMD_AUTH_LOGON_CHALLENGE_Client, Locale, Os, Platform, ProtocolVersion, Version,
};
use wow_login_messages::helper::tokio_expect_server_message;
use wow_login_messages::version_2::{
    CMD_AUTH_LOGON_CHALLENGE_Server, CMD_AUTH_LOGON_CHALLENGE_Server_LoginResult,
    CMD_AUTH_LOGON_PROOF_Client, CMD_AUTH_LOGON_PROOF_Server,
    CMD_AUTH_LOGON_PROOF_Server_LoginResult, CMD_REALM_LIST_Client, CMD_REALM_LIST_Server,
};
use wow_srp::client::SrpClientUser;
use wow_srp::normalized_string::NormalizedString;

/// Vanilla 1.12.2 client build.
const CLIENT_BUILD: u16 = 5875;
const CLIENT_VERSION: Version = Version {
    major: 1,
    minor: 12,
    patch: 2,
    build: CLIENT_BUILD,
};

pub struct AuthOutcome {
    pub session_key: [u8; 40],
    pub realm_address: String,
}

pub async fn perform(auth_addr: &str, username: &str) -> std::io::Result<AuthOutcome> {
    let mut stream = TcpStream::connect(auth_addr).await?;

    let challenge = CMD_AUTH_LOGON_CHALLENGE_Client {
        protocol_version: ProtocolVersion::Two,
        version: CLIENT_VERSION,
        platform: Platform::X86,
        os: Os::Windows,
        locale: Locale::EnGb,
        utc_timezone_offset: 0,
        client_ip_address: Ipv4Addr::new(127, 0, 0, 1),
        account_name: username.to_string(),
    };
    challenge
        .tokio_write(&mut stream)
        .await
        .map_err(|e| std::io::Error::other(format!("auth: write challenge: {e}")))?;

    let resp: CMD_AUTH_LOGON_CHALLENGE_Server =
        tokio_expect_server_message::<CMD_AUTH_LOGON_CHALLENGE_Server, _>(&mut stream)
            .await
            .map_err(|e| std::io::Error::other(format!("auth: read challenge resp: {e:?}")))?;
    let (server_public_key, generator, large_safe_prime, salt) = match resp.result {
        CMD_AUTH_LOGON_CHALLENGE_Server_LoginResult::Success {
            server_public_key,
            generator,
            large_safe_prime,
            salt,
            ..
        } => (server_public_key, generator, large_safe_prime, salt),
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
    let password_norm = NormalizedString::new(username.to_string()).map_err(|e| {
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
    };
    proof_client
        .tokio_write(&mut stream)
        .await
        .map_err(|e| std::io::Error::other(format!("auth: write proof: {e}")))?;

    let proof_server: CMD_AUTH_LOGON_PROOF_Server =
        tokio_expect_server_message::<CMD_AUTH_LOGON_PROOF_Server, _>(&mut stream)
            .await
            .map_err(|e| std::io::Error::other(format!("auth: read proof resp: {e:?}")))?;
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
    realm_list_req
        .tokio_write(&mut stream)
        .await
        .map_err(|e| std::io::Error::other(format!("auth: write realm list req: {e}")))?;

    let realm_list: CMD_REALM_LIST_Server =
        tokio_expect_server_message::<CMD_REALM_LIST_Server, _>(&mut stream)
            .await
            .map_err(|e| std::io::Error::other(format!("auth: read realm list: {e:?}")))?;

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
