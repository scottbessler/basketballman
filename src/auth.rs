//! WebAuthn (passkey) registration and login ceremonies, mirroring screwball.
//!
//! Identity is established entirely by passkeys — there are no passwords. A
//! successful ceremony writes a signed session cookie (see [`crate::session`]).
//! The in-flight ceremony state is parked in a short-lived signed cookie so the
//! server stays stateless between the `begin` and `finish` round trips.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, SameSite, SignedCookieJar};
use chrono::Utc;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use uuid::Uuid;
use webauthn_rs::prelude::{
    PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
};

use crate::routes::AppState;
use crate::session::{SESSION_COOKIE, session_cookie};
use crate::users::{User, UserError};

/// Cookie holding the pending registration ceremony state.
const REG_COOKIE: &str = "reg_state";
/// Cookie holding the pending login ceremony state.
const AUTH_COOKIE: &str = "auth_state";

const MAX_USERNAME: usize = 32;
const MAX_DISPLAY_NAME: usize = 48;

/// A JSON error response for the auth endpoints (the client reads `error`).
pub struct AuthReject {
    status: StatusCode,
    message: String,
}

impl AuthReject {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
}

impl IntoResponse for AuthReject {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl From<UserError> for AuthReject {
    fn from(err: UserError) -> Self {
        let status = match err {
            UserError::UsernameTaken => StatusCode::CONFLICT,
            UserError::NotFound => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::new(status, err.to_string())
    }
}

#[derive(Deserialize)]
pub struct RegisterBegin {
    username: String,
    display_name: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct PendingRegistration {
    user_id: Uuid,
    username: String,
    display_name: String,
    state: PasskeyRegistration,
}

#[derive(Deserialize)]
pub struct LoginBegin {
    username: String,
}

#[derive(Serialize, Deserialize)]
struct PendingLogin {
    user_id: Uuid,
    state: PasskeyAuthentication,
}

/// Begin registration: validate the desired username, mint a user id, and hand
/// the browser a credential-creation challenge.
pub async fn register_begin(
    State(state): State<AppState>,
    jar: SignedCookieJar,
    Json(body): Json<RegisterBegin>,
) -> Result<Response, AuthReject> {
    let username = clean(&body.username, MAX_USERNAME)
        .ok_or_else(|| AuthReject::bad_request("pick a username (1–32 characters)"))?;
    let display_name = body
        .display_name
        .as_deref()
        .and_then(|name| clean(name, MAX_DISPLAY_NAME))
        .unwrap_or_else(|| username.clone());

    if state.users.username_taken(&username) {
        return Err(AuthReject::new(
            StatusCode::CONFLICT,
            "that username is already taken",
        ));
    }

    // Dev mode: no passkey, just create the account and sign in.
    if state.passkey_disabled {
        let user_id = Uuid::new_v4();
        let user = User {
            id: user_id,
            username,
            display_name,
            credentials: vec![],
            created_at: Utc::now(),
        };
        let display_name = user.display_name.clone();
        state.users.insert(user)?;
        let jar = jar.add(session_cookie(user_id));
        return Ok((jar, Json(json!({ "display_name": display_name }))).into_response());
    }

    let user_id = Uuid::new_v4();
    let (challenge, state_token) = state
        .webauthn
        .start_passkey_registration(user_id, &username, &display_name, None)
        .map_err(webauthn_failed)?;

    let pending = PendingRegistration {
        user_id,
        username,
        display_name,
        state: state_token,
    };
    let jar = jar.add(state_cookie(REG_COOKIE, &pending)?);
    Ok((jar, Json(challenge)).into_response())
}

/// Finish registration: verify the attestation, persist the new user with their
/// passkey, and sign them in.
pub async fn register_finish(
    State(state): State<AppState>,
    jar: SignedCookieJar,
    Json(credential): Json<RegisterPublicKeyCredential>,
) -> Result<(SignedCookieJar, Json<serde_json::Value>), AuthReject> {
    let pending: PendingRegistration = take_state(&jar, REG_COOKIE)
        .ok_or_else(|| AuthReject::bad_request("registration session expired; start again"))?;

    let passkey = state
        .webauthn
        .finish_passkey_registration(&credential, &pending.state)
        .map_err(webauthn_failed)?;

    let user = User {
        id: pending.user_id,
        username: pending.username,
        display_name: pending.display_name,
        credentials: vec![passkey],
        created_at: Utc::now(),
    };
    let display_name = user.display_name.clone();
    state.users.insert(user)?;

    let jar = jar
        .remove(removal_cookie(REG_COOKIE))
        .add(session_cookie(pending.user_id));
    Ok((jar, Json(json!({ "display_name": display_name }))))
}

/// Begin login: look up the user by username and challenge their passkeys.
pub async fn login_begin(
    State(state): State<AppState>,
    jar: SignedCookieJar,
    Json(body): Json<LoginBegin>,
) -> Result<Response, AuthReject> {
    let username = clean(&body.username, MAX_USERNAME)
        .ok_or_else(|| AuthReject::bad_request("enter your username"))?;
    let user = state
        .users
        .get_by_username(&username)
        .ok_or_else(|| AuthReject::new(StatusCode::NOT_FOUND, "no account with that username"))?;

    // Dev mode: skip the passkey assertion, sign in on username alone.
    if state.passkey_disabled {
        let jar = jar.add(session_cookie(user.id));
        return Ok((jar, Json(json!({ "display_name": user.display_name }))).into_response());
    }

    if user.credentials.is_empty() {
        return Err(AuthReject::bad_request(
            "that account has no passkeys registered",
        ));
    }

    let (challenge, state_token) = state
        .webauthn
        .start_passkey_authentication(&user.credentials)
        .map_err(webauthn_failed)?;

    let pending = PendingLogin {
        user_id: user.id,
        state: state_token,
    };
    let jar = jar.add(state_cookie(AUTH_COOKIE, &pending)?);
    Ok((jar, Json(challenge)).into_response())
}

/// Finish login: verify the assertion, bump the credential counter, and sign in.
pub async fn login_finish(
    State(state): State<AppState>,
    jar: SignedCookieJar,
    Json(credential): Json<PublicKeyCredential>,
) -> Result<(SignedCookieJar, Json<serde_json::Value>), AuthReject> {
    let pending: PendingLogin = take_state(&jar, AUTH_COOKIE)
        .ok_or_else(|| AuthReject::bad_request("login session expired; start again"))?;

    let result = state
        .webauthn
        .finish_passkey_authentication(&credential, &pending.state)
        .map_err(webauthn_failed)?;

    // Persist any counter advance so cloned-credential detection keeps working.
    if result.needs_update() {
        state.users.update(pending.user_id, |user| {
            for passkey in &mut user.credentials {
                passkey.update_credential(&result);
            }
        })?;
    }

    let display_name = state
        .users
        .get(pending.user_id)
        .map(|user| user.display_name)
        .unwrap_or_default();

    let jar = jar
        .remove(removal_cookie(AUTH_COOKIE))
        .add(session_cookie(pending.user_id));
    Ok((jar, Json(json!({ "display_name": display_name }))))
}

/// Clear the session cookie and return to the home page.
pub async fn logout(jar: SignedCookieJar) -> (SignedCookieJar, Redirect) {
    let jar = jar.remove(removal_cookie(SESSION_COOKIE));
    (jar, Redirect::to("/"))
}

/// A removal cookie that matches the `path=/` the cookies were created with, so
/// the browser actually clears them.
fn removal_cookie(name: &'static str) -> Cookie<'static> {
    Cookie::build((name, "")).path("/").build()
}

/// Trim and length-bound a free-text field, returning `None` when empty.
fn clean(raw: &str, max: usize) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().count() > max {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn webauthn_failed(_err: webauthn_rs::prelude::WebauthnError) -> AuthReject {
    AuthReject::bad_request("passkey verification failed; please try again")
}

/// Build a short-lived (session-scoped) signed cookie carrying ceremony state.
fn state_cookie<T: Serialize>(
    name: &'static str,
    value: &T,
) -> Result<Cookie<'static>, AuthReject> {
    let encoded = serde_json::to_string(value)
        .map_err(|err| AuthReject::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Cookie::build((name, encoded))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .build())
}

fn take_state<T: DeserializeOwned>(jar: &SignedCookieJar, name: &str) -> Option<T> {
    let cookie = jar.get(name)?;
    serde_json::from_str(cookie.value()).ok()
}
