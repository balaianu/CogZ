//! Entity file I/O — read and write canonical markdown entity files.
//!
//! Each entity file has YAML frontmatter (id, title, type, status,
//! timestamps, references) and a markdown body. Type-specific fields
//! live in the frontmatter alongside the common fields.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::FrontmatterError;
use super::frontmatter::{FmValue, Frontmatter};

/// Convert a frontmatter value to a JSON value for storage in the
/// entity `properties` field.
pub fn fm_value_to_json(value: &FmValue) -> serde_json::Value {
    match value {
        FmValue::String(s) => serde_json::Value::String(s.clone()),
        FmValue::Float(f) => {
            serde_json::Value::Number(serde_json::Number::from_f64(*f).unwrap_or(0.into()))
        }
        FmValue::Int(i) => serde_json::Value::Number((*i).into()),
        FmValue::Bool(b) => serde_json::Value::Bool(*b),
        FmValue::Array(a) => serde_json::Value::Array(
            a.iter()
                .map(|s| serde_json::Value::String(s.clone()))
                .collect(),
        ),
    }
}

/// Maximum slug length (per entity-spec.md).
const MAX_SLUG_LEN: usize = 60;

/// Generate a slug from a title: lowercase, spaces → hyphens,
/// non-alphanumeric stripped, max 60 chars.
pub fn slugify(title: &str) -> String {
    let slugified = slug::slugify(title);
    truncate_slug(&slugified)
}

/// Truncate slug to MAX_SLUG_LEN, breaking on hyphen boundaries
/// when possible.
fn truncate_slug(s: &str) -> String {
    if s.len() <= MAX_SLUG_LEN {
        return s.to_string();
    }

    // Try to break at a hyphen within the limit
    let prefix = &s[..MAX_SLUG_LEN];
    if let Some(pos) = prefix.rfind('-')
        && pos > 0
    {
        return s[..pos].to_string();
    }
    prefix.to_string()
}

/// Append a short hash suffix to avoid slug collisions.
pub fn slug_with_hash(slug: &str, hash_seed: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(hash_seed.as_bytes());
    let hash = hasher.finalize();
    let suffix = &hash[..2]
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    format!("{}-{}", slug, suffix)
}

/// The entity types that are file-backed (not code entities).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileEntityType {
    Observation,
    Rule,
    Knowledge,
}

impl FileEntityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Observation => "observation",
            Self::Rule => "rule",
            Self::Knowledge => "knowledge",
        }
    }
}

impl std::str::FromStr for FileEntityType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "observation" => Ok(Self::Observation),
            "rule" => Ok(Self::Rule),
            "knowledge" => Ok(Self::Knowledge),
            _ => Err(format!("invalid file entity type: {}", s)),
        }
    }
}

/// A parsed entity file — frontmatter + body.
#[derive(Debug, Clone)]
pub struct EntityFile {
    pub id: String,
    pub title: String,
    pub entity_type: FileEntityType,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub references: Vec<String>,
    /// All frontmatter entries, including type-specific fields.
    pub frontmatter: Frontmatter,
    /// The markdown body (content after frontmatter).
    pub body: String,
}

