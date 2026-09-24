use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::io::Write;
use std::path::{Path, PathBuf};

const UPDATE_PUBLIC_KEY_ENV: &str = "HARPER_UPDATE_PUBLIC_KEY_B64";
const DEFAULT_UPDATE_PUBLIC_KEY_B64: &str = include_str!("update-public-key.b64");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallSource {
    Direct,
    Homebrew,
    Cargo,
    Npm,
    Unknown,
}

impl InstallSource {
    pub fn infer_from_executable(path: &Path) -> Self {
        let path_text = path.to_string_lossy().to_lowercase();
        if path_text.contains("cellar") || path_text.contains("homebrew") {
            return Self::Homebrew;
        }
        if path_text.contains(".cargo/bin") {
            return Self::Cargo;
        }
        if path_text.contains("node_modules") || path_text.contains("npm") {
            return Self::Npm;
        }
        if path.is_absolute() {
            return Self::Direct;
        }
        Self::Unknown
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Homebrew => "homebrew",
            Self::Cargo => "cargo",
            Self::Npm => "npm",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct InstallMetadata {
    install_source: InstallSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseArtifact {
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub version: String,
    #[serde(default)]
    pub published_at: Option<String>,
    pub artifacts: BTreeMap<String, ReleaseArtifact>,
}

impl ReleaseManifest {
    pub fn from_json(text: &str) -> crate::HarperResult<Self> {
        serde_json::from_str(text)
            .map_err(|e| crate::HarperError::Validation(format!("Invalid release manifest: {}", e)))
    }

    pub fn artifact_for_target<'a>(&'a self, target: &str) -> Option<&'a ReleaseArtifact> {
        self.artifacts.get(target)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: String,
    pub target: String,
    pub artifact_available: bool,
    pub update_available: bool,
}

pub fn load_persisted_install_source() -> crate::HarperResult<Option<InstallSource>> {
    load_persisted_install_source_from_home(home_dir()?)
}

pub fn save_persisted_install_source(source: InstallSource) -> crate::HarperResult<()> {
    save_persisted_install_source_from_home(home_dir()?, source)
}

pub fn resolve_install_source(executable: &Path) -> crate::HarperResult<InstallSource> {
    let inferred = InstallSource::infer_from_executable(executable);
    if matches!(
        inferred,
        InstallSource::Homebrew | InstallSource::Cargo | InstallSource::Npm
    ) {
        return Ok(inferred);
    }

    if let Some(saved) = load_persisted_install_source()? {
        if saved != InstallSource::Unknown {
            return Ok(saved);
        }
    }

    Ok(inferred)
}

pub async fn fetch_release_manifest(url: &str) -> crate::HarperResult<ReleaseManifest> {
    let response = reqwest::get(url)
        .await
        .map_err(|e| crate::HarperError::Api(format!("Failed to fetch release manifest: {}", e)))?;

    let status = response.status();
    let body = response.text().await.map_err(|e| {
        crate::HarperError::Api(format!("Failed to read release manifest response: {}", e))
    })?;

    if !status.is_success() {
        return Err(crate::HarperError::Api(format!(
            "Release manifest request failed with status {}",
            status
        )));
    }

    ReleaseManifest::from_json(&body)
}

pub async fn download_release_artifact(url: &str) -> crate::HarperResult<Vec<u8>> {
    let response = reqwest::get(url).await.map_err(|e| {
        crate::HarperError::Api(format!("Failed to download release artifact: {}", e))
    })?;

    let status = response.status();
    let bytes = response.bytes().await.map_err(|e| {
        crate::HarperError::Api(format!("Failed to read release artifact response: {}", e))
    })?;

    if !status.is_success() {
        return Err(crate::HarperError::Api(format!(
            "Release artifact request failed with status {}",
            status
        )));
    }

    Ok(bytes.to_vec())
}

pub fn verify_artifact_checksum(bytes: &[u8], expected_sha256: &str) -> crate::HarperResult<()> {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    let actual = encode_hex_lower(digest.as_ref());
    let expected = expected_sha256.trim().to_lowercase();

    if actual != expected {
        return Err(crate::HarperError::Validation(format!(
            "Release artifact checksum mismatch: expected {}, got {}",
            expected, actual
        )));
    }

    Ok(())
}

pub fn resolve_update_public_key() -> crate::HarperResult<Vec<u8>> {
    let encoded = std::env::var(UPDATE_PUBLIC_KEY_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_UPDATE_PUBLIC_KEY_B64.trim().to_string());
    decode_update_public_key(&encoded)
}

pub fn verify_artifact_signature(
    bytes: &[u8],
    signature_b64: &str,
    public_key: &[u8],
) -> crate::HarperResult<()> {
    use base64::Engine;

    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature_b64.trim())
        .map_err(|e| {
            crate::HarperError::Validation(format!("Invalid release artifact signature: {}", e))
        })?;

    let verifier = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key);
    verifier.verify(bytes, &signature).map_err(|_| {
        crate::HarperError::Validation("Release artifact signature verification failed".to_string())
    })
}

