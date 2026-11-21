use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

use crate::models::{AppSettings, CacheInfo, PersistedState, PlaylistDump, PlaylistEntry};
use crate::updater::{AppUpdateInfo, ToolStatus};

pub const MPV_PIPE_NAME: &str = r"\\.\pipe\yt-playlist-mpv";
pub const CREATE_NO_WINDOW: u32 = 0x08000000;

pub struct ProcessGuard {
    job: HANDLE,
    next_id: usize,
    children: HashMap<usize, Child>,
}

impl ProcessGuard {
    pub fn new() -> Self {
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if !job.is_null() {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
            }
        }
        Self {
            job,
            next_id: 1,
            children: HashMap::new(),
        }
    }

    pub fn track(&mut self, child: Child) -> usize {
        if !self.job.is_null() {
            unsafe {
                let _ = AssignProcessToJobObject(self.job, child.as_raw_handle() as HANDLE);
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        self.children.insert(id, child);
        id
    }

    pub fn kill_and_remove(&mut self, id: usize, save_position: bool) {
        if let Some(mut child) = self.children.remove(&id) {
            if save_position {
                let _ = send_mpv_ipc("{\"command\":[\"write-watch-later-config\"]}\n");
            }
            let _ = send_mpv_ipc("{\"command\":[\"quit\"]}\n");
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline {
                match child.try_wait() {
                    Ok(Some(_)) => return,
                    Ok(None) => thread::sleep(Duration::from_millis(15)),
                    Err(_) => break,
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    pub fn try_wait_id(&mut self, id: usize) -> Option<bool> {
        let child = self.children.get_mut(&id)?;
        match child.try_wait() {
            Ok(Some(_)) => {
                self.children.remove(&id);
                Some(true)
            }
            Ok(None) => Some(false),
            Err(_) => {
                self.children.remove(&id);
                Some(true)
            }
        }
    }

    pub fn reap_finished(&mut self) {
        let mut done = Vec::new();
        for (&id, child) in self.children.iter_mut() {
            if let Ok(Some(_)) = child.try_wait() {
                done.push(id);
            }
        }
        for id in done {
            self.children.remove(&id);
        }
    }

    pub fn kill_all(&mut self) {
        for (_, mut child) in self.children.drain() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if !self.job.is_null() {
            unsafe {
                CloseHandle(self.job);
            }
            self.job = std::ptr::null_mut();
        }
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        self.kill_all();
    }
}

pub fn app_data_dir() -> PathBuf {
    let mut dir = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    dir.push("yt-playlist-gui");
    dir
}

pub fn cache_dir_path() -> PathBuf {
    let mut dir = app_data_dir();
    dir.push("cache");
    dir
}

pub fn state_file_path() -> PathBuf {
    let mut dir = app_data_dir();
    dir.push("player_state.json");
    dir
}

pub fn settings_file_path() -> PathBuf {
    let mut dir = app_data_dir();
    dir.push("settings.json");
    dir
}

pub fn load_settings() -> AppSettings {
    fs::read_to_string(settings_file_path())
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

pub fn save_settings(settings: &AppSettings) {
    let path = settings_file_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = fs::write(path, json);
    }
}

pub fn set_run_at_startup(enabled: bool) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    use winreg::RegKey;

    let run_key_path = r"Software\Microsoft\Windows\CurrentVersion\Run";
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let run_key = current_user
        .open_subkey_with_flags(run_key_path, KEY_SET_VALUE)
        .map_err(|error| format!("Could not open the Windows startup settings: {error}"))?;

    if enabled {
        let executable = std::env::current_exe()
            .map_err(|error| format!("Could not locate SpotiFast.exe: {error}"))?;
        let command = format!("\"{}\"", executable.display());
        run_key
            .set_value("SpotiFast", &command)
            .map_err(|error| format!("Could not enable Run at startup: {error}"))
    } else {
        match run_key.delete_value("SpotiFast") {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("Could not disable Run at startup: {error}")),
        }
    }
}

pub fn is_valid_audio_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    !matches!(ext.as_str(), "part" | "ytdl" | "tmp" | "temp")
        && matches!(
            ext.as_str(),
            "webm" | "m4a" | "opus" | "ogg" | "mp3" | "aac" | "wav" | "flac" | "mp4"
        )
}

pub fn find_cached_audio(cache_key: &str) -> Option<PathBuf> {
    let dir = cache_dir_path();
    let entries = fs::read_dir(&dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_valid_audio_file(&path) {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if stem == cache_key {
            return Some(path);
        }
    }
    None
}

pub fn scan_cache_info(known_entries: &[PlaylistEntry]) -> (HashSet<String>, Vec<CacheInfo>, u64) {
    let mut keys = HashSet::new();
    let mut items = Vec::new();
    let mut total_bytes = 0u64;

    let mut lookup: HashMap<String, &PlaylistEntry> = HashMap::new();
    for entry in known_entries {
        if let Some(key) = entry.cache_key() {
            lookup.insert(key, entry);
        }
    }

    let dir = cache_dir_path();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if is_valid_audio_file(&path) {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let key = stem.to_string();
                    keys.insert(key.clone());
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    total_bytes += size;

                    let (title, artist, duration) = if let Some(known) = lookup.get(&key) {
                        (
                            known.display_title(),
                            known.display_artist(),
                            known.duration,
                        )
                    } else {
                        (
                            format!("Cached Track ({key})"),
                            "Local File".to_string(),
                            None,
                        )
                    };

                    items.push(CacheInfo {
                        key,
                        path,
                        file_size_bytes: size,
                        title,
                        artist,
                        duration,
                    });
                }
            }
        }
    }
    items.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    (keys, items, total_bytes)
}

