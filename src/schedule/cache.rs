use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const BASE: &str = "data/gtfs";

/// Stable on-disk layout for one feed: `data/gtfs/<ident>/`.
#[derive(Clone)]
pub struct FeedDirs {
    pub root: PathBuf,
    pub zip: PathBuf,
    /// Where a download goes until it's complete and replaces `zip`.
    pub part: PathBuf,
    pub meta: PathBuf,
}

impl FeedDirs {
    pub fn new(ident: &str) -> Self {
        let root = Path::new(BASE).join(ident);
        Self {
            zip: root.join("static.zip"),
            part: root.join("static.zip.part"),
            meta: root.join("static.meta"),
            root,
        }
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.root).with_context(|| format!("create {}", self.root.display()))
    }

    pub fn read_validator(&self) -> Result<Option<Validator>> {
        match fs::read_to_string(&self.meta) {
            Ok(s) => Ok(Validator::parse(&s)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("read {}", self.meta.display())),
        }
    }

    pub fn write_validator(&self, v: &Validator) -> Result<()> {
        fs::write(&self.meta, v.serialize())
            .with_context(|| format!("write {}", self.meta.display()))
    }
}

/// What we compare against to decide whether a re-download needs processing.
/// Stored next to the zip so a restart can still skip unchanged feeds.
#[derive(Clone, Debug)]
pub enum Validator {
    Etag(String),
    Hash(String),
}

impl Validator {
    fn parse(s: &str) -> Option<Self> {
        let mut lines = s.lines();
        let kind = lines.next()?.trim();
        let value = lines.next()?.trim().to_owned();
        match kind {
            "etag" => Some(Validator::Etag(value)),
            "hash" => Some(Validator::Hash(value)),
            _ => None,
        }
    }

    fn serialize(&self) -> String {
        match self {
            Validator::Etag(v) => format!("etag\n{v}\n"),
            Validator::Hash(v) => format!("hash\n{v}\n"),
        }
    }
}
