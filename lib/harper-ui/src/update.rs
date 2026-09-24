use harper_core::{
    InstallSource, compare_versions, current_target_key, download_release_artifact,
    evaluate_update, extract_release_executable, fetch_release_manifest,
    install_downloaded_executable, resolve_install_source, resolve_update_public_key,
    save_persisted_install_source, verify_artifact_checksum, verify_artifact_signature,
};
use serde::Deserialize;
#[cfg(unix)]
use std::fs;
use std::path::PathBuf;
use std::{env, path::Path};

const UPDATE_MANIFEST_ENV: &str = "HARPER_UPDATE_MANIFEST_URL";
const RELEASES_API_URL: &str = "https://api.github.com/repos/coccinella-labs/harper/releases";
const MANIFEST_ASSET_NAME: &str = "release-manifest.json";

#[derive(Debug, Deserialize)]
struct GitHubReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    draft: bool,
    prerelease: bool,
    assets: Vec<GitHubReleaseAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomebrewPathFix {
    pub shadow_path: PathBuf,
    pub homebrew_path: PathBuf,
}

pub async fn handle_update_command(args: &[String]) -> Option<i32> {
    if args.len() < 2 {
        return None;
    }

    match args[1].as_str() {
        "version" => {
            print_version();
            Some(0)
        }
        "self-update" => Some(handle_self_update(args).await),
        _ => None,
    }
}

pub async fn fetch_update_status() -> Result<Option<String>, String> {
    let manifest_url = resolve_manifest_url().await?;
    let executable = env::current_exe()
        .map_err(|err| format!("failed to resolve current executable: {}", err))?;
    let target = current_target_key();
    let _install_source = resolve_install_source(&executable)
        .map_err(|err| format!("failed to resolve install source: {}", err))?;
    let manifest = fetch_release_manifest(&manifest_url)
        .await
        .map_err(|err| err.to_string())?;
    let result = evaluate_update(crate::CLI_VERSION, &manifest, &target);

    if !result.artifact_available {
        return Ok(Some(format!("update: no {}", result.target)));
    }

    match compare_versions(&result.current_version, &result.latest_version) {
        Some(std::cmp::Ordering::Less) => Ok(Some(format!("update: {}", result.latest_version))),
        Some(std::cmp::Ordering::Equal) => Ok(Some("update: latest".to_string())),
        Some(std::cmp::Ordering::Greater) => Ok(Some("update: local newer".to_string())),
        None => Ok(Some("update: unknown".to_string())),
    }
}

