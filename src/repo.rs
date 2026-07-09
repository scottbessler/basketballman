use crate::generator::generate_league;
use crate::models::League;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RepoError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct LeagueRepository {
    path: PathBuf,
}

impl LeagueRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn load_or_generate(&self, seed: u64) -> Result<League, RepoError> {
        if self.path.exists() {
            self.load()
        } else {
            let league = generate_league(seed);
            self.save(&league)?;
            Ok(league)
        }
    }

    pub fn load(&self) -> Result<League, RepoError> {
        let body = fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&body)?)
    }

    pub fn save(&self, league: &League) -> Result<(), RepoError> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(league)?;
        fs::write(&self.path, body)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
