//! Extra mailbox passwords so tools can switch account without the admin API.
//!
//! Sources (later wins on key clash):
//! 1. `JMAP_SECRETS_FILE` — mailman4 `secrets.toml` `[passwords]` table
//! 2. `JMAP_ACCOUNTS` — `user@host=password;other@host=password`

use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Load mailbox passwords from env. Empty map if nothing is configured.
pub fn load_from_env() -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();

    if let Ok(path) = std::env::var("JMAP_SECRETS_FILE") {
        let parsed = load_toml_file(Path::new(&path))
            .with_context(|| format!("failed to read JMAP_SECRETS_FILE '{path}'"))?;
        map.extend(parsed);
    }

    if let Ok(raw) = std::env::var("JMAP_ACCOUNTS") {
        map.extend(parse_accounts_env(&raw)?);
    }

    Ok(map)
}

pub fn lookup<'a>(map: &'a HashMap<String, String>, account: &str) -> Option<&'a str> {
    map.get(&normalize(account)).map(String::as_str)
}

fn normalize(account: &str) -> String {
    account.trim().to_ascii_lowercase()
}

/// Parse mailman4-style:
///
/// ```toml
/// [passwords]
/// "hello@codechap.com" = "secret"
/// other@example.com = "also-secret"
/// ```
pub fn parse_passwords_toml(contents: &str) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    let mut in_passwords = false;

    for raw in contents.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            in_passwords = line.eq_ignore_ascii_case("[passwords]");
            continue;
        }
        if !in_passwords {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            bail!("invalid passwords entry: {line}");
        };
        let email = unquote(key.trim());
        let password = unquote(value.trim());
        if email.is_empty() || password.is_empty() {
            bail!("empty email or password in: {line}");
        }
        map.insert(normalize(&email), password);
    }

    Ok(map)
}

fn load_toml_file(path: &Path) -> Result<HashMap<String, String>> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    parse_passwords_toml(&contents)
}

/// `user@host=password;other@host=password`
fn parse_accounts_env(raw: &str) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for part in raw.split([';', '\n']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((email, password)) = part.split_once('=') else {
            bail!("JMAP_ACCOUNTS entry must be email=password, got: {part}");
        };
        let email = email.trim();
        let password = password.trim();
        if email.is_empty() || password.is_empty() {
            bail!("empty email or password in JMAP_ACCOUNTS entry: {part}");
        }
        map.insert(normalize(email), password.to_string());
    }
    Ok(map)
}

fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            '#' if !in_quote => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mailman4_passwords_table() {
        let toml = r#"
# mailman4 secrets
[passwords]
"hello@codechap.com" = "test-secret"
ai@codechap.com = "other"
[ignored]
foo = "bar"
"#;
        let map = parse_passwords_toml(toml).unwrap();
        assert_eq!(lookup(&map, "Hello@codechap.com"), Some("test-secret"));
        assert_eq!(lookup(&map, "ai@codechap.com"), Some("other"));
        assert_eq!(lookup(&map, "missing@x.com"), None);
        assert!(!map.contains_key("foo"));
    }

    #[test]
    fn parses_accounts_env() {
        let map =
            parse_accounts_env("hello@codechap.com=one; ai@codechap.com=two\nthird@x.com=three")
                .unwrap();
        assert_eq!(map.len(), 3);
        assert_eq!(lookup(&map, "hello@codechap.com"), Some("one"));
        assert_eq!(lookup(&map, "third@x.com"), Some("three"));
    }
}
