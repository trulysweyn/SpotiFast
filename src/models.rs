use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PlaylistEntry {
    pub id: Option<String>,
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub webpage_url: Option<String>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub uploader: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(skip)]
    pub title_lower: String,
    #[serde(skip)]
    pub artist_lower: String,
}

impl PlaylistEntry {
    pub fn ensure_lower(&mut self) {
        if self.title_lower.is_empty() {
            self.title_lower = self.display_title().to_lowercase();
        }
        if self.artist_lower.is_empty() {
            self.artist_lower = self.display_artist().to_lowercase();
        }
    }

    pub fn display_title(&self) -> String {
        if let Some(t) = &self.title {
            let trimmed = t.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        "Unknown Track".to_string()
    }

    pub fn display_artist(&self) -> String {
        if let Some(a) = &self.artist {
            let trimmed = a.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        if let Some(c) = &self.channel {
            let trimmed = c.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        if let Some(u) = &self.uploader {
            let trimmed = u.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
        if let Some(t) = &self.title {
            if let Some((artist_part, _)) = t.split_once(" - ") {
                let trimmed = artist_part.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
        "YouTube".to_string()
    }

    pub fn formatted_duration(&self) -> String {
        match self.duration {
            Some(d) if d > 0.0 => {
                let total_secs = d.round() as u64;
                let mins = total_secs / 60;
                let secs = total_secs % 60;
                format!("{mins}:{secs:02}")
            }
            _ => "--:--".to_string(),
        }
    }

    pub fn watch_url(&self) -> Option<String> {
        if let Some(u) = &self.webpage_url {
            if is_youtube_video_url(u) {
                return Some(u.clone());
            }
        }
        if let Some(u) = &self.url {
            if is_youtube_video_url(u) {
                return Some(u.clone());
            }
            if is_video_id(u) {
                return Some(format!("https://www.youtube.com/watch?v={}", u));
            }
        }
        self.id
            .as_ref()
            .filter(|id| is_video_id(id))
            .map(|id| format!("https://www.youtube.com/watch?v={}", id))
    }

    pub fn cache_key(&self) -> Option<String> {
        if let Some(id) = &self.id {
            let clean: String = id
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !clean.is_empty() {
                return Some(clean);
            }
        }
        let raw = self.watch_url()?;
        let clean: String = raw
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if clean.is_empty() {
            None
        } else {
            Some(
                clean
                    .chars()
                    .rev()
                    .take(24)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect(),
            )
        }
    }

    pub fn matches_track(&self, other: &PlaylistEntry) -> bool {
        if let (Some(a), Some(b)) = (&self.id, &other.id) {
            if !a.is_empty() && a == b {
                return true;
            }
        }
        match (self.watch_url(), other.watch_url()) {
            (Some(u1), Some(u2)) => u1 == u2,
            _ => false,
        }
    }
}

fn is_video_id(value: &str) -> bool {
    value.len() == 11
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn is_youtube_video_url(value: &str) -> bool {
    let Some((scheme, rest)) = value.split_once("://") else {
        return false;
    };
    if scheme != "https" && scheme != "http" {
        return false;
    }

    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..host_end].to_ascii_lowercase();
    let path_and_query = &rest[host_end..];
    let (path, query) = path_and_query
        .split_once('?')
        .unwrap_or((path_and_query, ""));
    let video_id = if host == "youtu.be" || host == "www.youtu.be" {
        path.trim_start_matches('/').split('/').next().unwrap_or("")
    } else if host == "youtube.com"
        || host == "www.youtube.com"
        || host == "m.youtube.com"
        || host == "music.youtube.com"
    {
        if path == "/watch" {
            query
                .split('&')
                .find_map(|pair| {
                    let (key, value) = pair.split_once('=')?;
                    (key == "v").then_some(value)
                })
                .unwrap_or("")
        } else {
            let mut parts = path.trim_start_matches('/').split('/');
            match (parts.next(), parts.next()) {
                (Some("shorts" | "embed" | "live"), Some(id)) => id,
                _ => "",
            }
        }
    } else {
        return false;
    };

    is_video_id(video_id)
}

#[derive(Deserialize, Debug)]
pub struct PlaylistDump {
    pub title: Option<String>,
    pub entries: Option<Vec<Option<PlaylistEntry>>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SavedPlaylist {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub tracks: Vec<PlaylistEntry>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct PersistedState {
    pub last_url: String,
    pub playlist_title: String,
    pub tracks: Vec<PlaylistEntry>,
    #[serde(default)]
    pub liked_tracks: Vec<PlaylistEntry>,
    pub last_track_index: Option<usize>,
    #[serde(default)]
    pub last_played_track: Option<PlaylistEntry>,
    #[serde(default)]
    pub last_track_position_secs: f64,
    #[serde(default)]
    pub resume_playback: bool,
    pub shuffle: bool,
    pub auto_advance: bool,
    pub loop_playlist: bool,
    pub volume: f32,
    #[serde(default)]
    pub last_unmuted_volume: f32,
    pub saved_playlists: Vec<SavedPlaylist>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct AppSettings {
    pub run_at_startup: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            run_at_startup: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActiveView {
    Search,
    Library,
    Liked,
    Downloaded,
    Queue,
    Playlists,
    Settings,
}

#[derive(Clone, Debug)]
pub struct CacheInfo {
    pub key: String,
    #[allow(dead_code)]
    pub path: PathBuf,
    pub file_size_bytes: u64,
    pub title: String,
    pub artist: String,
    pub duration: Option<f64>,
}