pub fn clear_audio_cache() -> Result<usize, String> {
    let dir = cache_dir_path();
    if !dir.exists() {
        return Ok(0);
    }
    let mut removed = 0usize;
    for entry in fs::read_dir(&dir)
        .map_err(|e| format!("Failed reading cache dir: {e}"))?
        .flatten()
    {
        let path = entry.path();
        if path.is_file() && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn load_persisted_state() -> PersistedState {
    let path = state_file_path();
    let Ok(data) = fs::read_to_string(&path) else {
        return PersistedState {
            playlist_title: "My Playlist".to_string(),
            auto_advance: true,
            loop_playlist: true,
            volume: 80.0,
            ..Default::default()
        };
    };
    let mut state: PersistedState =
        serde_json::from_str(&data).unwrap_or_else(|_| PersistedState {
            playlist_title: "My Playlist".to_string(),
            auto_advance: true,
            loop_playlist: true,
            volume: 80.0,
            ..Default::default()
        });
    for entry in &mut state.tracks {
        entry.ensure_lower();
    }
    for entry in &mut state.liked_tracks {
        entry.ensure_lower();
    }
    if state.playlist_title.trim().is_empty() {
        state.playlist_title = "My Playlist".to_string();
    }
    if !(0.0..=100.0).contains(&state.volume) {
        state.volume = 80.0;
    }
    state
}

pub fn save_persisted_state(state: &PersistedState) {
    let path = state_file_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = fs::write(path, json);
    }
}

pub fn send_mpv_ipc(json_cmd: &str) -> Result<(), String> {
    let mut pipe = OpenOptions::new()
        .write(true)
        .open(MPV_PIPE_NAME)
        .map_err(|e| format!("IPC pipe unavailable: {e}"))?;
    pipe.write_all(json_cmd.as_bytes())
        .map_err(|e| format!("Failed writing to mpv pipe: {e}"))?;
    pipe.flush()
        .map_err(|e| format!("Failed flushing mpv pipe: {e}"))?;
    Ok(())
}

pub fn seek_mpv(seconds: f64) -> Result<(), String> {
    let s = seconds.max(0.0).round() as u64;
    send_mpv_ipc(&format!("{{\"command\":[\"seek\",{s},\"absolute\"]}}\n"))
}

pub fn set_mpv_volume(volume: f32) -> Result<(), String> {
    let vol_val = volume.clamp(0.0, 100.0).round() as u32;
    send_mpv_ipc(&format!(
        "{{\"command\":[\"set_property\",\"volume\",{vol_val}]}}\n"
    ))
}

pub fn cycle_mpv_pause() -> Result<(), String> {
    send_mpv_ipc("{\"command\":[\"cycle\",\"pause\"]}\n")
}

pub fn spawn_mpv_target(
    target: &Path,
    volume: f32,
    start_position_secs: f64,
) -> Result<Child, String> {
    let vol_arg = format!("--volume={}", volume.clamp(0.0, 100.0).round() as u32);
    let start_arg = format!("--start={}", start_position_secs.max(0.0));
    let ipc_arg = format!("--input-ipc-server={MPV_PIPE_NAME}");
    let target_str = target.to_string_lossy().to_string();
    Command::new(crate::updater::tool_executable("mpv"))
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "--no-video",
            "--really-quiet",
            "--no-ytdl",
            &vol_arg,
            &start_arg,
            &ipc_arg,
            &target_str,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to launch mpv: {e}"))
}

pub fn download_track_file(watch_url: &str, key: &str) -> Result<PathBuf, String> {
    let cache_dir = cache_dir_path();
    fs::create_dir_all(&cache_dir).map_err(|e| format!("Failed creating cache dir: {e}"))?;
    let out_template = cache_dir.join(format!("{key}.%(ext)s"));
    let out_str = out_template.to_string_lossy().to_string();
    let status = Command::new(crate::updater::tool_executable("yt-dlp"))
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "--extractor-args",
            "youtube:player_client=android,web",
            "-f",
            "bestaudio/best",
            "--no-warnings",
            "--no-playlist",
            "-o",
            &out_str,
            watch_url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Failed running yt-dlp download: {e}"))?;
    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        return Err(format!("Download error: {}", stderr.trim()));
    }
    find_cached_audio(key)
        .ok_or_else(|| "Downloaded audio file was not found in cache.".to_string())
}

