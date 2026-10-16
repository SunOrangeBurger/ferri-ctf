use url::Url;

use crate::errors::{AppError, AppResult};

/// Store a browser destination, never a URL for the server to fetch or execute.
/// LAN addresses are intentional: the original event runs on college Wi-Fi.
pub fn validate_challenge_url(raw: &str) -> AppResult<Option<String>> {
    let input = raw.trim();
    if input.is_empty() {
        return Ok(None);
    }
    let invalid = || {
        AppError::BadRequest(
            "Challenge website must be a full http:// or https:// URL without credentials or spaces"
                .into(),
        )
    };
    let absolute_http = input.split_once("://").is_some_and(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    });
    if input.len() > 2048
        || !absolute_http
        || raw.chars().any(char::is_control)
        || input.chars().any(char::is_whitespace)
        || input.contains('\\')
    {
        return Err(invalid());
    }
    let parsed = Url::parse(input).map_err(|_| invalid())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.has_host()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(invalid());
    }
    Ok(Some(parsed.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_optional_links_and_lan_event_hosts() {
        assert_eq!(validate_challenge_url("  ").unwrap(), None);
        for (input, expected) in [
            (" https://q01.example.org ", "https://q01.example.org/"),
            ("http://192.168.1.10:9101", "http://192.168.1.10:9101/"),
            (
                "http://host.local:9101/play?view=evidence#start",
                "http://host.local:9101/play?view=evidence#start",
            ),
            ("http://[::1]:9101/", "http://[::1]:9101/"),
        ] {
            assert_eq!(
                validate_challenge_url(input).unwrap().as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn rejects_executable_relative_credentialed_or_malformed_urls() {
        for input in [
            "javascript:alert(1)",
            "data:text/html,test",
            "file:///etc/passwd",
            "//q01.example.org",
            "/q01/",
            "https:example.org",
            "https://",
            "https://operator:password@example.org",
            "https://operator@example.org",
            "https://example.org:99999",
            "https://example.org/a b",
            "https://example.org/\n",
            "https://example.org\\evil",
            "http://[not-an-ip]/",
        ] {
            assert!(validate_challenge_url(input).is_err(), "accepted {input:?}");
        }
        assert!(
            validate_challenge_url(&format!("https://example.org/{}", "a".repeat(2048))).is_err()
        );
    }
}
