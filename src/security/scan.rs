//! Secret pattern scanner.
//!
//! Detects high-confidence secret patterns in entity content before
//! it's written to disk. Returns the matched pattern type so the
//! caller can report what was found.
//!
//! Design choices:
//! - Reject, don't redact. Redaction can mangle content in subtle
//!   ways and the agent should rewrite without the secret.
//! - High-confidence patterns only. False positives are disruptive
//!   (the write is rejected), so each pattern must be specific enough
//!   that it rarely matches non-secret text.
//! - No entropy-based detection. High-entropy string detection has
//!   too many false positives in code-aware knowledge bases that
//!   legitimately discuss hashes, UUIDs, and encoded data.

use std::sync::LazyLock;

/// A detected secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    /// What kind of secret was detected.
    pub kind: SecretKind,
    /// The first few characters of the match (for error reporting,
    /// never the full secret).
    pub preview: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    PrivateKey,
    GithubToken,
    AwsAccessKey,
    SlackToken,
    OpenAiKey,
    AnthropicKey,
    GenericCredential,
}

impl std::fmt::Display for SecretKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrivateKey => write!(f, "private key"),
            Self::GithubToken => write!(f, "GitHub token"),
            Self::AwsAccessKey => write!(f, "AWS access key"),
            Self::SlackToken => write!(f, "Slack token"),
            Self::OpenAiKey => write!(f, "OpenAI API key"),
            Self::AnthropicKey => write!(f, "Anthropic API key"),
            Self::GenericCredential => write!(f, "credential assignment"),
        }
    }
}

struct Pattern {
    kind: SecretKind,
    regex: regex::Regex,
}

static PATTERNS: LazyLock<Vec<Pattern>> = LazyLock::new(|| {
    vec![
        // PEM private keys (RSA, EC, OpenSSH, PGP, etc.)
        Pattern {
            kind: SecretKind::PrivateKey,
            regex: regex::Regex::new(
                r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY-----",
            )
            .expect("valid regex"),
        },
        // GitHub tokens: ghp_, gho_, ghu_, ghs_, ghr_ + 36+ chars
        Pattern {
            kind: SecretKind::GithubToken,
            regex: regex::Regex::new(r"gh[pousr]_[A-Za-z0-9]{36,}").expect("valid regex"),
        },
        // AWS access key IDs: AKIA + 16 chars
        Pattern {
            kind: SecretKind::AwsAccessKey,
            regex: regex::Regex::new(r"AKIA[0-9A-Z]{16}").expect("valid regex"),
        },
        // Slack tokens: xox[baprs]- + 10+ chars
        Pattern {
            kind: SecretKind::SlackToken,
            regex: regex::Regex::new(r"xox[baprs]-[A-Za-z0-9-]{10,}").expect("valid regex"),
        },
        // OpenAI API keys: sk- + 20+ chars (project keys sk-proj-, classic sk-)
        Pattern {
            kind: SecretKind::OpenAiKey,
            regex: regex::Regex::new(r"sk-(?:proj-)?[A-Za-z0-9]{20,}").expect("valid regex"),
        },
        // Anthropic API keys: sk-ant- + 30+ chars
        Pattern {
            kind: SecretKind::AnthropicKey,
            regex: regex::Regex::new(r"sk-ant-[A-Za-z0-9_-]{30,}").expect("valid regex"),
        },
        // Generic credential assignment:
        // (password|passwd|pwd|secret|api_key|apikey|api-key|token|access_key)
        // optionally quoted, followed by = or : then a quoted string of 8+ chars.
        // Handles JSON ("password": "x"), YAML (password: "x"), and env (api_key="x").
        Pattern {
            kind: SecretKind::GenericCredential,
            regex: regex::Regex::new(
                r#"(?i)(?:password|passwd|pwd|secret|api[_-]?key|token|access[_-]?key)["']?\s*[:=]\s*["'][^"']{8,}["']"#,
            )
            .expect("valid regex"),
        },
    ]
});

/// Scan content for secret patterns. Returns the first match, if any.
///
/// Scans both the title and body. The preview in the result is
/// truncated to 4 characters — just the identifying prefix, not
/// enough to expose the secret body. The `kind` field already
/// identifies the pattern type.
pub fn scan_content(title: &str, body: &str) -> Option<ScanResult> {
    let combined = format!("{}\n{}", title, body);
    for pattern in PATTERNS.iter() {
        if let Some(m) = pattern.regex.find(&combined) {
            let preview = m.as_str().chars().take(4).collect();
            return Some(ScanResult {
                kind: pattern.kind,
                preview,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_private_key() {
        let body = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...";
        assert_eq!(scan_content("", body).unwrap().kind, SecretKind::PrivateKey);
    }

    #[test]
    fn detects_github_token() {
        let body = "token: ghp_1234567890abcdefghijklmnopqrstuvwxyz";
        assert_eq!(
            scan_content("", body).unwrap().kind,
            SecretKind::GithubToken
        );
    }

    #[test]
    fn detects_aws_access_key() {
        let body = "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE";
        assert_eq!(
            scan_content("", body).unwrap().kind,
            SecretKind::AwsAccessKey
        );
    }

    #[test]
    fn detects_openai_key() {
        let body = "sk-proj-abcdefghijklmnopqrstuvwx";
        assert_eq!(scan_content("", body).unwrap().kind, SecretKind::OpenAiKey);
    }

    #[test]
    fn detects_anthropic_key() {
        let body = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
        assert_eq!(
            scan_content("", body).unwrap().kind,
            SecretKind::AnthropicKey
        );
    }

    #[test]
    fn detects_generic_credential_in_quotes() {
        let body = r#""password": "hunter2abc""#;
        assert_eq!(
            scan_content("", body).unwrap().kind,
            SecretKind::GenericCredential
        );
    }

    #[test]
    fn detects_generic_credential_with_equals() {
        let body = r#"api_key = "mysecret123abc""#;
        assert_eq!(
            scan_content("", body).unwrap().kind,
            SecretKind::GenericCredential
        );
    }

    #[test]
    fn detects_in_title() {
        let title = "Key: ghp_1234567890abcdefghijklmnopqrstuvwxyz";
        assert_eq!(
            scan_content(title, "").unwrap().kind,
            SecretKind::GithubToken
        );
    }

    #[test]
    fn no_false_positive_on_short_values() {
        // "password: \"abc\"" is only 3 chars — below the 8-char minimum.
        let body = r#"password = "abc""#;
        assert!(scan_content("", body).is_none());
    }

    #[test]
    fn no_false_positive_on_discussion_text() {
        let body = "When using API keys, store them in environment variables, not in code.";
        assert!(scan_content("", body).is_none());
    }

    #[test]
    fn no_false_positive_on_uuids() {
        let body = "Entity ID: 550e8400-e29b-41d4-a716-446655440000";
        assert!(scan_content("", body).is_none());
    }

    #[test]
    fn no_false_positive_on_hashes() {
        let body = "SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(scan_content("", body).is_none());
    }

    #[test]
    fn no_false_positive_on_base64_discussion() {
        let body = "Encode the payload as base64 before sending it to the API endpoint.";
        assert!(scan_content("", body).is_none());
    }

    #[test]
    fn preview_is_truncated() {
        let body = "ghp_1234567890abcdefghijklmnopqrstuvwxyz";
        let result = scan_content("", body).unwrap();
        assert!(result.preview.chars().count() <= 4);
        assert_eq!(result.preview, "ghp_");
    }
}