async fn handle_self_update(args: &[String]) -> i32 {
    let check_only = args.iter().skip(2).any(|arg| arg == "--check");
    if args
        .iter()
        .skip(2)
        .any(|arg| arg == "--help" || arg == "-h")
    {
        print_self_update_usage();
        return 0;
    }

    let executable = match env::current_exe() {
        Ok(path) => path,
        Err(err) => {
            eprintln!("Failed to resolve current executable: {}", err);
            return 1;
        }
    };

    let install_source = match resolve_install_source(&executable) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("Failed to resolve install source: {}", err);
            return 1;
        }
    };
    let target = current_target_key();
    let manifest_url = match resolve_manifest_url().await {
        Ok(url) => url,
        Err(err) => {
            eprintln!("Update check failed: {}", err);
            if check_only {
                return 2;
            }
            return 1;
        }
    };

    println!("harper v{}", crate::CLI_VERSION);
    println!("Install source: {}", install_source.display_name());
    println!("Target: {}", target);

    println!("Manifest: {}", manifest_url);

    let manifest = match fetch_release_manifest(&manifest_url).await {
        Ok(manifest) => manifest,
        Err(err) => {
            eprintln!("Update check failed: {}", err);
            if check_only {
                return 2;
            }
            return 1;
        }
    };

    if install_source != InstallSource::Unknown {
        if let Err(err) = save_persisted_install_source(install_source) {
            eprintln!("Warning: failed to persist install metadata: {}", err);
        }
    }

    let result = evaluate_update(crate::CLI_VERSION, &manifest, &target);
    println!("Latest version: {}", result.latest_version);
    let homebrew_path_fix =
        detect_homebrew_path_fix_for_install_source(&executable, install_source);

    if !result.artifact_available {
        println!(
            "No release artifact is available for target {} in the current manifest.",
            result.target
        );
        return 2;
    }

    match compare_versions(&result.current_version, &result.latest_version) {
        Some(std::cmp::Ordering::Less) => {
            println!(
                "Update available: {} -> {}",
                result.current_version, result.latest_version
            );
        }
        Some(std::cmp::Ordering::Equal) => {
            println!("You are already on the latest version.");
        }
        Some(std::cmp::Ordering::Greater) => {
            println!("Current version is newer than the published manifest.");
        }
        None => {
            println!("Could not compare current and latest versions.");
            return 2;
        }
    }

    if check_only {
        if install_source == InstallSource::Homebrew || homebrew_path_fix.is_some() {
            if result.update_available {
                println!("Run: brew upgrade coccinella-labs/tap/harper-ai");
            }
            print_homebrew_shadow_guidance(&executable, install_source);
        }
        return 0;
    }

    if !result.update_available {
        return 0;
    }

    match install_source {
        _ if homebrew_path_fix.is_some() => {
            print_install_source_guidance(InstallSource::Homebrew, &executable)
        }
        InstallSource::Homebrew | InstallSource::Cargo | InstallSource::Npm => {
            print_install_source_guidance(install_source, &executable)
        }
        InstallSource::Direct => {
            let Some(artifact) = manifest.artifact_for_target(&target) else {
                println!(
                    "No release artifact is available for target {} in the current manifest.",
                    target
                );
                return 2;
            };

            let bytes = match download_release_artifact(&artifact.url).await {
                Ok(bytes) => bytes,
                Err(err) => {
                    eprintln!("Update download failed: {}", err);
                    return 1;
                }
            };

            if let Err(err) = verify_artifact_checksum(&bytes, &artifact.sha256) {
                eprintln!("Update verification failed: {}", err);
                return 1;
            }

            let Some(signature) = artifact.signature.as_deref() else {
                eprintln!("Update verification failed: release artifact signature is missing.");
                return 1;
            };

            let public_key = match resolve_update_public_key() {
                Ok(key) => key,
                Err(err) => {
                    eprintln!("Update verification failed: {}", err);
                    return 1;
                }
            };

            if let Err(err) = verify_artifact_signature(&bytes, signature, &public_key) {
                eprintln!("Update verification failed: {}", err);
                return 1;
            }

            let executable_name = match executable.file_name().and_then(|name| name.to_str()) {
                Some(name) => name,
                None => {
                    eprintln!("Could not determine executable filename for update installation.");
                    return 1;
                }
            };

            let install_bytes =
                match extract_release_executable(&artifact.url, &bytes, executable_name) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        eprintln!("Update extraction failed: {}", err);
                        return 1;
                    }
                };

            if let Err(err) = install_downloaded_executable(&executable, &install_bytes) {
                eprintln!("Update install failed: {}", err);
                return 1;
            }

            if let Err(err) = save_persisted_install_source(InstallSource::Direct) {
                eprintln!(
                    "Updated Harper, but failed to persist install metadata: {}",
                    err
                );
                return 1;
            }

            println!("Updated Harper to {}.", result.latest_version);
            0
        }
        InstallSource::Unknown => {
            println!("Install source is unknown.");
            println!("Refusing to mutate this install automatically.");
            println!("Use `harper self-update --check` and update the binary manually.");
            2
        }
    }
}

fn print_install_source_guidance(install_source: InstallSource, executable: &Path) -> i32 {
    match install_source {
        InstallSource::Homebrew => {
            println!("This install is managed by Homebrew.");
            println!("Run: brew upgrade coccinella-labs/tap/harper-ai");
            print_homebrew_shadow_guidance(executable, install_source);
            0
        }
        InstallSource::Cargo => {
            println!("This install is managed by cargo.");
            println!("Run: cargo install harper-ui --force");
            0
        }
        InstallSource::Npm => {
            println!("This install is managed by npm.");
            println!("Run: npm install -g harper-ai@latest");
            0
        }
        InstallSource::Direct | InstallSource::Unknown => {
            println!("This install is not managed by a package manager.");
            println!("Use `harper self-update --check` to inspect the published release manifest.");
            2
        }
    }
}

fn print_homebrew_shadow_guidance(executable: &Path, install_source: InstallSource) {
    let Some(fix) = detect_homebrew_path_fix_for_install_source(executable, install_source) else {
        return;
    };

    let executable_text = fix.shadow_path.to_string_lossy();
    let backup_path = next_backup_path(&fix.shadow_path);
    let executable_arg = shell_quote(&executable_text);
    let backup_arg = shell_quote(&backup_path.to_string_lossy());
    let homebrew_arg = shell_quote(&fix.homebrew_path.to_string_lossy());
    println!();
    println!("Your shell is running another Harper binary:");
    println!("  {}", fix.shadow_path.display());
    println!();
    println!("To make it use Homebrew's Harper:");
    println!(
        "  mv {executable_arg} {backup_arg}; ln -sfn {homebrew_arg} {executable_arg}; hash -r"
    );
}

