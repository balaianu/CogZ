//! Minimal YAML frontmatter parser for entity files.
//!
//! Handles only the subset used by CogZ frontmatter: scalar key-value
//! pairs and inline string arrays. No nested mappings, no anchors, no
//! multi-line strings. This avoids a full YAML dependency for what is
//! a very constrained format.

use std::fmt::Write;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub enum FmValue {
    String(String),
    Float(f64),
    Int(i64),
    Bool(bool),
    Array(Vec<String>),
}

impl FmValue {
    /// Get as a string if this is a String value.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// Get as a float if applicable.
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Get as an int if applicable.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Get as a bool if applicable.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Get as an array of strings if applicable.
    pub fn as_array(&self) -> Option<&[String]> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }
}

/// Ordered frontmatter — preserves key order for stable serialization.
#[derive(Debug, Clone, PartialEq)]
pub struct Frontmatter {
    pub entries: Vec<(String, FmValue)>,
}

impl Frontmatter {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&FmValue> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn insert(&mut self, key: &str, value: FmValue) {
        if let Some(slot) = self.entries.iter_mut().find(|(k, _)| k == key) {
            slot.1 = value;
        } else {
            self.entries.push((key.to_string(), value));
        }
    }

    pub fn remove(&mut self, key: &str) {
        self.entries.retain(|(k, _)| k != key);
    }
}

impl Default for Frontmatter {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Error)]
pub enum FrontmatterError {
    #[error("missing frontmatter delimiters (---)")]
    MissingDelimiters,
    #[error("frontmatter parse error at line {line}: {message}")]
    Parse { line: usize, message: String },
}

/// Parse a markdown file into (frontmatter, body).
///
/// The file must start with `---\n` and have a closing `---` line.
/// Returns an error if delimiters are missing.
pub fn split_frontmatter(content: &str) -> Result<(String, String), FrontmatterError> {
    let after_open = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
        .ok_or(FrontmatterError::MissingDelimiters)?;

    // Find the closing delimiter line. It must be a line containing
    // exactly "---" (possibly with trailing whitespace).
    let mut close_byte = None;
    let mut search_start = 0;
    for remainder in after_open.split_inclusive('\n') {
        let line = remainder.trim_end_matches('\n').trim_end_matches('\r');
        if line == "---" {
            close_byte = Some(search_start);
            break;
        }
        search_start += remainder.len();
    }

    let close_byte = close_byte.ok_or(FrontmatterError::MissingDelimiters)?;

    let fm_text = &after_open[..close_byte];
    // Strip trailing newline from frontmatter text
    let fm_text = fm_text.strip_suffix('\n').unwrap_or(fm_text);

    // Body is everything after the closing "---\n" line.
    // The canonical format is `---\n\n<body>`, so we strip the
    // leading newline that separates the delimiter from the body.
    let after_close = &after_open[close_byte..];
    let after_close = after_close
        .strip_prefix("---\n")
        .or_else(|| after_close.strip_prefix("---\r\n"))
        .unwrap_or("");
    // Strip the single leading blank line that separates frontmatter
    // from body in the canonical format.
    let body = after_close.strip_prefix('\n').unwrap_or(after_close);

    Ok((fm_text.to_string(), body.to_string()))
}

/// Parse frontmatter text into an ordered Frontmatter struct.
pub fn parse(fm_text: &str) -> Result<Frontmatter, FrontmatterError> {
    let mut fm = Frontmatter::new();

    for (idx, line) in fm_text.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Find the first colon that separates key from value
        let colon_pos = trimmed.find(':').ok_or(FrontmatterError::Parse {
            line: line_no,
            message: "missing ':' separator".to_string(),
        })?;

        let key = trimmed[..colon_pos].trim().to_string();
        let value_str = trimmed[colon_pos + 1..].trim();

        if key.is_empty() {
            return Err(FrontmatterError::Parse {
                line: line_no,
                message: "empty key".to_string(),
            });
        }

        let value = parse_value(value_str)?;
        fm.insert(&key, value);
    }

    Ok(fm)
}

/// Parse a single YAML value string.
fn parse_value(s: &str) -> Result<FmValue, FrontmatterError> {
    // Empty value
    if s.is_empty() || s == "~" || s == "null" {
        return Ok(FmValue::String(String::new()));
    }

    // Inline array: ["a", "b", "c"]
    if s.starts_with('[') && s.ends_with(']') {
        let inner = &s[1..s.len() - 1];
        let items = parse_inline_array(inner);
        return Ok(FmValue::Array(items));
    }

    // Boolean
    if s == "true" {
        return Ok(FmValue::Bool(true));
    }
    if s == "false" {
        return Ok(FmValue::Bool(false));
    }

    // Integer
    if let Ok(i) = s.parse::<i64>() {
        return Ok(FmValue::Int(i));
    }

    // Float
    if let Ok(f) = s.parse::<f64>() {
        return Ok(FmValue::Float(f));
    }

    // Quoted string: "..." or '...'
    if (s.starts_with('"') && s.ends_with('"') && s.len() >= 2)
        || (s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2)
    {
        let inner = &s[1..s.len() - 1];
        // Unescape backslash sequences: \\ → \, \" → "
        // (matching the serializer's escape_string).
        let unescaped = unescape_string(inner);
        return Ok(FmValue::String(unescaped));
    }

    // Unquoted string
    Ok(FmValue::String(s.to_string()))
}