pub fn prepare_track_file(
    watch_url: &str,
    cache_key: Option<&str>,
    download_lock: &Mutex<()>,
) -> Result<PathBuf, String> {
    if let Some(key) = cache_key {
        if let Some(cached_file) = find_cached_audio(key) {
            return Ok(cached_file);
        }
    }
    let _guard = download_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(key) = cache_key {
        if let Some(cached_file) = find_cached_audio(key) {
            return Ok(cached_file);
        }
    }
    download_track_file(watch_url, cache_key.unwrap_or("track"))
}

pub enum WorkerMsg {
    PlaylistLoaded {
        url: String,
        title: String,
        tracks: Vec<PlaylistEntry>,
    },
    PlaylistError(String),
    SearchResultsLoaded {
        query: String,
        results: Vec<PlaylistEntry>,
    },
    SearchError(String),
    DurationsLoaded(HashMap<String, f64>),
    PlaybackReady {
        request_id: u64,
        entry: PlaylistEntry,
        target: PathBuf,
        start_position_secs: f64,
    },
    PlaybackError {
        request_id: u64,
        error: String,
    },
    PrefetchFinished(String),
    CacheDownloadFinished {
        key: String,
        success: bool,
    },
    UpdateCheckFinished(Result<Vec<ToolStatus>, String>),
    UpdateInstalled(Result<ToolStatus, String>),
    RollbackFinished(Result<ToolStatus, String>),
    AppUpdateCheckFinished(Result<AppUpdateInfo, String>),
    AppUpdateStaged(Result<PathBuf, String>),
}

pub fn run_playlist_fetch(url: String, tx: Sender<WorkerMsg>) {
    let output = Command::new(crate::updater::tool_executable("yt-dlp"))
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "--extractor-args",
            "youtube:player_client=android,web",
            "--flat-playlist",
            "-J",
            "--no-warnings",
            &url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    match output {
        Ok(out) => {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let _ = tx.send(WorkerMsg::PlaylistError(format!(
                    "yt-dlp error: {}",
                    stderr.trim()
                )));
                return;
            }
            match serde_json::from_slice::<PlaylistDump>(&out.stdout) {
                Ok(parsed) => {
                    let title = parsed
                        .title
                        .unwrap_or_else(|| "YouTube Playlist".to_string());
                    let mut entries: Vec<PlaylistEntry> = parsed
                        .entries
                        .unwrap_or_default()
                        .into_iter()
                        .flatten()
                        .filter(|e| e.watch_url().is_some())
                        .collect();
                    for item in &mut entries {
                        item.ensure_lower();
                    }
                    if entries.is_empty() {
                        let _ = tx.send(WorkerMsg::PlaylistError(
                            "No playable tracks found in playlist.".to_string(),
                        ));
                    } else {
                        let _ = tx.send(WorkerMsg::PlaylistLoaded {
                            url,
                            title,
                            tracks: entries,
                        });
                    }
                }
                Err(e) => {
                    let _ = tx.send(WorkerMsg::PlaylistError(format!("JSON parse error: {e}")));
                }
            }
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::PlaylistError(format!(
                "Failed to run yt-dlp: {e}"
            )));
        }
    }
}