pub fn detect_homebrew_path_fix() -> Option<HomebrewPathFix> {
    let executable = env::current_exe().ok()?;
    let install_source = resolve_install_source(&executable).ok()?;
    detect_homebrew_path_fix_for_install_source(&executable, install_source)
}

fn detect_homebrew_path_fix_for_install_source(
    executable: &Path,
    install_source: InstallSource,
) -> Option<HomebrewPathFix> {
    detect_homebrew_path_fix_for_executable_with_candidates(
        executable,
        install_source,
        &[
            PathBuf::from("/opt/homebrew/bin/harper"),
            PathBuf::from("/usr/local/bin/harper"),
        ],
    )
}

fn detect_homebrew_path_fix_for_executable_with_candidates(
    executable: &Path,
    install_source: InstallSource,
    homebrew_paths: &[PathBuf],
) -> Option<HomebrewPathFix> {
    let home = PathBuf::from(env::var_os("HOME")?);
    detect_homebrew_path_fix_for_executable_with_home(
        executable,
        install_source,
        &home,
        homebrew_paths,
    )
}

fn detect_homebrew_path_fix_for_executable_with_home(
    executable: &Path,
    install_source: InstallSource,
    home: &Path,
    homebrew_paths: &[PathBuf],
) -> Option<HomebrewPathFix> {
    if install_source != InstallSource::Homebrew {
        return None;
    }

    if InstallSource::infer_from_executable(executable) == InstallSource::Homebrew {
        return None;
    }

    let local_harper = home.join(".local").join("bin").join("harper");
    if executable != local_harper {
        return None;
    }

    let homebrew_path = homebrew_paths.iter().find(|path| path.exists())?.clone();

    Some(HomebrewPathFix {
        shadow_path: executable.to_path_buf(),
        homebrew_path,
    })
}

pub fn apply_homebrew_path_fix(fix: &HomebrewPathFix) -> Result<PathBuf, String> {
    #[cfg(not(unix))]
    {
        let _ = fix;
        Err("Homebrew PATH fix is only supported on Unix-like systems.".to_string())
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let backup_path = next_backup_path(&fix.shadow_path);
        fs::rename(&fix.shadow_path, &backup_path).map_err(|err| {
            format!(
                "failed to move {} to {}: {}",
                fix.shadow_path.display(),
                backup_path.display(),
                err
            )
        })?;
        if let Err(err) = symlink(&fix.homebrew_path, &fix.shadow_path) {
            let rollback = fs::rename(&backup_path, &fix.shadow_path)
                .map(|_| String::new())
                .unwrap_or_else(|rollback_err| {
                    format!(
                        " rollback failed: {} is still at {}: {}",
                        fix.shadow_path.display(),
                        backup_path.display(),
                        rollback_err
                    )
                });
            return Err(format!(
                "failed to link {} to {}: {}{}",
                fix.shadow_path.display(),
                fix.homebrew_path.display(),
                err,
                rollback
            ));
        }
        Ok(backup_path)
    }
}