/// Parse inline array contents: `"a", "b", "c"` → vec of strings.
/// Respects quoted commas so `["a,b", "c"]` produces two items, not three.
fn parse_inline_array(inner: &str) -> Vec<String> {
    if inner.trim().is_empty() {
        return Vec::new();
    }

    let mut items = Vec::new();
    let mut current = String::new();
    let mut in_quotes: Option<char> = None;
    let mut chars = inner.chars().peekable();

    while let Some(c) = chars.next() {
        match in_quotes {
            Some(q) => {
                if c == '\\' {
                    if let Some(&next) = chars.peek()
                        && (next == '\\' || next == q)
                    {
                        current.push('\\');
                        current.push(next);
                        chars.next();
                        continue;
                    }
                    current.push(c);
                } else if c == q {
                    // Push the closing quote so the item retains its
                    // outer quotes for the post-processing strip step.
                    current.push(c);
                    in_quotes = None;
                } else {
                    current.push(c);
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    // Push the opening quote so the item retains its
                    // outer quotes for the post-processing strip step.
                    current.push(c);
                    in_quotes = Some(c);
                } else if c == ',' {
                    items.push(current.trim().to_string());
                    current.clear();
                } else {
                    current.push(c);
                }
            }
        }
    }
    if !current.trim().is_empty() || in_quotes.is_some() {
        items.push(current.trim().to_string());
    }

    // Strip outer quotes and unescape each item.
    items
        .iter()
        .map(|item| {
            let trimmed = item.trim();
            if (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
                || (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
            {
                unescape_string(&trimmed[1..trimmed.len() - 1])
            } else {
                trimmed.to_string()
            }
        })
        .collect()
}

/// Serialize a Frontmatter to a YAML string (without delimiters).
pub fn serialize(fm: &Frontmatter) -> String {
    let mut out = String::new();
    for (key, value) in &fm.entries {
        match value {
            FmValue::String(s) => {
                if needs_quotes(s) {
                    let _ = writeln!(out, "{}: \"{}\"", key, escape_string(s));
                } else {
                    let _ = writeln!(out, "{}: {}", key, s);
                }
            }
            FmValue::Float(f) => {
                let _ = writeln!(out, "{}: {}", key, f);
            }
            FmValue::Int(i) => {
                let _ = writeln!(out, "{}: {}", key, i);
            }
            FmValue::Bool(b) => {
                let _ = writeln!(out, "{}: {}", key, b);
            }
            FmValue::Array(arr) => {
                if arr.is_empty() {
                    let _ = writeln!(out, "{}: []", key);
                } else {
                    let items: Vec<String> = arr
                        .iter()
                        .map(|s| format!("\"{}\"", escape_string(s)))
                        .collect();
                    let _ = writeln!(out, "{}: [{}]", key, items.join(", "));
                }
            }
        }
    }
    out
}

/// Check if a string value needs quoting in YAML output.
fn needs_quotes(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    // Quote if it contains special characters, starts with a digit,
    // or could be confused with a YAML keyword
    s.contains(':')
        || s.contains('#')
        || s.contains('{')
        || s.contains('}')
        || s.contains('[')
        || s.contains(']')
        || s.contains(',')
        || s.contains('|')
        || s.contains('>')
        || s.contains('&')
        || s.contains('*')
        || s.contains('!')
        || s.contains('?')
        || s.contains('@')
        || s.contains('`')
        || s.contains('"')
        || s.contains('\'')
        || s.starts_with(' ')
        || s.ends_with(' ')
        || s == "true"
        || s == "false"
        || s == "null"
        || s == "~"
        || s.parse::<f64>().is_ok()
        || s.parse::<i64>().is_ok()
}

/// Escape double quotes and backslashes in a string for YAML output.
fn escape_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Unescape backslash sequences in a parsed quoted string: `\\` → `\`,
/// `\"` → `"`. Reverses `escape_string` so round-tripping is stable.
fn unescape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&next) = chars.peek()
            && (next == '\\' || next == '"')
        {
            chars.next();
            out.push(next);
            continue;
        }
        out.push(c);
    }
    out
}
