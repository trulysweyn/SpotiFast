use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::backend::app_data_dir;

const YT_DLP_RELEASES: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";
const MPV_RELEASES: &str =
    "https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases/latest";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    YtDlp,
    Mpv,
}

impl ToolKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::YtDlp => "yt-dlp",
            Self::Mpv => "mpv",
        }
    }

    fn folder(self) -> &'static str {
        match self {
            Self::YtDlp => "yt-dlp",
            Self::Mpv => "mpv",
        }
    }

    fn binary(self) -> &'static str {
        match self {
            Self::YtDlp => "yt-dlp.exe",
            Self::Mpv => "mpv.exe",
        }
    }

    fn repo(self) -> &'static str {
        match self {
            Self::YtDlp => YT_DLP_RELEASES,
            Self::Mpv => MPV_RELEASES,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ToolStatus {
    pub kind: ToolKind,
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub rollback_available: bool,
}

#[derive(Clone)]
struct ReleaseAsset {
    version: String,
    url: String,
    name: String,
}

#[derive(Clone, Debug)]
pub struct AppUpdateInfo {
    pub repository: String,
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub rollback_available: bool,
}

fn normalize_repository(input: &str) -> Result<String, String> {
    let value = input
        .trim()
        .trim_end_matches('/')
        .trim_end_matches("/releases")
        .trim_end_matches("/releases/latest");
    let value = value
        .strip_prefix("https://github.com/")
        .or_else(|| value.strip_prefix("http://github.com/"))
        .unwrap_or(value);
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or("");
    let repository = parts.next().unwrap_or("").trim_end_matches(".git");
    let valid = |part: &str| {
        !part.is_empty()
            && part.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
    };
    if !valid(owner) || !valid(repository) || parts.next().is_some() {
        return Err("Enter a GitHub repository as owner/repository or paste its URL.".to_string());
    }
    Ok(format!("{owner}/{repository}"))
}

fn app_release(repository: &str) -> Result<(String, String, String, String), String> {
    let repository = normalize_repository(repository)?;
    let api = format!("https://api.github.com/repos/{repository}/releases/latest");
    let value: Value = http_agent()
        .get(&api)
        .call()
        .map_err(|error| format!("Could not check SpotiFast releases: {error}"))?
        .into_json()
        .map_err(|error| format!("Could not read the SpotiFast release: {error}"))?;
    let tag = value
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or_else(|| "The latest release has no version tag.".to_string())?
        .to_string();
    let assets = value
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| "The latest release has no downloadable files.".to_string())?;
    let asset_url = |names: &[&str]| {
        assets
            .iter()
            .find(|asset| {
                asset
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| {
                        names
                            .iter()
                            .any(|candidate| name.eq_ignore_ascii_case(candidate))
                    })
            })
            .and_then(|asset| asset.get("browser_download_url"))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let app_url = asset_url(&["SpotiFast.exe", "spotifast.exe"])
        .ok_or_else(|| "The release needs a SpotiFast.exe asset.".to_string())?;
    let helper_url = asset_url(&[
        "SpotiFastUpdater.exe",
        "spotifast_updater.exe",
        "spotifast-updater.exe",
    ])
    .ok_or_else(|| {
        "The release needs a SpotiFastUpdater.exe asset for safe replacement.".to_string()
    })?;
    Ok((repository, tag, app_url, helper_url))
}

fn version_without_prefix(version: &str) -> &str {
    version.trim().trim_start_matches(['v', 'V'])
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    let latest = comparable_version(version_without_prefix(latest));
    let current = comparable_version(version_without_prefix(current));
    latest > current
}

pub fn check_app_updates(repository: &str) -> Result<AppUpdateInfo, String> {
    let (repository, latest_version, _, _) = app_release(repository)?;
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    Ok(AppUpdateInfo {
        repository,
        update_available: is_newer_version(&latest_version, &current_version),
        latest_version,
        current_version,
        rollback_available: app_rollback_available(),
    })
}

fn app_update_dir() -> PathBuf {
    app_data_dir().join("app_updates")
}

fn app_rollback_path() -> PathBuf {
    app_data_dir().join("previous_spotifast.exe")
}

pub fn app_rollback_available() -> bool {
    app_rollback_path().is_file()
}

