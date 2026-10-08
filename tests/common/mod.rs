//! Shared helpers for integration tests that drive the HTTP app.
#![allow(dead_code)]

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum_extra::extract::cookie::Key;
use basketballman::models::League;
use basketballman::repo::LeagueRepository;
use basketballman::routes::{AppState, app};
use basketballman::users::UserStore;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tower::ServiceExt;
use webauthn_rs::WebauthnBuilder;
use webauthn_rs::prelude::Url;

pub struct TestApp {
    pub router: Router,
    pub state: AppState,
}

pub fn temp_path(name: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("basketballman-{nanos}-{name}"))
}

pub fn test_app(league: League) -> TestApp {
    let repo = LeagueRepository::new(temp_path("league.json"));
    let users_dir = temp_path("users");
    std::fs::create_dir_all(&users_dir).unwrap();
    let rp_origin = Url::parse("http://localhost:3000").unwrap();
    let webauthn = WebauthnBuilder::new("localhost", &rp_origin)
        .unwrap()
        .build()
        .unwrap();
    let state = AppState {
        repo,
        league: Arc::new(Mutex::new(league)),
        users: Arc::new(UserStore::load(&users_dir).unwrap()),
        webauthn: Arc::new(webauthn),
        key: Key::generate(),
        passkey_disabled: true,
    };
    TestApp {
        router: app(state.clone()),
        state,
    }
}

pub struct Reply {
    pub status: StatusCode,
    pub location: Option<String>,
    pub body: String,
    pub set_cookie: Option<String>,
}

impl TestApp {
    pub async fn send(&self, mut request: Request<Body>, cookie: Option<&str>) -> Reply {
        if let Some(cookie) = cookie {
            request
                .headers_mut()
                .insert(header::COOKIE, cookie.parse().unwrap());
        }
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_string());
        let set_cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string());
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        Reply {
            status,
            location,
            body: String::from_utf8_lossy(&bytes).to_string(),
            set_cookie,
        }
    }

    pub async fn get(&self, uri: &str, cookie: Option<&str>) -> Reply {
        let request = Request::builder().uri(uri).body(Body::empty()).unwrap();
        self.send(request, cookie).await
    }

    pub async fn post_form(&self, uri: &str, form: &[(&str, &str)], cookie: Option<&str>) -> Reply {
        let body = form
            .iter()
            .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let request = Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        self.send(request, cookie).await
    }

    pub async fn post(&self, uri: &str, cookie: Option<&str>) -> Reply {
        self.post_form(uri, &[], cookie).await
    }

    /// Register a dev-mode user and return their session cookie.
    pub async fn sign_up(&self, username: &str) -> String {
        let request = Request::builder()
            .method("POST")
            .uri("/auth/register/begin")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(format!(r#"{{"username":"{username}"}}"#)))
            .unwrap();
        let reply = self.send(request, None).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        reply.set_cookie.expect("session cookie")
    }
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