fn next_backup_path(path: &Path) -> PathBuf {
    let first = PathBuf::from(format!("{}.old", path.display()));
    if !first.exists() {
        return first;
    }

    for index in 1..100 {
        let candidate = PathBuf::from(format!("{}.old.{}", path.display(), index));
        if !candidate.exists() {
            return candidate;
        }
    }

    PathBuf::from(format!("{}.old.{}", path.display(), std::process::id()))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn configured_manifest_url() -> Option<String> {
    env::var(UPDATE_MANIFEST_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn resolve_manifest_url() -> Result<String, String> {
    if let Some(url) = configured_manifest_url() {
        return Ok(url);
    }

    let client = reqwest::Client::builder()
        .user_agent(format!("harper-ui/{}", crate::CLI_VERSION))
        .build()
        .map_err(|err| format!("failed to build update client: {}", err))?;

    let response = client
        .get(RELEASES_API_URL)
        .send()
        .await
        .map_err(|err| format!("API error: failed to fetch releases: {}", err))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| format!("API error: failed to read releases response: {}", err))?;

    if !status.is_success() {
        return Err(format!(
            "API error: release list request failed with status {}",
            status
        ));
    }

    let releases: Vec<GitHubRelease> = serde_json::from_str(&body)
        .map_err(|err| format!("API error: invalid releases response: {}", err))?;

    find_manifest_url(&releases).ok_or_else(|| {
        format!(
            "API error: no stable release asset named {} was found",
            MANIFEST_ASSET_NAME
        )
    })
}

fn find_manifest_url(releases: &[GitHubRelease]) -> Option<String> {
    releases
        .iter()
        .filter(|release| !release.draft && !release.prerelease)
        .flat_map(|release| release.assets.iter())
        .find(|asset| asset.name == MANIFEST_ASSET_NAME)
        .map(|asset| asset.browser_download_url.clone())
}

fn print_version() {
    println!("harper v{}", crate::CLI_VERSION);
}

fn print_self_update_usage() {
    eprintln!("Usage:");
    eprintln!("  harper self-update --check");
    eprintln!("  harper self-update");
}

#[cfg(test)]
mod tests {
    use super::{
        GitHubRelease, GitHubReleaseAsset, HomebrewPathFix, configured_manifest_url,
        detect_homebrew_path_fix_for_executable_with_home, fetch_update_status, find_manifest_url,
        handle_update_command, next_backup_path, shell_quote,
    };
    use harper_core::InstallSource;
    use std::fs;
    use tempfile::tempdir;

    /// Serializes tests that mutate the process-global manifest override.
    /// Async-aware mutex: the guard is held across await points, which
    /// clippy::await_holding_lock forbids for std::sync::Mutex.
    static UPDATE_MANIFEST_ENV_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> =
        std::sync::OnceLock::new();

    async fn lock_manifest_env() -> tokio::sync::MutexGuard<'static, ()> {
        UPDATE_MANIFEST_ENV_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await
    }

    /// Restores the previous manifest override on drop, even if the test panics.
    struct UpdateManifestEnvGuard {
        saved: Option<std::ffi::OsString>,
    }

    impl UpdateManifestEnvGuard {
        fn set(value: &str) -> Self {
            let saved = std::env::var_os(super::UPDATE_MANIFEST_ENV);
            unsafe {
                std::env::set_var(super::UPDATE_MANIFEST_ENV, value);
            }
            Self { saved }
        }
    }

    impl Drop for UpdateManifestEnvGuard {
        fn drop(&mut self) {
            if let Some(value) = self.saved.take() {
                unsafe {
                    std::env::set_var(super::UPDATE_MANIFEST_ENV, value);
                }
            } else {
                unsafe {
                    std::env::remove_var(super::UPDATE_MANIFEST_ENV);
                }
            }
        }
    }

    #[tokio::test]
    async fn version_command_is_handled() {
        let args = vec!["harper".to_string(), "version".to_string()];
        assert_eq!(handle_update_command(&args).await, Some(0));
    }

    #[tokio::test]
    async fn self_update_command_is_handled() {
        let _lock = lock_manifest_env().await;
        let _guard = UpdateManifestEnvGuard::set("://invalid-manifest-url");
        let args = vec![
            "harper".to_string(),
            "self-update".to_string(),
            "--check".to_string(),
        ];
        assert_eq!(handle_update_command(&args).await, Some(2));
    }

    #[tokio::test]
    async fn fetch_update_status_is_err_with_invalid_manifest_env() {
        let _lock = lock_manifest_env().await;
        let _guard = UpdateManifestEnvGuard::set("://invalid-manifest-url");
        assert_eq!(
            configured_manifest_url().as_deref(),
            Some("://invalid-manifest-url")
        );
        assert!(fetch_update_status().await.is_err());
    }

    #[test]
    fn find_manifest_url_prefers_first_stable_release_with_manifest() {
        let releases = vec![
            GitHubRelease {
                draft: false,
                prerelease: false,
                assets: vec![],
            },
            GitHubRelease {
                draft: false,
                prerelease: true,
                assets: vec![GitHubReleaseAsset {
                    name: "release-manifest.json".to_string(),
                    browser_download_url: "https://example.com/prerelease.json".to_string(),
                }],
            },
            GitHubRelease {
                draft: false,
                prerelease: false,
                assets: vec![
                    GitHubReleaseAsset {
                        name: "something-else.txt".to_string(),
                        browser_download_url: "https://example.com/other.txt".to_string(),
                    },
                    GitHubReleaseAsset {
                        name: "release-manifest.json".to_string(),
                        browser_download_url: "https://example.com/release-manifest.json"
                            .to_string(),
                    },
                ],
            },
        ];

        assert_eq!(
            find_manifest_url(&releases).as_deref(),
            Some("https://example.com/release-manifest.json")
        );
    }

    #[test]
    fn shell_quote_handles_spaces_and_quotes() {
        assert_eq!(
            shell_quote("/Users/test/local harper's/bin/harper"),
            "'/Users/test/local harper'\\''s/bin/harper'"
        );
    }

    #[test]
    fn next_backup_path_skips_existing_backups() {
        let tempdir = tempdir().expect("tempdir");
        let path = tempdir.path().join("harper");
        fs::write(&path, "old").expect("write original");
        fs::write(tempdir.path().join("harper.old"), "older").expect("write backup");

        assert_eq!(next_backup_path(&path), tempdir.path().join("harper.old.1"));
    }

    #[test]
    fn detects_shadowed_homebrew_with_homebrew_source() {
        let tempdir = tempdir().expect("tempdir");
        let home = tempdir.path().join("home");
        let shadow_path = home.join(".local").join("bin").join("harper");
        let homebrew_path = tempdir
            .path()
            .join("opt")
            .join("homebrew")
            .join("bin")
            .join("harper");
        fs::create_dir_all(shadow_path.parent().expect("shadow parent")).expect("shadow dir");
        fs::create_dir_all(homebrew_path.parent().expect("homebrew parent")).expect("homebrew dir");
        fs::write(&shadow_path, "shadow").expect("write shadow");
        fs::write(&homebrew_path, "homebrew").expect("write homebrew");

        assert_eq!(
            detect_homebrew_path_fix_for_executable_with_home(
                &shadow_path,
                InstallSource::Homebrew,
                &home,
                std::slice::from_ref(&homebrew_path),
            ),
            Some(HomebrewPathFix {
                shadow_path,
                homebrew_path,
            })
        );
    }

    #[test]
    fn does_not_treat_direct_local_install_as_homebrew_shadow() {
        let tempdir = tempdir().expect("tempdir");
        let home = tempdir.path().join("home");
        let shadow_path = home.join(".local").join("bin").join("harper");
        let homebrew_path = tempdir
            .path()
            .join("opt")
            .join("homebrew")
            .join("bin")
            .join("harper");
        fs::create_dir_all(shadow_path.parent().expect("shadow parent")).expect("shadow dir");
        fs::create_dir_all(homebrew_path.parent().expect("homebrew parent")).expect("homebrew dir");
        fs::write(&shadow_path, "direct").expect("write direct");
        fs::write(&homebrew_path, "homebrew").expect("write homebrew");

        assert_eq!(
            detect_homebrew_path_fix_for_executable_with_home(
                &shadow_path,
                InstallSource::Direct,
                &home,
                std::slice::from_ref(&homebrew_path),
            ),
            None
        );
    }

    #[test]
    fn detects_intel_homebrew_prefix() {
        let tempdir = tempdir().expect("tempdir");
        let home = tempdir.path().join("home");
        let shadow_path = home.join(".local").join("bin").join("harper");
        let apple_silicon_path = tempdir
            .path()
            .join("opt")
            .join("homebrew")
            .join("bin")
            .join("harper");
        let intel_path = tempdir
            .path()
            .join("usr")
            .join("local")
            .join("bin")
            .join("harper");
        fs::create_dir_all(shadow_path.parent().expect("shadow parent")).expect("shadow dir");
        fs::create_dir_all(intel_path.parent().expect("intel parent")).expect("intel dir");
        fs::write(&shadow_path, "shadow").expect("write shadow");
        fs::write(&intel_path, "homebrew").expect("write homebrew");

        assert_eq!(
            detect_homebrew_path_fix_for_executable_with_home(
                &shadow_path,
                InstallSource::Homebrew,
                &home,
                &[apple_silicon_path, intel_path.clone()],
            )
            .map(|fix| fix.homebrew_path),
            Some(intel_path)
        );
    }

    #[cfg(unix)]
    #[test]
    fn apply_homebrew_path_fix_moves_shadow_and_links_homebrew() {
        let tempdir = tempdir().expect("tempdir");
        let shadow_path = tempdir.path().join("harper");
        let homebrew_path = tempdir.path().join("homebrew-harper");
        fs::write(&shadow_path, "shadow").expect("write shadow");
        fs::write(&homebrew_path, "homebrew").expect("write homebrew");

        let backup = super::apply_homebrew_path_fix(&HomebrewPathFix {
            shadow_path: shadow_path.clone(),
            homebrew_path: homebrew_path.clone(),
        })
        .expect("apply fix");

        assert_eq!(backup, tempdir.path().join("harper.old"));
        assert_eq!(fs::read_to_string(&backup).expect("read backup"), "shadow");
        assert_eq!(
            fs::read_link(&shadow_path).expect("read symlink"),
            homebrew_path
        );
    }
}