pub fn stage_app_update(repository: &str) -> Result<PathBuf, String> {
    let (_, latest_version, app_url, helper_url) = app_release(repository)?;
    let update_dir = app_update_dir();
    fs::create_dir_all(&update_dir)
        .map_err(|error| format!("Could not prepare app update folder: {error}"))?;
    let staged = update_dir.join("SpotiFast-new.exe");
    let helper = update_dir.join("SpotiFastUpdater.exe");
    fs::remove_file(&staged).ok();
    download_to(&app_url, &staged)?;
    download_to(&helper_url, &helper)?;

    let mut command = Command::new(&staged);
    command.arg("--version");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command
        .output()
        .map_err(|error| format!("Could not validate the downloaded SpotiFast build: {error}"))?;
    let downloaded_version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success()
        || !downloaded_version.starts_with(version_without_prefix(&latest_version))
    {
        fs::remove_file(&staged).ok();
        return Err("The downloaded SpotiFast build did not pass its version check.".to_string());
    }
    Ok(staged)
}

pub fn stage_app_rollback() -> Result<PathBuf, String> {
    let previous = app_rollback_path();
    if !previous.is_file() {
        return Err("There is no previous SpotiFast version to restore.".to_string());
    }
    let update_dir = app_update_dir();
    fs::create_dir_all(&update_dir)
        .map_err(|error| format!("Could not prepare app rollback: {error}"))?;
    let staged = update_dir.join("SpotiFast-rollback.exe");
    fs::copy(&previous, &staged)
        .map_err(|error| format!("Could not stage the previous app: {error}"))?;
    Ok(staged)
}

pub fn launch_app_update(staged: &Path) -> Result<(), String> {
    let helper = app_update_dir().join("SpotiFastUpdater.exe");
    if !helper.is_file() {
        return Err("The update helper is missing. Download the app update again.".to_string());
    }
    let target = std::env::current_exe()
        .map_err(|error| format!("Could not find the installed SpotiFast executable: {error}"))?;
    let rollback = app_rollback_path();
    let mut command = Command::new(helper);
    command
        .arg(&target)
        .arg(staged)
        .arg(rollback)
        .arg(std::process::id().to_string());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .spawn()
        .map_err(|error| format!("Could not start the SpotiFast updater: {error}"))?;
    Ok(())
}

fn tool_root(kind: ToolKind) -> PathBuf {
    app_data_dir().join("tools").join(kind.folder())
}

fn current_dir(kind: ToolKind) -> PathBuf {
    tool_root(kind).join("current")
}

fn previous_dir(kind: ToolKind) -> PathBuf {
    tool_root(kind).join("previous")
}

pub fn tool_executable(name: &str) -> PathBuf {
    let (kind, exe) = if name.eq_ignore_ascii_case("mpv") || name.eq_ignore_ascii_case("mpv.exe") {
        (ToolKind::Mpv, "mpv.exe")
    } else {
        (ToolKind::YtDlp, "yt-dlp.exe")
    };
    let managed = current_dir(kind).join(exe);
    if managed.is_file() {
        return managed;
    }

    let path_name = if name.to_ascii_lowercase().ends_with(".exe") {
        name.to_string()
    } else {
        format!("{name}.exe")
    };
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            let candidate = directory.join(&path_name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    PathBuf::from(name)
}

fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(45))
        .user_agent("SpotiFast updater")
        .build()
}

fn latest_release(kind: ToolKind) -> Result<(String, ReleaseAsset), String> {
    let value: Value = http_agent()
        .get(kind.repo())
        .call()
        .map_err(|error| format!("Could not check {} releases: {error}", kind.label()))?
        .into_json()
        .map_err(|error| {
            format!(
                "Could not read {} release information: {error}",
                kind.label()
            )
        })?;

    let version = value
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} release did not include a version tag", kind.label()))?
        .to_string();
    let assets = value
        .get("assets")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!(
                "{} release did not include downloadable assets",
                kind.label()
            )
        })?;

    let asset = match kind {
        ToolKind::YtDlp => assets
            .iter()
            .find(|asset| asset.get("name").and_then(Value::as_str) == Some("yt-dlp.exe")),
        ToolKind::Mpv => {
            let arch = if cfg!(target_arch = "aarch64") {
                "aarch64"
            } else if cfg!(target_arch = "x86") {
                "i686"
            } else {
                "x86_64"
            };
            assets.iter().find(|asset| {
                let name = asset.get("name").and_then(Value::as_str).unwrap_or("");
                name.starts_with(&format!("mpv-{arch}-")) && name.ends_with(".7z")
            })
        }
    }
    .ok_or_else(|| format!("No compatible {} update was found", kind.label()))?;

    let name = asset
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} release asset has no filename", kind.label()))?
        .to_string();
    let url = asset
        .get("browser_download_url")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} release asset has no download URL", kind.label()))?
        .to_string();
    let asset_version = if kind == ToolKind::Mpv {
        name.strip_prefix("mpv-")
            .and_then(|s| s.split_once("-git-"))
            .map(|(date, hash)| format!("{date}-git-{}", hash.trim_end_matches(".7z")))
            .unwrap_or_else(|| version.clone())
    } else {
        version.clone()
    };

    Ok((
        version,
        ReleaseAsset {
            version: asset_version,
            url,
            name,
        },
    ))
}

