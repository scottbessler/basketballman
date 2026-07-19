use axum_extra::extract::cookie::Key;
use basketballman::repo::LeagueRepository;
use basketballman::routes::{AppState, app};
use basketballman::users::UserStore;
use std::env;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use webauthn_rs::prelude::{Url, WebauthnBuilder};

const LOCAL_DEV_SESSION_SECRET: &str =
    "basketballman-local-development-session-secret-v1-keep-browser-sessions-across-restarts";

#[tokio::main]
async fn main() {
    let data_path = env::var("DATA_PATH").unwrap_or_else(|_| "data".to_string());
    let league_path = Path::new(&data_path).join("league.json");
    let repo = LeagueRepository::new(&league_path);
    let league = repo
        .load_or_generate(7)
        .expect("league state should load or generate");
    let users = UserStore::load(&data_path).expect("user store should load");
    let state = AppState {
        repo,
        league: Arc::new(Mutex::new(league)),
        users: Arc::new(users),
        webauthn: Arc::new(build_webauthn()),
        key: load_key(),
        passkey_disabled: env_flag("PASSKEY_DISABLED"),
    };
    let app = app(state);
    let port = env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr: SocketAddr = format!("0.0.0.0:{port}")
        .parse()
        .expect("server address should parse");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("server should bind");

    println!("basketballman listening on http://{addr}");
    axum::serve(listener, app).await.expect("server should run");
}

/// Build the WebAuthn relying party from `RP_ID` / `RP_ORIGIN` (defaults suit
/// local development on `http://localhost:3000`).
fn build_webauthn() -> webauthn_rs::prelude::Webauthn {
    let rp_id = env::var("RP_ID").unwrap_or_else(|_| "localhost".to_string());
    let rp_origin_raw =
        env::var("RP_ORIGIN").unwrap_or_else(|_| "http://localhost:3000".to_string());
    let rp_origin = Url::parse(&rp_origin_raw).expect("RP_ORIGIN must be a valid URL");
    WebauthnBuilder::new(&rp_id, &rp_origin)
        .expect("valid WebAuthn relying-party configuration")
        .rp_name("Basketballman")
        .build()
        .expect("WebAuthn relying party should build")
}

/// Derive the cookie-signing key from `SESSION_SECRET`. Debug builds use a
/// stable local-dev fallback so `cargo run` restarts keep browser sessions.
fn load_key() -> Key {
    match env::var("SESSION_SECRET") {
        Ok(secret) if secret.len() >= 64 => Key::from(secret.as_bytes()),
        _ if cfg!(debug_assertions) => Key::from(LOCAL_DEV_SESSION_SECRET.as_bytes()),
        _ => Key::generate(),
    }
}

/// Read a boolean env flag (`1`/`true`, case-insensitive); absent or anything
/// else is false.
fn env_flag(name: &str) -> bool {
    matches!(
        env::var(name).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("True")
    )
}