impl EntityFile {
    /// Create a new entity file with a fresh UUID and current timestamps.
    pub fn new(title: &str, entity_type: FileEntityType, body: &str) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        let id = Uuid::new_v4().to_string();
        Self {
            id,
            title: title.to_string(),
            entity_type,
            status: "active".to_string(),
            created_at: now.clone(),
            updated_at: now,
            references: Vec::new(),
            frontmatter: Frontmatter::new(),
            body: body.to_string(),
        }
    }

    /// Build the full file content (frontmatter + body).
    pub fn to_file_content(&self) -> String {
        // Start with common fields in canonical order
        let mut fm = Frontmatter::new();
        fm.insert("id", FmValue::String(self.id.clone()));
        fm.insert("title", FmValue::String(self.title.clone()));
        fm.insert(
            "type",
            FmValue::String(self.entity_type.as_str().to_string()),
        );
        fm.insert("status", FmValue::String(self.status.clone()));
        fm.insert("created_at", FmValue::String(self.created_at.clone()));
        fm.insert("updated_at", FmValue::String(self.updated_at.clone()));
        fm.insert("references", FmValue::Array(self.references.clone()));

        // Merge in any extra frontmatter fields (type-specific)
        for (key, value) in &self.frontmatter.entries {
            // Skip fields we've already set
            if !matches!(
                key.as_str(),
                "id" | "title" | "type" | "status" | "created_at" | "updated_at" | "references"
            ) {
                fm.insert(key, value.clone());
            }
        }

        let fm_text = super::frontmatter::serialize(&fm);
        format!("---\n{}---\n\n{}", fm_text, self.body)
    }

    /// Parse an entity file from its full text content.
    pub fn from_content(content: &str) -> Result<Self, FrontmatterError> {
        let (fm_text, body) = super::frontmatter::split_frontmatter(content)?;
        let fm = super::frontmatter::parse(&fm_text)?;

        let id = fm
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or(FrontmatterError::Parse {
                line: 0,
                message: "missing required field: id".to_string(),
            })?
            .to_string();

        let title = fm
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or(FrontmatterError::Parse {
                line: 0,
                message: "missing required field: title".to_string(),
            })?
            .to_string();

        let type_str = fm
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or(FrontmatterError::Parse {
                line: 0,
                message: "missing required field: type".to_string(),
            })?;

        let entity_type =
            type_str
                .parse::<FileEntityType>()
                .map_err(|_| FrontmatterError::Parse {
                    line: 0,
                    message: format!("invalid entity type: {}", type_str),
                })?;

        let status = fm
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("active")
            .to_string();

        let created_at = fm
            .get("created_at")
            .and_then(|v| v.as_str())
            .ok_or(FrontmatterError::Parse {
                line: 0,
                message: "missing required field: created_at".to_string(),
            })?
            .to_string();

        let updated_at = fm
            .get("updated_at")
            .and_then(|v| v.as_str())
            .ok_or(FrontmatterError::Parse {
                line: 0,
                message: "missing required field: updated_at".to_string(),
            })?
            .to_string();

        let references = fm
            .get("references")
            .and_then(|v| v.as_array())
            .map(|a| a.to_vec())
            .unwrap_or_default();

        Ok(Self {
            id,
            title,
            entity_type,
            status,
            created_at,
            updated_at,
            references,
            frontmatter: fm,
            body,
        })
    }

    /// Compute the expected file path for this entity.
    ///
    /// - Knowledge: `.cogz/knowledge/<category>/<slug>.md`
    /// - Rule: `.cogz/rules/<slug>.md`
    /// - Observation: `.cogz/observations/<year-month>/<uuid>.md`
    pub fn file_path(&self, cogz_dir: &Path) -> PathBuf {
        match self.entity_type {
            FileEntityType::Knowledge => {
                let category = self
                    .frontmatter
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("uncategorized");
                let slug = slugify(&self.title);
                cogz_dir
                    .join("knowledge")
                    .join(category)
                    .join(format!("{}.md", slug))
            }
            FileEntityType::Rule => {
                let slug = slugify(&self.title);
                cogz_dir.join("rules").join(format!("{}.md", slug))
            }
            FileEntityType::Observation => {
                // Extract year-month from created_at (format: 2026-08-27T...)
                let year_month = self.created_at.get(..7).unwrap_or("unknown");
                cogz_dir
                    .join("observations")
                    .join(year_month)
                    .join(format!("{}.md", self.id))
            }
        }
    }

    /// Compute the file path, appending a short hash suffix if a
    /// different entity already owns the slug-based path. This
    /// prevents silent overwrites when two knowledge entries or rules
    /// have titles that slugify identically.
    ///
    /// Observations use UUID-based filenames and never collide.
    pub fn file_path_safe(&self, cogz_dir: &Path) -> PathBuf {
        let path = self.file_path(cogz_dir);
        if self.entity_type == FileEntityType::Observation {
            return path;
        }
        if !path.exists() {
            return path;
        }
        // File exists — check if it belongs to this entity already.
        if let Ok(existing) = read_entity_file(&path)
            && existing.id == self.id
        {
            return path; // Same entity, safe to overwrite.
        }
        // Collision: a different entity owns this path. Use slug + hash.
        let slug = slugify(&self.title);
        let hashed = slug_with_hash(&slug, &self.id);
        match self.entity_type {
            FileEntityType::Knowledge => {
                let category = self
                    .frontmatter
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("uncategorized");
                cogz_dir
                    .join("knowledge")
                    .join(category)
                    .join(format!("{}.md", hashed))
            }
            FileEntityType::Rule => cogz_dir.join("rules").join(format!("{}.md", hashed)),
            FileEntityType::Observation => path, // unreachable
        }
    }
}

/// Read an entity file from disk.
pub fn read_entity_file(path: &Path) -> Result<EntityFile, FrontmatterError> {
    let content = std::fs::read_to_string(path).map_err(|e| FrontmatterError::Parse {
        line: 0,
        message: format!("failed to read {}: {}", path.display(), e),
    })?;
    EntityFile::from_content(&content)
}

/// Write an entity file to disk. Creates parent directories as needed.
pub fn write_entity_file(path: &Path, entity: &EntityFile) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = entity.to_file_content();
    std::fs::write(path, content)
}