fn read_version(path: &Path, kind: ToolKind) -> String {
    if !path.is_file() {
        return "Not found".to_string();
    }
    let mut command = Command::new(path);
    command.arg("--version");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command.output();
    let Ok(output) = output else {
        return "Installed (version unavailable)".to_string();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let first_line = text.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return if kind == ToolKind::YtDlp {
            "Installed (version unavailable)".to_string()
        } else {
            String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or("Installed (version unavailable)")
                .trim()
                .to_string()
        };
    }
    first_line.to_string()
}

fn comparable_version(value: &str) -> Vec<u64> {
    value
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

fn has_update(kind: ToolKind, current: &str, latest: &str, asset_name: &str) -> bool {
    if current == "Not found" {
        return true;
    }
    if kind == ToolKind::YtDlp {
        let current = current.trim();
        return comparable_version(current) < comparable_version(latest);
    }

    let current_lower = current.to_ascii_lowercase();
    let latest_hash = asset_name
        .split("-git-")
        .nth(1)
        .unwrap_or("")
        .trim_end_matches(".7z")
        .to_ascii_lowercase();
    !latest_hash.is_empty() && !current_lower.contains(&latest_hash)
}

pub fn check_for_updates() -> Result<Vec<ToolStatus>, String> {
    let mut statuses = Vec::new();
    for kind in [ToolKind::YtDlp, ToolKind::Mpv] {
        let (release_version, asset) = latest_release(kind)?;
        let current = tool_executable(kind.label());
        let current_version = read_version(&current, kind);
        statuses.push(ToolStatus {
            kind,
            update_available: has_update(kind, &current_version, &release_version, &asset.name),
            current_version,
            latest_version: asset.version,
            rollback_available: previous_dir(kind).join(kind.binary()).is_file(),
        });
    }
    Ok(statuses)
}

fn copy_file(source: &Path, destination: &Path) -> Result<(), String> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create backup folder: {error}"))?;
    }
    fs::copy(source, destination)
        .map_err(|error| format!("Could not back up {}: {error}", source.display()))?;
    Ok(())
}

fn copy_dir(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("Could not create backup folder: {error}"))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("Could not read {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("Could not read backup entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Could not inspect backup entry: {error}"))?;
        let to = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else if file_type.is_file() {
            copy_file(&entry.path(), &to)?;
        }
    }
    Ok(())
}

fn download_to(url: &str, destination: &Path) -> Result<(), String> {
    let response = http_agent()
        .get(url)
        .call()
        .map_err(|error| format!("Download failed: {error}"))?;
    let mut file = fs::File::create(destination)
        .map_err(|error| format!("Could not create update file: {error}"))?;
    let mut reader = response.into_reader().take(300 * 1024 * 1024);
    std::io::copy(&mut reader, &mut file)
        .map_err(|error| format!("Could not save update: {error}"))?;
    file.flush()
        .map_err(|error| format!("Could not finish saving update: {error}"))?;
    Ok(())
}

fn locate_binary(root: &Path, name: &str) -> Option<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case(name))
            {
                return Some(path);
            }
            if entry.file_type().ok()?.is_dir() {
                pending.push(path);
            }
        }
    }
    None
}

fn prepare_backup(kind: ToolKind, backup_stage: &Path) -> Result<bool, String> {
    let managed = current_dir(kind);
    if managed.join(kind.binary()).is_file() {
        copy_dir(&managed, backup_stage)?;
        return Ok(true);
    }

    let installed = tool_executable(kind.label());
    if !installed.is_file() {
        return Ok(false);
    }
    if kind == ToolKind::Mpv {
        if let Some(parent) = installed.parent() {
            copy_dir(parent, backup_stage)?;
            return Ok(true);
        }
    }
    copy_file(&installed, &backup_stage.join(kind.binary()))?;
    Ok(true)
}

