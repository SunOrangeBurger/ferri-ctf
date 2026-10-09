use std::path::{Path, PathBuf};
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};

const ZIP_MAGIC: [u8; 4] = [0x50, 0x4B, 0x03, 0x04]; // PK\x03\x04

/// Sanitize a filename for use in HTTP Content-Disposition header.
/// Strips quotes, CR, LF, semicolons, and path separators.
pub fn sanitize_header_filename(raw: &str) -> String {
    let base = Path::new(raw)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(raw);

    let sanitized: String = base
        .chars()
        .filter(|&c| c != '"' && c != '\r' && c != '\n' && c != ';' && c != '/' && c != '\\')
        .collect();

    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "challenge.zip".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Ensure uploads directory exists and is canonicalized.
pub async fn ensure_uploads_dir() -> AppResult<PathBuf> {
    let uploads = PathBuf::from("./uploads");
    if !uploads.exists() {
        fs::create_dir_all(&uploads).await?;
    }
    let canonical = fs::canonicalize(&uploads).await?;
    Ok(canonical)
}

/// Validate and store an uploaded zip file stream for a challenge.
/// Verifies size <= max_bytes and PK magic bytes, then atomically renames to `./uploads/{uuid}.zip`.
/// Returns (stored_path_string, sanitized_filename).
pub async fn validate_and_store_zip_bytes(
    raw_filename: Option<&str>,
    data: &[u8],
    max_bytes: usize,
) -> AppResult<(String, String)> {
    if data.len() > max_bytes {
        return Err(AppError::PayloadTooLarge);
    }

    if data.len() < 4 || data[0..4] != ZIP_MAGIC {
        return Err(AppError::BadRequest(
            "Uploaded file must be a valid ZIP archive (magic bytes mismatch)".into(),
        ));
    }

    let uploads_dir = ensure_uploads_dir().await?;
    let challenge_uuid = Uuid::new_v4().to_string();
    let temp_filename = format!("{challenge_uuid}.tmp");
    let dest_filename = format!("{challenge_uuid}.zip");

    let temp_path = uploads_dir.join(&temp_filename);
    let dest_path = uploads_dir.join(&dest_filename);

    // Defense in depth: path traversal verification
    if !temp_path.starts_with(&uploads_dir) || !dest_path.starts_with(&uploads_dir) {
        return Err(AppError::internal("Upload path traversal detected"));
    }

    let mut file = File::create(&temp_path).await?;
    file.write_all(data).await?;
    file.flush().await?;
    drop(file);

    fs::rename(&temp_path, &dest_path).await?;

    let safe_name = sanitize_header_filename(raw_filename.unwrap_or("challenge.zip"));
    let stored_path_str = dest_path.to_string_lossy().to_string();

    Ok((stored_path_str, safe_name))
}

/// Delete an uploaded challenge file from disk if it exists.
pub async fn delete_challenge_file(path_str: &str) -> AppResult<()> {
    let path = Path::new(path_str);
    if path.exists() {
        fs::remove_file(path).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_header_filename() {
        assert_eq!(sanitize_header_filename("test.zip"), "test.zip");
        assert_eq!(sanitize_header_filename("../../../etc/passwd.zip"), "passwd.zip");
        assert_eq!(sanitize_header_filename("foo\"bar;baz\r\n.zip"), "foobarbaz.zip");
        assert_eq!(sanitize_header_filename("   "), "challenge.zip");
    }

    #[tokio::test]
    async fn test_validate_and_store_zip() {
        // Valid ZIP (PK\x03\x04 followed by dummy data)
        let mut valid_zip = vec![0x50, 0x4B, 0x03, 0x04];
        valid_zip.extend_from_slice(b"some zip contents");

        let (path, name) = validate_and_store_zip_bytes(Some("chal.zip"), &valid_zip, 1024 * 1024)
            .await
            .unwrap();

        assert_eq!(name, "chal.zip");
        assert!(Path::new(&path).exists());

        // Cleanup
        delete_challenge_file(&path).await.unwrap();
        assert!(!Path::new(&path).exists());

        // Invalid magic bytes
        let invalid = b"NOT A ZIP";
        let err = validate_and_store_zip_bytes(Some("bad.zip"), invalid, 1024 * 1024)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        // Too large
        let err = validate_and_store_zip_bytes(Some("big.zip"), &valid_zip, 5)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::PayloadTooLarge));
    }
}