pub fn extract_release_executable(
    artifact_name: &str,
    bytes: &[u8],
    executable_name: &str,
) -> crate::HarperResult<Vec<u8>> {
    if artifact_name.ends_with(".tar.gz") || artifact_name.ends_with(".tgz") {
        return extract_from_tar_gz(bytes, executable_name);
    }
    if artifact_name.ends_with(".zip") {
        return extract_from_zip(bytes, executable_name);
    }
    Ok(bytes.to_vec())
}

pub fn install_downloaded_executable(executable: &Path, bytes: &[u8]) -> crate::HarperResult<()> {
    let parent = executable.parent().ok_or_else(|| {
        crate::HarperError::Io("Executable path has no parent directory".to_string())
    })?;

    let mut staged = tempfile::Builder::new()
        .prefix(".harper-update-")
        .tempfile_in(parent)
        .map_err(|e| {
            crate::HarperError::Io(format!("Failed to create staged update file: {}", e))
        })?;

    staged.write_all(bytes).map_err(|e| {
        crate::HarperError::Io(format!("Failed to write staged update artifact: {}", e))
    })?;
    staged.flush().map_err(|e| {
        crate::HarperError::Io(format!("Failed to flush staged update artifact: {}", e))
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staged
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))
            .map_err(|e| {
                crate::HarperError::Io(format!(
                    "Failed to set executable permissions on staged artifact: {}",
                    e
                ))
            })?;
    }

    let backup = executable.with_extension("old");
    if backup.exists() {
        fs::remove_file(&backup).map_err(|e| {
            crate::HarperError::Io(format!("Failed to remove stale backup binary: {}", e))
        })?;
    }

    fs::rename(executable, &backup).map_err(|e| {
        crate::HarperError::Io(format!(
            "Failed to create binary backup before update: {}",
            e
        ))
    })?;

    let staged_path = staged.into_temp_path();
    match staged_path.persist(executable) {
        Ok(_) => {
            fs::remove_file(&backup).map_err(|e| {
                crate::HarperError::Io(format!(
                    "Updated binary installed, but backup cleanup failed: {}",
                    e
                ))
            })?;
            Ok(())
        }
        Err(err) => {
            let _ = fs::rename(&backup, executable);
            Err(crate::HarperError::Io(format!(
                "Failed to replace executable with updated binary: {}",
                err.error
            )))
        }
    }
}

pub fn evaluate_update(
    current_version: &str,
    manifest: &ReleaseManifest,
    target: &str,
) -> UpdateCheckResult {
    let artifact_available = manifest.artifact_for_target(target).is_some();
    let update_available = compare_versions(current_version, &manifest.version)
        .is_some_and(|ordering| ordering == Ordering::Less);

    UpdateCheckResult {
        current_version: current_version.to_string(),
        latest_version: manifest.version.clone(),
        target: target.to_string(),
        artifact_available,
        update_available,
    }
}