pub fn install_update(kind: ToolKind) -> Result<ToolStatus, String> {
    let (release_version, asset) = latest_release(kind)?;
    let root = tool_root(kind);
    fs::create_dir_all(&root).map_err(|error| format!("Could not create tools folder: {error}"))?;
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let staging = root.join(format!("staging-{nonce}"));
    let backup_staging = root.join(format!("backup-{nonce}"));
    let old_current = root.join(format!("old-current-{nonce}"));
    fs::create_dir_all(&staging)
        .map_err(|error| format!("Could not create staging folder: {error}"))?;

    let result = (|| {
        let download = staging.join(&asset.name);
        download_to(&asset.url, &download)?;
        match kind {
            ToolKind::YtDlp => {
                let executable = staging.join(kind.binary());
                if download != executable {
                    fs::rename(&download, &executable)
                        .map_err(|error| format!("Could not stage yt-dlp: {error}"))?;
                }
            }
            ToolKind::Mpv => {
                let extracted = staging.join("extracted");
                sevenz_rust::decompress_file(&download, &extracted)
                    .map_err(|error| format!("Could not unpack mpv update: {error}"))?;
                let binary = locate_binary(&extracted, kind.binary())
                    .ok_or_else(|| "The mpv update archive did not contain mpv.exe".to_string())?;
                let package_root = binary
                    .parent()
                    .ok_or_else(|| "Could not locate the mpv package folder".to_string())?;
                let payload = staging.join("payload");
                fs::rename(package_root, &payload)
                    .map_err(|error| format!("Could not stage mpv files: {error}"))?;
                fs::remove_file(&download).ok();
                fs::remove_dir_all(&extracted).ok();
                let entries = fs::read_dir(&payload)
                    .map_err(|error| format!("Could not inspect mpv package: {error}"))?;
                for entry in entries.flatten() {
                    fs::rename(entry.path(), staging.join(entry.file_name()))
                        .map_err(|error| format!("Could not finish staging mpv: {error}"))?;
                }
                fs::remove_dir_all(payload).ok();
            }
        }

        let staged_binary = staging.join(kind.binary());
        if !staged_binary.is_file() {
            return Err(format!(
                "The downloaded {} package is incomplete",
                kind.label()
            ));
        }
        let version = read_version(&staged_binary, kind);
        if version == "Not found" || version.contains("unavailable") {
            return Err(format!(
                "The downloaded {} binary did not pass its version check",
                kind.label()
            ));
        }

        let had_backup = prepare_backup(kind, &backup_staging)?;
        let current = current_dir(kind);
        if current.exists() {
            fs::rename(&current, &old_current).map_err(|error| {
                format!(
                    "Could not stage the installed {} version: {error}",
                    kind.label()
                )
            })?;
        }
        if let Err(error) = fs::rename(&staging, &current) {
            if old_current.exists() {
                let _ = fs::rename(&old_current, &current);
            }
            return Err(format!(
                "Could not activate {} update: {error}",
                kind.label()
            ));
        }

        if had_backup {
            let previous = previous_dir(kind);
            if previous.exists() {
                fs::remove_dir_all(&previous).map_err(|error| {
                    format!("Update installed, but old rollback data could not be rotated: {error}")
                })?;
            }
            fs::rename(&backup_staging, &previous).map_err(|error| {
                format!("Update installed, but rollback could not be saved: {error}")
            })?;
        }
        if old_current.exists() {
            fs::remove_dir_all(old_current).ok();
        }

        Ok(ToolStatus {
            kind,
            current_version: version,
            latest_version: asset.version,
            update_available: false,
            rollback_available: previous_dir(kind).join(kind.binary()).is_file(),
        })
    })();

    if staging.exists() {
        fs::remove_dir_all(&staging).ok();
    }
    if backup_staging.exists() {
        fs::remove_dir_all(&backup_staging).ok();
    }
    result.map(|mut status| {
        status.latest_version = release_version;
        status
    })
}

pub fn rollback(kind: ToolKind) -> Result<ToolStatus, String> {
    let current = current_dir(kind);
    let previous = previous_dir(kind);
    if !previous.join(kind.binary()).is_file() {
        return Err(format!(
            "There is no saved {} version to roll back to",
            kind.label()
        ));
    }
    let temporary = tool_root(kind).join(format!("rollback-{}", std::process::id()));
    if temporary.exists() {
        fs::remove_dir_all(&temporary)
            .map_err(|error| format!("Could not prepare rollback: {error}"))?;
    }
    if current.exists() {
        fs::rename(&current, &temporary)
            .map_err(|error| format!("Could not prepare {} rollback: {error}", kind.label()))?;
    }
    if let Err(error) = fs::rename(&previous, &current) {
        if temporary.exists() {
            let _ = fs::rename(&temporary, &current);
        }
        return Err(format!("Could not roll back {}: {error}", kind.label()));
    }
    fs::rename(&temporary, &previous).ok();

    let current_version = read_version(&current.join(kind.binary()), kind);
    let previous_version = previous
        .join(kind.binary())
        .is_file()
        .then(|| read_version(&previous.join(kind.binary()), kind))
        .unwrap_or_else(|| "None".to_string());
    Ok(ToolStatus {
        kind,
        current_version,
        latest_version: previous_version,
        update_available: false,
        rollback_available: previous.join(kind.binary()).is_file(),
    })
}

pub fn rollback_available(kind: ToolKind) -> bool {
    previous_dir(kind).join(kind.binary()).is_file()
}
