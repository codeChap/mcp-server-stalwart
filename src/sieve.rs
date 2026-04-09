use anyhow::{Result, bail};
use regex::Regex;

/// Parse a DSN notify Sieve script, extracting the list of email addresses.
///
/// Returns `Err` if the script doesn't match the expected template structure,
/// which indicates manual customisation that would be lost on overwrite.
pub fn parse_dsn_script(script: &str) -> Result<Vec<String>> {
    // Structural checks — make sure this looks like our generated template
    let trimmed = script.trim();
    if !trimmed.contains("require") || !trimmed.contains("envelope.notify") {
        bail!(
            "DSN script has unexpected structure (manual customisation?).\n\
             Raw script:\n{script}"
        );
    }

    let re = Regex::new(r#"envelope\s+:is\s+"from"\s+"([^"]+)""#)?;
    let accounts: Vec<String> = re
        .captures_iter(script)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect();

    if accounts.is_empty() {
        bail!(
            "No email addresses found in DSN script.\nRaw script:\n{script}"
        );
    }

    Ok(accounts)
}

/// Generate a DSN notify Sieve script from a list of email addresses.
///
/// Validates each address to prevent Sieve injection.
/// Returns `Err` if the list is empty or any address is invalid.
pub fn generate_dsn_script(accounts: &[String]) -> Result<String> {
    if accounts.is_empty() {
        bail!("At least one email address is required");
    }

    for addr in accounts {
        validate_email(addr)?;
    }

    let conditions: Vec<String> = accounts
        .iter()
        .map(|addr| format!("    envelope :is \"from\" \"{addr}\""))
        .collect();

    let joined = conditions.join(",\n");

    Ok(format!(
        "require [\"variables\", \"envelope\"];\n\
         \n\
         if anyof(\n\
         {joined}\n\
         ) {{\n    \
             set \"envelope.notify\" \"SUCCESS,FAILURE\";\n\
         }}\n"
    ))
}

/// Validate an email address for safe embedding in a Sieve script.
///
/// This is a safety check to prevent injection, not full RFC 5322 validation.
fn validate_email(addr: &str) -> Result<()> {
    if addr.is_empty() {
        bail!("Email address cannot be empty");
    }
    if addr.contains('"') || addr.contains('\\') || addr.contains('\n') || addr.contains('\r') {
        bail!("Email address contains unsafe characters: {addr}");
    }
    if addr.contains(char::is_whitespace) {
        bail!("Email address contains whitespace: {addr}");
    }
    let parts: Vec<&str> = addr.split('@').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        bail!("Invalid email format: {addr}");
    }
    if !parts[1].contains('.') {
        bail!("Email domain must contain a dot: {addr}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let accounts = vec![
            "rose@thegrapevine.co.za".to_string(),
            "info@thegrapevine.co.za".to_string(),
        ];
        let script = generate_dsn_script(&accounts).unwrap();
        let parsed = parse_dsn_script(&script).unwrap();
        assert_eq!(parsed, accounts);
    }

    #[test]
    fn parse_live_script() {
        let script = r#"require ["variables", "envelope"];

if anyof(
    envelope :is "from" "rose@thegrapevine.co.za",
    envelope :is "from" "salesrose@thegrapevine.co.za",
    envelope :is "from" "info@thegrapevine.co.za"
) {
    set "envelope.notify" "SUCCESS,FAILURE";
}
"#;
        let accounts = parse_dsn_script(script).unwrap();
        assert_eq!(accounts, vec![
            "rose@thegrapevine.co.za",
            "salesrose@thegrapevine.co.za",
            "info@thegrapevine.co.za",
        ]);
    }

    #[test]
    fn reject_empty_list() {
        assert!(generate_dsn_script(&[]).is_err());
    }

    #[test]
    fn reject_injection() {
        let bad = vec!["test\"inject".to_string()];
        assert!(generate_dsn_script(&bad).is_err());
    }

    #[test]
    fn reject_bad_email() {
        let bad = vec!["notanemail".to_string()];
        assert!(generate_dsn_script(&bad).is_err());
    }

    #[test]
    fn reject_whitespace_injection() {
        let bad = vec!["test @example.com".to_string()];
        assert!(generate_dsn_script(&bad).is_err());
    }

    #[test]
    fn detect_custom_script() {
        let custom = "# my custom sieve rules\nredirect \"admin@example.com\";";
        assert!(parse_dsn_script(custom).is_err());
    }
}
