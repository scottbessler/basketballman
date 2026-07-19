//! Registered owners. Identity is established by passkeys (see [`crate::auth`]);
//! users are persisted as one JSON file each under `<data>/users/`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;
use webauthn_rs::prelude::Passkey;

#[derive(Debug, Error)]
pub enum UserError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("that username is already taken")]
    UsernameTaken,
    #[error("user not found")]
    NotFound,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub display_name: String,
    pub credentials: Vec<Passkey>,
    pub created_at: DateTime<Utc>,
}

/// Normalize a login handle so lookups are case- and whitespace-insensitive.
pub fn normalize_username(raw: &str) -> String {
    raw.trim().to_lowercase()
}

struct Index {
    by_id: HashMap<Uuid, User>,
    by_username: HashMap<String, Uuid>,
}

/// In-memory registry of users backed by per-user JSON files on disk.
pub struct UserStore {
    index: Mutex<Index>,
    dir: PathBuf,
}

impl UserStore {
    /// Load all persisted users under `<data_root>/users`, creating the
    /// directory if needed. Invalid files are skipped.
    pub fn load(data_root: impl Into<PathBuf>) -> Result<Self, UserError> {
        let dir = data_root.into().join("users");
        std::fs::create_dir_all(&dir)?;

        let mut by_id = HashMap::new();
        let mut by_username = HashMap::new();
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(user) = serde_json::from_str::<User>(&text) else {
                continue;
            };
            by_username.insert(normalize_username(&user.username), user.id);
            by_id.insert(user.id, user);
        }

        Ok(Self {
            index: Mutex::new(Index { by_id, by_username }),
            dir,
        })
    }

    pub fn get(&self, id: Uuid) -> Option<User> {
        self.index
            .lock()
            .expect("user index")
            .by_id
            .get(&id)
            .cloned()
    }

    pub fn get_by_username(&self, username: &str) -> Option<User> {
        let key = normalize_username(username);
        let guard = self.index.lock().expect("user index");
        let id = guard.by_username.get(&key)?;
        guard.by_id.get(id).cloned()
    }

    pub fn username_taken(&self, username: &str) -> bool {
        let key = normalize_username(username);
        self.index
            .lock()
            .expect("user index")
            .by_username
            .contains_key(&key)
    }

    /// Persist a new user, then commit it to memory and the username index.
    pub fn insert(&self, user: User) -> Result<(), UserError> {
        let key = normalize_username(&user.username);
        let mut guard = self.index.lock().expect("user index");
        if guard.by_username.contains_key(&key) {
            return Err(UserError::UsernameTaken);
        }
        self.persist(&user)?;
        guard.by_username.insert(key, user.id);
        guard.by_id.insert(user.id, user);
        Ok(())
    }

    /// Mutate a user under the registry lock, persisting before the change
    /// becomes visible in memory.
    pub fn update<R>(&self, id: Uuid, f: impl FnOnce(&mut User) -> R) -> Result<R, UserError> {
        let mut guard = self.index.lock().expect("user index");
        let mut working = guard.by_id.get(&id).ok_or(UserError::NotFound)?.clone();
        let outcome = f(&mut working);
        self.persist(&working)?;
        guard.by_id.insert(id, working);
        Ok(outcome)
    }

    fn persist(&self, user: &User) -> Result<(), UserError> {
        let path = self.dir.join(format!("{}.json", user.id));
        let tmp = temp_path(&path);
        let bytes = serde_json::to_vec_pretty(user)?;
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

fn temp_path(path: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    path.with_extension(format!("json.tmp-{nanos}"))
}