pub fn compare_versions(left: &str, right: &str) -> Option<Ordering> {
    let left = parse_version_triplet(left)?;
    let right = parse_version_triplet(right)?;
    Some(left.cmp(&right))
}

pub fn current_target_key() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    format!("{}-{}", os, arch)
}

fn home_dir() -> crate::HarperResult<PathBuf> {
    dirs::home_dir().ok_or_else(|| {
        crate::HarperError::Config("Home directory not found for update metadata".to_string())
    })
}

fn metadata_path_from_home(home: &Path) -> PathBuf {
    home.join(".harper")
        .join("update")
        .join("install-source.json")
}

fn load_persisted_install_source_from_home(
    home: PathBuf,
) -> crate::HarperResult<Option<InstallSource>> {
    let path = metadata_path_from_home(&home);
    if !path.exists() {
        return Ok(None);
    }

    let text = fs::read_to_string(&path).map_err(|e| {
        crate::HarperError::Io(format!(
            "Failed to read install-source metadata {}: {}",
            path.display(),
            e
        ))
    })?;
    let metadata: InstallMetadata = serde_json::from_str(&text).map_err(|e| {
        crate::HarperError::Validation(format!(
            "Invalid install-source metadata {}: {}",
            path.display(),
            e
        ))
    })?;
    Ok(Some(metadata.install_source))
}

fn save_persisted_install_source_from_home(
    home: PathBuf,
    source: InstallSource,
) -> crate::HarperResult<()> {
    let path = metadata_path_from_home(&home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            crate::HarperError::Io(format!(
                "Failed to create update metadata directory {}: {}",
                parent.display(),
                e
            ))
        })?;
    }

    let body = serde_json::to_string_pretty(&InstallMetadata {
        install_source: source,
    })
    .map_err(|e| {
        crate::HarperError::Api(format!(
            "Failed to serialize install-source metadata: {}",
            e
        ))
    })?;
    fs::write(&path, format!("{}\n", body)).map_err(|e| {
        crate::HarperError::Io(format!(
            "Failed to write install-source metadata {}: {}",
            path.display(),
            e
        ))
    })
}

fn parse_version_triplet(version: &str) -> Option<[u64; 3]> {
    let trimmed = version.trim();
    let trimmed = trimmed
        .find(|ch: char| ch.is_ascii_digit() || ch == 'v')
        .and_then(|index| trimmed.get(index..))
        .unwrap_or(trimmed)
        .trim_start_matches('v');
    let core = trimmed.split(['-', '+']).next()?;
    let mut parts = core.split('.');

    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;

    Some([major, minor, patch])
}

fn extract_from_tar_gz(bytes: &[u8], executable_name: &str) -> crate::HarperResult<Vec<u8>> {
    let cursor = Cursor::new(bytes);
    let decoder = flate2::read::GzDecoder::new(cursor);
    let mut archive = tar::Archive::new(decoder);

    for entry in archive.entries().map_err(|e| {
        crate::HarperError::Validation(format!("Failed to read tar.gz release artifact: {}", e))
    })? {
        let mut entry = entry.map_err(|e| {
            crate::HarperError::Validation(format!("Failed to read tar.gz entry: {}", e))
        })?;
        let path = entry.path().map_err(|e| {
            crate::HarperError::Validation(format!("Failed to inspect tar.gz entry path: {}", e))
        })?;

        if matches_executable_name(&path, executable_name) {
            let mut out = Vec::new();
            std::io::copy(&mut entry, &mut out).map_err(|e| {
                crate::HarperError::Validation(format!(
                    "Failed to extract executable from tar.gz artifact: {}",
                    e
                ))
            })?;
            return Ok(out);
        }
    }

    Err(crate::HarperError::Validation(format!(
        "Executable {} not found in tar.gz release artifact",
        executable_name
    )))
}