pub fn run_youtube_search(query: String, tx: Sender<WorkerMsg>) {
    let is_direct_url = query.starts_with("http://")
        || query.starts_with("https://")
        || query.contains("youtube.com/")
        || query.contains("youtu.be/");

    let arg = if is_direct_url {
        query.clone()
    } else {
        format!("ytsearch25:{query}")
    };

    let output = Command::new(crate::updater::tool_executable("yt-dlp"))
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "--extractor-args",
            "youtube:player_client=android,web",
            "--flat-playlist",
            "-J",
            "--no-warnings",
            &arg,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    match output {
        Ok(out) => {
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let _ = tx.send(WorkerMsg::SearchError(format!(
                    "Search error: {}",
                    stderr.trim()
                )));
                return;
            }
            match parse_youtube_search_results(&out.stdout) {
                Ok(mut entries) if !entries.is_empty() => {
                    for item in &mut entries {
                        item.ensure_lower();
                    }
                    let _ = tx.send(WorkerMsg::SearchResultsLoaded {
                        query,
                        results: entries,
                    });
                }
                Ok(_) => {
                    let _ = tx.send(WorkerMsg::SearchError(
                        "YouTube returned no playable songs for that search. Try a different title or artist.".to_string(),
                    ));
                }
                Err(err) => {
                    let _ = tx.send(WorkerMsg::SearchError(format!(
                        "Could not parse YouTube results: {err}"
                    )));
                }
            }
        }
        Err(e) => {
            let _ = tx.send(WorkerMsg::SearchError(format!("Failed to run yt-dlp: {e}")));
        }
    }
}

pub fn run_duration_lookup(entries: Vec<PlaylistEntry>, tx: Sender<WorkerMsg>) {
    let mut durations = HashMap::new();
    let mut remote_urls = Vec::new();

    for entry in entries {
        let Some(key) = entry.cache_key() else {
            continue;
        };
        if let Some(path) = find_cached_audio(&key) {
            if let Some(duration) = probe_audio_duration(&path) {
                durations.insert(key, duration);
                continue;
            }
        }
        if let Some(url) = entry.watch_url() {
            remote_urls.push((key, url));
        }
    }

    if !remote_urls.is_empty() {
        let mut command = Command::new(crate::updater::tool_executable("yt-dlp"));
        command.creation_flags(CREATE_NO_WINDOW).args([
            "--extractor-args",
            "youtube:player_client=android,web",
            "--skip-download",
            "--no-warnings",
            "--no-playlist",
            "--dump-json",
        ]);
        for (_, url) in &remote_urls {
            command.arg(url);
        }

        if let Ok(output) = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
        {
            let requested_keys: HashMap<&str, &str> = remote_urls
                .iter()
                .map(|(key, url)| (url.as_str(), key.as_str()))
                .collect();
            for line in output.stdout.split(|byte| *byte == b'\n') {
                let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
                    continue;
                };
                let Some(id) = value.get("id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(duration) = value.get("duration").and_then(|v| v.as_f64()) else {
                    continue;
                };
                if duration <= 0.0 {
                    continue;
                }
                let webpage_url = value
                    .get("webpage_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let key = requested_keys.get(webpage_url).copied().unwrap_or(id);
                durations.insert(key.to_string(), duration);
            }
        }
    }

    let _ = tx.send(WorkerMsg::DurationsLoaded(durations));
}

fn probe_audio_duration(path: &Path) -> Option<f64> {
    let output = Command::new("ffprobe")
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let duration = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()?;
    (duration > 0.0).then_some(duration)
}

/// yt-dlp normally returns a playlist wrapper for `ytsearch`, but direct video
/// URLs and some extractor responses return one video object instead. Parse
/// both shapes so a valid single result isn't reported as an empty playlist.
fn parse_youtube_search_results(data: &[u8]) -> Result<Vec<PlaylistEntry>, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_slice(data)?;
    let mut entries = if let Some(raw_entries) = value.get("entries").and_then(|v| v.as_array()) {
        raw_entries
            .iter()
            .filter_map(|entry| serde_json::from_value::<PlaylistEntry>(entry.clone()).ok())
            .collect()
    } else if value.get("entries").is_none() {
        serde_json::from_value::<PlaylistEntry>(value)
            .ok()
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };

    entries.retain(|entry| entry.watch_url().is_some());
    Ok(entries)
}