fn extract_from_zip(bytes: &[u8], executable_name: &str) -> crate::HarperResult<Vec<u8>> {
    let cursor = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| {
        crate::HarperError::Validation(format!("Failed to read zip release artifact: {}", e))
    })?;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|e| {
            crate::HarperError::Validation(format!("Failed to read zip entry: {}", e))
        })?;
        let path = PathBuf::from(file.name());
        if matches_executable_name(&path, executable_name) {
            let mut out = Vec::new();
            std::io::copy(&mut file, &mut out).map_err(|e| {
                crate::HarperError::Validation(format!(
                    "Failed to extract executable from zip artifact: {}",
                    e
                ))
            })?;
            return Ok(out);
        }
    }

    Err(crate::HarperError::Validation(format!(
        "Executable {} not found in zip release artifact",
        executable_name
    )))
}

fn matches_executable_name(path: &Path, executable_name: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == executable_name)
}

fn encode_hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn decode_update_public_key(encoded: &str) -> crate::HarperResult<Vec<u8>> {
    use base64::Engine;

    base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|e| crate::HarperError::Validation(format!("Invalid update public key: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::{
        InstallSource, ReleaseManifest, compare_versions, current_target_key, evaluate_update,
        extract_release_executable, install_downloaded_executable,
        load_persisted_install_source_from_home, metadata_path_from_home,
        save_persisted_install_source_from_home, verify_artifact_checksum,
        verify_artifact_signature,
    };
    use base64::Engine;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use std::cmp::Ordering;
    use std::fs;
    use std::io::{Cursor, Write};
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    #[test]
    fn parses_release_manifest() {
        let manifest = ReleaseManifest::from_json(
            r#"{
                "version": "0.16.0",
                "published_at": "2026-05-01T00:00:00Z",
                "artifacts": {
                    "macos-aarch64": {
                        "url": "https://example.invalid/harper-macos-aarch64.tar.gz",
                        "sha256": "abc123",
                        "signature": "sig123"
                    }
                }
            }"#,
        )
        .expect("manifest should parse");

        assert_eq!(manifest.version, "0.16.0");
        assert_eq!(
            manifest
                .artifact_for_target("macos-aarch64")
                .expect("artifact")
                .sha256,
            "abc123"
        );
        assert_eq!(
            manifest
                .artifact_for_target("macos-aarch64")
                .expect("artifact")
                .signature
                .as_deref(),
            Some("sig123")
        );
    }

    #[test]
    fn infers_install_source_from_common_paths() {
        assert_eq!(
            InstallSource::infer_from_executable(Path::new(
                "/opt/homebrew/Cellar/harper/0.1.0/bin/harper"
            )),
            InstallSource::Homebrew
        );
        assert_eq!(
            InstallSource::infer_from_executable(Path::new("/Users/test/.cargo/bin/harper")),
            InstallSource::Cargo
        );
        assert_eq!(
            InstallSource::infer_from_executable(Path::new(
                "/usr/local/lib/node_modules/harper/bin/harper"
            )),
            InstallSource::Npm
        );

        #[cfg(windows)]
        let direct_path = Path::new(r"C:\Program Files\Harper\harper.exe");
        #[cfg(not(windows))]
        let direct_path = Path::new("/usr/local/bin/harper");

        assert_eq!(
            InstallSource::infer_from_executable(direct_path),
            InstallSource::Direct
        );
    }

    #[test]
    fn compares_versions_semantically() {
        assert_eq!(compare_versions("0.16.0", "0.16.1"), Some(Ordering::Less));
        assert_eq!(compare_versions("v0.16.1", "0.16.1"), Some(Ordering::Equal));
        assert_eq!(
            compare_versions("0.17.1", "harper-0.17.2"),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare_versions("harper-0.17.2", "0.17.2"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_versions("0.17.0", "0.16.9"),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn evaluates_update_availability_and_artifact_presence() {
        let manifest = ReleaseManifest::from_json(
            r#"{
                "version": "0.16.1",
                "artifacts": {
                    "macos-aarch64": {
                        "url": "https://example.invalid/harper-macos-aarch64.tar.gz",
                        "sha256": "abc123",
                        "signature": "sig123"
                    }
                }
            }"#,
        )
        .expect("manifest should parse");

        let result = evaluate_update("0.16.0", &manifest, "macos-aarch64");
        assert!(result.update_available);
        assert!(result.artifact_available);
    }

    #[test]
    fn current_target_key_is_non_empty() {
        assert!(!current_target_key().is_empty());
    }

    #[test]
    fn verifies_release_artifact_checksum() {
        let bytes = b"harper";
        let expected = "eaef53d9b579688e06f0ffda25b907cf7a2a08dd98d34debf9ad3d1a9e2514ea";
        verify_artifact_checksum(bytes, expected).expect("checksum should match");
    }

    #[test]
    fn verifies_release_artifact_signature() {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).expect("generate keypair");
        let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("parse keypair");
        let bytes = b"harper";
        let signature = key_pair.sign(bytes);
        let encoded_signature =
            base64::engine::general_purpose::STANDARD.encode(signature.as_ref());

        verify_artifact_signature(bytes, &encoded_signature, key_pair.public_key().as_ref())
            .expect("signature should verify");
    }

    #[test]
    fn installs_downloaded_executable_by_replacing_existing_binary() {
        let tempdir = tempdir().expect("tempdir");
        let executable = tempdir.path().join("harper");
        fs::write(&executable, b"old-binary").expect("write existing binary");

        install_downloaded_executable(&executable, b"new-binary")
            .expect("binary replacement should succeed");

        assert_eq!(
            fs::read(&executable).expect("read updated binary"),
            b"new-binary"
        );
        assert!(!executable.with_extension("old").exists());
    }

    #[test]
    fn extracts_executable_from_tar_gz_artifact() {
        let mut archive_bytes = Vec::new();
        {
            let encoder =
                flate2::write::GzEncoder::new(&mut archive_bytes, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            let payload = b"binary-data";
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "harper", &payload[..])
                .expect("append tar entry");
            builder.finish().expect("finish tar builder");
        }

        let extracted =
            extract_release_executable("harper-macos-aarch64.tar.gz", &archive_bytes, "harper")
                .expect("extract executable");
        assert_eq!(extracted, b"binary-data");
    }

    #[test]
    fn extracts_executable_from_zip_artifact() {
        let mut zip_bytes = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut zip_bytes);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o755);
            writer
                .start_file("harper", options)
                .expect("start zip file");
            writer.write_all(b"binary-data").expect("write zip payload");
            writer.finish().expect("finish zip");
        }

        let extracted =
            extract_release_executable("harper-macos-aarch64.zip", zip_bytes.get_ref(), "harper")
                .expect("extract executable");
        assert_eq!(extracted, b"binary-data");
    }

    #[test]
    fn persists_and_loads_install_source_metadata() {
        let tempdir = tempdir().expect("tempdir");
        save_persisted_install_source_from_home(
            tempdir.path().to_path_buf(),
            InstallSource::Direct,
        )
        .expect("save metadata");

        let loaded = load_persisted_install_source_from_home(tempdir.path().to_path_buf())
            .expect("load metadata");
        assert_eq!(loaded, Some(InstallSource::Direct));
        assert!(metadata_path_from_home(tempdir.path()).exists());
    }

    #[test]
    fn persisted_install_source_can_cover_ambiguous_path_cases() {
        let tempdir = tempdir().expect("tempdir");
        save_persisted_install_source_from_home(
            tempdir.path().to_path_buf(),
            InstallSource::Direct,
        )
        .expect("save metadata");

        let executable = PathBuf::from("harper");
        assert_eq!(
            InstallSource::infer_from_executable(&executable),
            InstallSource::Unknown
        );
        assert_eq!(
            load_persisted_install_source_from_home(tempdir.path().to_path_buf())
                .expect("load metadata"),
            Some(InstallSource::Direct)
        );
    }
}
