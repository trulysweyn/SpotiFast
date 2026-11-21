#![windows_subsystem = "windows"]

mod backend;
mod models;
mod ui;
mod updater;

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, vec2, Align, Align2, CentralPanel, Color32, CornerRadius, FontId, Layout, Margin, Panel,
    Pos2, Rect, RichText, Sense, Stroke,
};
use rand::seq::SliceRandom;

use backend::{
    clear_audio_cache, cycle_mpv_pause, find_cached_audio, load_persisted_state,
    prepare_track_file, run_duration_lookup, run_playlist_fetch, run_youtube_search,
    save_persisted_state, save_settings, scan_cache_info, seek_mpv, set_mpv_volume,
    set_run_at_startup, spawn_mpv_target, ProcessGuard, WorkerMsg,
};
use models::{ActiveView, AppSettings, CacheInfo, PersistedState, PlaylistEntry, SavedPlaylist};
use ui::{
    apply_spotify_theme, brand_header, circle_play_button, draw_app_icon, draw_player_heart,
    draw_speaker_icon, format_bytes, format_time, icon_button, seek_scrubber, sidebar_nav_item,
    spotify_icon_pill_button, spotify_pill_button, track_bold_font, transport_button,
    volume_slider, AppIcon, SpotifyPalette,
};
use updater::{AppUpdateInfo, ToolKind, ToolStatus};

const VOLUME_IPC_DEBOUNCE_MS: u64 = 50;

struct PlayerApp {
    palette: SpotifyPalette,
    active_view: ActiveView,

    // Active Playlist / Library
    playlist_title: String,
    url_input: String,
    playlist_name_input: String,
    tracks: Vec<PlaylistEntry>,
    play_order: Vec<usize>,
    queue_pos: Option<usize>,
    last_track_index: Option<usize>,

    // Search View State
    search_input: String,
    search_results: Vec<PlaylistEntry>,
    search_in_flight: bool,
    search_status: String,

    // Liked and Queue
    liked_tracks: Vec<PlaylistEntry>,
    user_queue: Vec<PlaylistEntry>,

    // Filter in lists
    list_filter: String,

    // Playback Engine
    procs: ProcessGuard,
    active_player_id: Option<usize>,
    active_track: Option<PlaylistEntry>,
    paused: bool,
    volume: f32,
    last_volume_ipc: Option<Instant>,
    pending_volume_ipc: bool,

    // Time Tracking & Seeking
    playback_start: Instant,
    playback_offset_secs: f64,
    current_track_duration: f64,
    playback_request_id: u64,

    // Toggles
    shuffle: bool,
    auto_advance: bool,
    loop_playlist: bool,

    // Cache State
    cached_keys: HashSet<String>,
    cached_items: Vec<CacheInfo>,
    total_cache_bytes: u64,
    prefetching_keys: HashSet<String>,
    cache_downloading_keys: HashSet<String>,
    download_lock: Arc<Mutex<()>>,

    // Saved Playlists
    saved_playlists: Vec<SavedPlaylist>,
    selected_saved_idx: Option<usize>,

    // Settings and managed player tools
    settings: AppSettings,
    tool_statuses: Vec<ToolStatus>,
    settings_busy: bool,
    settings_message: String,
    app_update_repository_input: String,
    app_update_info: Option<AppUpdateInfo>,
    app_update_busy: bool,
    app_update_message: String,
    exit_for_app_update: bool,

    // Status & Threading
    status: String,
    in_flight_jobs: usize,
    tx: Sender<WorkerMsg>,
    rx: Receiver<WorkerMsg>,
}

impl PlayerApp {
    fn new(ctx: &egui::Context) -> Self {
        let palette = SpotifyPalette::default();
        apply_spotify_theme(ctx, &palette);

        let (tx, rx) = mpsc::channel();
        let persisted = load_persisted_state();
        let settings = backend::load_settings();
        let app_update_repository_input = settings.update_repository.clone();
        let resume_track = persisted.last_played_track.clone().or_else(|| {
            persisted
                .last_track_index
                .and_then(|index| persisted.tracks.get(index).cloned())
        });
        let resume_position_secs = persisted.last_track_position_secs.max(0.0);
        let should_resume_playback = persisted.resume_playback;

        let mut all_known = persisted.tracks.clone();
        for liked in &persisted.liked_tracks {
            if !all_known.iter().any(|t| t.matches_track(liked)) {
                all_known.push(liked.clone());
            }
        }
        let (cached_keys, cached_items, total_cache_bytes) = scan_cache_info(&all_known);

        let mut app = Self {
            palette,
            active_view: ActiveView::Search,
            playlist_title: persisted.playlist_title,
            url_input: persisted.last_url.clone(),
            playlist_name_input: String::new(),
            tracks: persisted.tracks,
            play_order: Vec::new(),
            queue_pos: None,
            last_track_index: persisted.last_track_index,

            search_input: String::new(),
            search_results: Vec::new(),
            search_in_flight: false,
            search_status: "Search for any song or artist on YouTube...".to_string(),

            liked_tracks: persisted.liked_tracks,
            user_queue: Vec::new(),
            list_filter: String::new(),

            procs: ProcessGuard::new(),
            active_player_id: None,
            active_track: None,
            paused: false,
            volume: persisted.volume,
            last_volume_ipc: None,
            pending_volume_ipc: false,

            playback_start: Instant::now(),
            playback_offset_secs: 0.0,
            current_track_duration: 0.0,
            playback_request_id: 0,

            shuffle: persisted.shuffle,
            auto_advance: persisted.auto_advance,
            loop_playlist: persisted.loop_playlist,

            cached_keys,
            cached_items,
            total_cache_bytes,
            prefetching_keys: HashSet::new(),
            cache_downloading_keys: HashSet::new(),
            download_lock: Arc::new(Mutex::new(())),

            saved_playlists: persisted.saved_playlists,
            selected_saved_idx: None,

            settings,
            tool_statuses: [ToolKind::YtDlp, ToolKind::Mpv]
                .into_iter()
                .map(|kind| ToolStatus {
                    kind,
                    current_version: "Not checked".to_string(),
                    latest_version: "Not checked".to_string(),
                    update_available: false,
                    rollback_available: updater::rollback_available(kind),
                })
                .collect(),
            settings_busy: false,
            settings_message: "Check for updates to see the latest player tools.".to_string(),
            app_update_repository_input,
            app_update_info: None,
            app_update_busy: false,
            app_update_message: "Set a GitHub repository to check for SpotiFast releases."
                .to_string(),
            exit_for_app_update: false,

            status: "Ready. Search for songs or choose from your Library.".to_string(),
            in_flight_jobs: 0,
            tx,
            rx,
        };

        app.rebuild_order(app.last_track_index);
        if let Some(track_idx) = app.last_track_index {
            if track_idx < app.tracks.len() {
                app.queue_pos = app.play_order.iter().position(|&i| i == track_idx);
                app.active_track = Some(app.tracks[track_idx].clone());
                app.current_track_duration = app.tracks[track_idx].duration.unwrap_or(0.0);
            }
        }

        app.start_duration_lookup();
        if should_resume_playback {
            if let Some(track) = resume_track {
                app.active_track = Some(track.clone());
                app.current_track_duration = track.duration.unwrap_or(0.0);
                app.playback_offset_secs = resume_position_secs;
                app.status = format!("Resuming: {}", track.display_title());
                app.play_entry_at(track, resume_position_secs);
            }
        }
        app
    }

    fn all_known_entries(&self) -> Vec<PlaylistEntry> {
        let mut entries = self.tracks.clone();
        for item in &self.liked_tracks {
            if !entries.iter().any(|t| t.matches_track(item)) {
                entries.push(item.clone());
            }
        }
        for item in &self.search_results {
            if !entries.iter().any(|t| t.matches_track(item)) {
                entries.push(item.clone());
            }
        }
        entries
    }

    fn set_startup_preference(&mut self, enabled: bool) {
        match set_run_at_startup(enabled) {
            Ok(()) => {
                self.settings.run_at_startup = enabled;
                save_settings(&self.settings);
                self.settings_message = if enabled {
                    "SpotiFast will open when you sign in to Windows.".to_string()
                } else {
                    "SpotiFast will no longer open automatically.".to_string()
                };
            }
            Err(error) => self.settings_message = error,
        }
    }

    fn check_tool_updates(&mut self) {
        if self.settings_busy {
            return;
        }
        self.settings_busy = true;
        self.settings_message = "Checking yt-dlp and mpv for updates…".to_string();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let _ = tx.send(WorkerMsg::UpdateCheckFinished(updater::check_for_updates()));
        });
    }

    fn install_tool_update(&mut self, kind: ToolKind) {
        if self.settings_busy {
            return;
        }
        if kind == ToolKind::Mpv {
            self.save_state();
            self.stop_active_player(true);
            self.save_state();
        }
        self.settings_busy = true;
        self.settings_message = format!("Downloading and preparing {}…", kind.label());
        let tx = self.tx.clone();
        thread::spawn(move || {
            let _ = tx.send(WorkerMsg::UpdateInstalled(updater::install_update(kind)));
        });
    }

    fn rollback_tool(&mut self, kind: ToolKind) {
        if self.settings_busy {
            return;
        }
        if kind == ToolKind::Mpv {
            self.save_state();
            self.stop_active_player(true);
            self.save_state();
        }
        self.settings_busy = true;
        self.settings_message = format!("Restoring the previous {} version…", kind.label());
        let tx = self.tx.clone();
        thread::spawn(move || {
            let _ = tx.send(WorkerMsg::RollbackFinished(updater::rollback(kind)));
        });
    }

    fn save_app_update_repository(&mut self) {
        let repository = self.app_update_repository_input.trim().to_string();
        self.settings.update_repository = repository.clone();
        self.app_update_repository_input = repository;
        save_settings(&self.settings);
        self.app_update_info = None;
        self.app_update_message =
            "Release source saved. Check for a new SpotiFast version.".to_string();
    }

    fn check_app_updates(&mut self) {
        if self.app_update_busy {
            return;
        }
        let repository = self.settings.update_repository.trim().to_string();
        if repository.is_empty() {
            self.app_update_message = "Enter and save your GitHub repository first.".to_string();
            return;
        }
        self.app_update_busy = true;
        self.app_update_message = "Checking the latest SpotiFast release…".to_string();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let _ = tx.send(WorkerMsg::AppUpdateCheckFinished(
                updater::check_app_updates(&repository),
            ));
        });
    }

    fn download_app_update(&mut self) {
        if self.app_update_busy {
            return;
        }
        let repository = self.settings.update_repository.trim().to_string();
        self.app_update_busy = true;
        self.app_update_message = "Downloading and validating the SpotiFast update…".to_string();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let _ = tx.send(WorkerMsg::AppUpdateStaged(updater::stage_app_update(
                &repository,
            )));
        });
    }

    fn rollback_app_update(&mut self) {
        if self.app_update_busy {
            return;
        }
        self.app_update_busy = true;
        self.app_update_message = "Preparing the previous SpotiFast version…".to_string();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let _ = tx.send(WorkerMsg::AppUpdateStaged(updater::stage_app_rollback()));
        });
    }

    fn set_tool_status(&mut self, status: ToolStatus) {
        if let Some(existing) = self
            .tool_statuses
            .iter_mut()
            .find(|existing| existing.kind == status.kind)
        {
            *existing = status;
        }
    }

    fn refresh_cache_state(&mut self) {
        let known = self.all_known_entries();
        let (keys, items, bytes) = scan_cache_info(&known);
        self.cached_keys = keys;
        self.cached_items = items;
        self.total_cache_bytes = bytes;
    }

    fn start_duration_lookup(&mut self) {
        let mut entries = self.tracks.clone();
        for entry in self.liked_tracks.iter().chain(
            self.saved_playlists
                .iter()
                .flat_map(|playlist| playlist.tracks.iter()),
        ) {
            if !entries.iter().any(|known| known.matches_track(entry)) {
                entries.push(entry.clone());
            }
        }
        entries.retain(|entry| {
            entry
                .duration
                .map(|duration| duration <= 0.0)
                .unwrap_or(true)
        });
        if entries.is_empty() {
            return;
        }

        self.in_flight_jobs += 1;
        let tx = self.tx.clone();
        thread::spawn(move || run_duration_lookup(entries, tx));
    }

    fn apply_track_durations(&mut self, durations: &HashMap<String, f64>) {
        fn update(entries: &mut [PlaylistEntry], durations: &HashMap<String, f64>) -> bool {
            let mut changed = false;
            for entry in entries {
                if entry
                    .duration
                    .map(|duration| duration > 0.0)
                    .unwrap_or(false)
                {
                    continue;
                }
                if let Some(duration) = entry
                    .cache_key()
                    .and_then(|key| durations.get(&key).copied())
                {
                    entry.duration = Some(duration);
                    changed = true;
                }
            }
            changed
        }

        let mut changed = update(&mut self.tracks, durations)
            | update(&mut self.liked_tracks, durations)
            | update(&mut self.search_results, durations)
            | update(&mut self.user_queue, durations);
        for playlist in &mut self.saved_playlists {
            changed |= update(&mut playlist.tracks, durations);
        }
        if let Some(track) = &mut self.active_track {
            changed |= update(std::slice::from_mut(track), durations);
            if let Some(duration) = track.duration {
                self.current_track_duration = duration;
            }
        }
        if changed {
            self.save_state();
            self.refresh_cache_state();
        }
    }

    fn save_state(&self) {
        let current_track = self
            .queue_pos
            .and_then(|p| self.play_order.get(p).copied())
            .or(self.last_track_index);
        let last_played_track = self
            .active_track
            .clone()
            .or_else(|| current_track.and_then(|index| self.tracks.get(index).cloned()));
        let last_track_position_secs = if self.active_player_id.is_some() {
            self.current_playback_position()
        } else {
            self.playback_offset_secs
        };
        let state = PersistedState {
            last_url: self.url_input.trim().to_string(),
            playlist_title: self.playlist_title.clone(),
            tracks: self.tracks.clone(),
            liked_tracks: self.liked_tracks.clone(),
            last_track_index: current_track,
            last_played_track,
            last_track_position_secs,
            resume_playback: self.active_player_id.is_some() && !self.paused,
            shuffle: self.shuffle,
            auto_advance: self.auto_advance,
            loop_playlist: self.loop_playlist,
            volume: self.volume,
            saved_playlists: self.saved_playlists.clone(),
        };
        save_persisted_state(&state);
    }

    fn rebuild_order(&mut self, keep_current_track: Option<usize>) {
        self.play_order = (0..self.tracks.len()).collect();
        if self.shuffle && !self.play_order.is_empty() {
            let mut rng = rand::rng();
            self.play_order.shuffle(&mut rng);
            if let Some(track_idx) = keep_current_track {
                if let Some(pos) = self.play_order.iter().position(|&i| i == track_idx) {
                    self.play_order.swap(0, pos);
                    self.queue_pos = Some(0);
                }
            }
        } else if let Some(track_idx) = keep_current_track {
            self.queue_pos = self.play_order.iter().position(|&i| i == track_idx);
        }
    }

    fn stop_active_player(&mut self, save_position: bool) -> u64 {
        self.playback_request_id = self.playback_request_id.wrapping_add(1);
        self.paused = false;
        let position = if save_position {
            if self.active_player_id.is_some() {
                self.current_playback_position()
            } else {
                self.playback_offset_secs
            }
        } else {
            0.0
        };
        if let Some(id) = self.active_player_id.take() {
            self.procs.kill_and_remove(id, save_position);
        }
        self.playback_offset_secs = position;
        self.playback_request_id
    }

    fn play_entry(&mut self, entry: PlaylistEntry) {
        self.play_entry_at(entry, 0.0);
    }

    fn play_entry_at(&mut self, entry: PlaylistEntry, start_position_secs: f64) {
        let Some(watch_url) = entry.watch_url() else {
            self.status = "Track has no valid URL.".to_string();
            return;
        };

        let title = entry.display_title();
        let cache_key = entry.cache_key();
        let is_cached = cache_key
            .as_deref()
            .map(|k| self.cached_keys.contains(k))
            .unwrap_or(false);

        let request_id = self.stop_active_player(false);
        self.active_track = Some(entry.clone());
        self.current_track_duration = entry.duration.unwrap_or(0.0);
        self.playback_offset_secs = start_position_secs.max(0.0);

        if is_cached {
            if let Some(ref key) = cache_key {
                if let Some(target) = find_cached_audio(key) {
                    match spawn_mpv_target(&target, self.volume, self.playback_offset_secs) {
                        Ok(child) => {
                            let id = self.procs.track(child);
                            self.active_player_id = Some(id);
                            self.playback_start = Instant::now();
                            self.paused = false;
                            self.status = format!("Playing (Cached): {title}");
                            self.maybe_prefetch_next();
                            return;
                        }
                        Err(e) => {
                            self.status = format!("Failed to launch mpv: {e}");
                            return;
                        }
                    }
                }
            }
        }

        self.status = format!("Downloading & caching: {title}...");
        self.in_flight_jobs += 1;
        let tx = self.tx.clone();
        let download_lock = self.download_lock.clone();

        thread::spawn(move || {
            match prepare_track_file(&watch_url, cache_key.as_deref(), &download_lock) {
                Ok(target) => {
                    let _ = tx.send(WorkerMsg::PlaybackReady {
                        request_id,
                        entry,
                        target,
                        start_position_secs,
                    });
                }
                Err(error) => {
                    let _ = tx.send(WorkerMsg::PlaybackError { request_id, error });
                }
            }
        });
    }

    fn play_queue_pos(&mut self, pos: usize) {
        if pos >= self.play_order.len() {
            return;
        }
        let track_idx = self.play_order[pos];
        self.queue_pos = Some(pos);
        self.last_track_index = Some(track_idx);
        self.save_state();

        let entry = self.tracks[track_idx].clone();
        self.play_entry(entry);
    }

    fn play_next(&mut self, from_auto: bool) {
        // If there are user queued songs, play them first!
        if !self.user_queue.is_empty() {
            let next_track = self.user_queue.remove(0);
            self.play_entry(next_track);
            return;
        }

        if self.play_order.is_empty() {
            return;
        }

        let next_pos = match self.queue_pos {
            Some(p) if p + 1 < self.play_order.len() => Some(p + 1),
            Some(_) if self.loop_playlist || !from_auto => Some(0),
            Some(_) => None,
            None => Some(0),
        };

        match next_pos {
            Some(pos) => self.play_queue_pos(pos),
            None => {
                self.queue_pos = None;
                self.paused = false;
                self.status = "Finished playlist.".to_string();
            }
        }
    }

    fn play_prev(&mut self) {
        if self.play_order.is_empty() {
            return;
        }
        let prev_pos = match self.queue_pos {
            Some(0) | None => self.play_order.len() - 1,
            Some(p) => p - 1,
        };
        self.play_queue_pos(prev_pos);
    }

    fn toggle_pause_resume(&mut self) {
        if self.active_player_id.is_none() {
            if let Some(track) = self.active_track.clone() {
                self.play_entry(track);
            } else if let Some(pos) = self.queue_pos {
                self.play_queue_pos(pos);
            } else if !self.play_order.is_empty() {
                self.play_queue_pos(0);
            }
            return;
        }

        match cycle_mpv_pause() {
            Ok(()) => {
                if self.paused {
                    // Resuming
                    self.playback_start = Instant::now();
                    self.paused = false;
                    let title = self
                        .active_track
                        .as_ref()
                        .map(|t| t.display_title())
                        .unwrap_or_else(|| "Track".to_string());
                    self.status = format!("Playing: {title}");
                } else {
                    // Pausing
                    self.playback_offset_secs += self.playback_start.elapsed().as_secs_f64();
                    self.paused = true;
                    let title = self
                        .active_track
                        .as_ref()
                        .map(|t| t.display_title())
                        .unwrap_or_else(|| "Track".to_string());
                    self.status = format!("Paused: {title}");
                }
            }
            Err(e) => {
                self.status = format!("Pause error: {e}");
            }
        }
    }

    fn seek_to(&mut self, target_secs: f64) {
        if self.active_player_id.is_none() {
            return;
        }
        let _ = seek_mpv(target_secs);
        self.playback_offset_secs = target_secs;
        self.playback_start = Instant::now();
    }

    fn current_playback_position(&self) -> f64 {
        if self.active_player_id.is_none() {
            return 0.0;
        }
        let pos = if self.paused {
            self.playback_offset_secs
        } else {
            self.playback_offset_secs + self.playback_start.elapsed().as_secs_f64()
        };
        if self.current_track_duration > 0.0 {
            pos.min(self.current_track_duration)
        } else {
            pos
        }
    }

    fn on_volume_changed(&mut self) {
        self.save_state();
        if self.active_player_id.is_none() {
            return;
        }
        let now = Instant::now();
        let ready = self
            .last_volume_ipc
            .map(|t| now.duration_since(t) >= Duration::from_millis(VOLUME_IPC_DEBOUNCE_MS))
            .unwrap_or(true);
        if ready {
            self.flush_volume_ipc_now();
        } else {
            self.pending_volume_ipc = true;
        }
    }

    fn flush_volume_ipc_now(&mut self) {
        self.pending_volume_ipc = false;
        self.last_volume_ipc = Some(Instant::now());
        if self.active_player_id.is_some() {
            let _ = set_mpv_volume(self.volume);
        }
    }

    fn maybe_prefetch_next(&mut self) {
        if !self.auto_advance {
            return;
        }
        // If there is an item in user queue, prefetch that
        let next_entry = if let Some(first_queued) = self.user_queue.first() {
            Some(first_queued.clone())
        } else if let Some(current_pos) = self.queue_pos {
            let next_pos = if current_pos + 1 < self.play_order.len() {
                Some(current_pos + 1)
            } else if self.loop_playlist {
                Some(0)
            } else {
                None
            };
            next_pos
                .and_then(|p| self.play_order.get(p).copied())
                .and_then(|idx| self.tracks.get(idx).cloned())
        } else {
            None
        };

        let Some(entry) = next_entry else { return };
        let Some(key) = entry.cache_key() else { return };
        if self.cached_keys.contains(&key) || self.prefetching_keys.contains(&key) {
            return;
        }
        let Some(watch_url) = entry.watch_url() else {
            return;
        };

        self.prefetching_keys.insert(key.clone());
        self.in_flight_jobs += 1;
        let tx = self.tx.clone();
        let download_lock = self.download_lock.clone();

        thread::spawn(move || {
            let _guard = download_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if find_cached_audio(&key).is_none() {
                let _ = backend::download_track_file(&watch_url, &key);
            }
            let _ = tx.send(WorkerMsg::PrefetchFinished(key));
        });
    }

    fn download_to_cache(&mut self, entry: &PlaylistEntry) {
        let Some(key) = entry.cache_key() else { return };
        if self.cached_keys.contains(&key) || self.cache_downloading_keys.contains(&key) {
            return;
        }
        let Some(watch_url) = entry.watch_url() else {
            return;
        };

        self.cache_downloading_keys.insert(key.clone());
        self.in_flight_jobs += 1;
        let tx = self.tx.clone();
        let download_lock = self.download_lock.clone();

        thread::spawn(move || {
            let _guard = download_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let success = if find_cached_audio(&key).is_some() {
                true
            } else {
                backend::download_track_file(&watch_url, &key).is_ok()
            };
            let _ = tx.send(WorkerMsg::CacheDownloadFinished { key, success });
        });
    }

    fn is_liked(&self, entry: &PlaylistEntry) -> bool {
        self.liked_tracks.iter().any(|t| t.matches_track(entry))
    }

    fn toggle_like(&mut self, mut entry: PlaylistEntry) {
        entry.ensure_lower();
        if let Some(pos) = self
            .liked_tracks
            .iter()
            .position(|t| t.matches_track(&entry))
        {
            self.liked_tracks.remove(pos);
            self.status = format!("Removed from Liked Songs: {}", entry.display_title());
        } else {
            self.liked_tracks.push(entry.clone());
            self.status = format!("Added to Liked Songs: {}", entry.display_title());
        }
        self.save_state();
    }

    fn add_track_to_saved_playlist(&mut self, playlist_idx: usize, entry: PlaylistEntry) {
        let current_track = self
            .queue_pos
            .and_then(|position| self.play_order.get(position).copied())
            .and_then(|track_idx| self.tracks.get(track_idx))
            .cloned();
        let Some(playlist) = self.saved_playlists.get_mut(playlist_idx) else {
            return;
        };
        if playlist.tracks.is_empty() && playlist.url == self.url_input.trim() {
            playlist.tracks = self.tracks.clone();
        }
        let was_added = if playlist
            .tracks
            .iter()
            .any(|track| track.matches_track(&entry))
        {
            false
        } else {
            playlist.tracks.push(entry.clone());
            true
        };
        let playlist_name = playlist.name.clone();
        let playlist_url = playlist.url.clone();
        let updated_tracks = playlist.tracks.clone();

        // The library shows `self.tracks`, while each saved playlist stores its own tracks.
        // Select and load the destination immediately so additions from Search are visible
        // the next time the user opens Your Library, even if no playlist was selected before.
        self.selected_saved_idx = Some(playlist_idx);
        self.playlist_name_input = playlist_name.clone();
        self.playlist_title = playlist_name.clone();
        self.url_input = playlist_url;
        self.tracks = updated_tracks;
        let current_track_idx = current_track.as_ref().and_then(|current| {
            self.tracks
                .iter()
                .position(|track| track.matches_track(current))
        });
        self.rebuild_order(current_track_idx);
        self.queue_pos = current_track_idx
            .and_then(|track_idx| self.play_order.iter().position(|&idx| idx == track_idx));
        self.last_track_index = current_track_idx;

        self.status = if was_added {
            format!("Added {} to {}.", entry.display_title(), playlist_name)
        } else {
            format!("{} is already in {}.", entry.display_title(), playlist_name)
        };
        self.save_state();
    }

    fn open_saved_playlist(&mut self, playlist_idx: usize) {
        let Some(playlist) = self.saved_playlists.get(playlist_idx).cloned() else {
            return;
        };
        self.selected_saved_idx = Some(playlist_idx);
        self.playlist_name_input = playlist.name.clone();
        self.url_input = playlist.url.clone();
        self.active_view = ActiveView::Library;

        if playlist.tracks.is_empty() {
            self.load_playlist_url();
            return;
        }

        self.playlist_title = playlist.name;
        self.tracks = playlist.tracks;
        self.rebuild_order(None);
        self.queue_pos = (!self.tracks.is_empty()).then_some(0);
        self.last_track_index = self.queue_pos;
        self.status = format!("Loaded {} tracks from playlist.", self.tracks.len());
        self.refresh_cache_state();
        self.save_state();
    }

    fn trigger_search(&mut self) {
        let q = self.search_input.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.search_in_flight = true;
        self.search_status = format!("Searching YouTube for '{q}'...");
        self.in_flight_jobs += 1;
        let tx = self.tx.clone();

        thread::spawn(move || {
            run_youtube_search(q, tx);
        });
    }

    fn load_playlist_url(&mut self) {
        let url = self.url_input.trim().to_string();
        if url.is_empty() {
            self.status = "Please enter a valid YouTube playlist URL.".to_string();
            return;
        }
        self.status = "Loading playlist from YouTube...".to_string();
        self.in_flight_jobs += 1;
        let tx = self.tx.clone();

        thread::spawn(move || {
            run_playlist_fetch(url, tx);
        });
    }

    fn poll_background(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                WorkerMsg::SearchResultsLoaded { query, results } => {
                    self.in_flight_jobs = self.in_flight_jobs.saturating_sub(1);
                    self.search_in_flight = false;
                    self.search_status = format!("Found {} results for '{}'", results.len(), query);
                    self.search_results = results;
                    self.refresh_cache_state();
                }
                WorkerMsg::SearchError(err) => {
                    self.in_flight_jobs = self.in_flight_jobs.saturating_sub(1);
                    self.search_in_flight = false;
                    self.search_status = err;
                }
                WorkerMsg::DurationsLoaded(durations) => {
                    self.in_flight_jobs = self.in_flight_jobs.saturating_sub(1);
                    self.apply_track_durations(&durations);
                }
                WorkerMsg::PlaylistLoaded { url, title, tracks } => {
                    self.in_flight_jobs = self.in_flight_jobs.saturating_sub(1);
                    self.url_input = url.clone();
                    self.playlist_title = title;
                    self.tracks = tracks;
                    if let Some(index) = self.selected_saved_idx {
                        if let Some(playlist) = self.saved_playlists.get_mut(index) {
                            if playlist.url == url {
                                playlist.tracks = self.tracks.clone();
                            }
                        }
                    }
                    self.rebuild_order(None);
                    self.queue_pos = (!self.tracks.is_empty()).then_some(0);
                    self.last_track_index = self.queue_pos;
                    self.save_state();
                    self.start_duration_lookup();
                    self.status = format!("Loaded {} tracks.", self.tracks.len());
                    self.refresh_cache_state();
                }
                WorkerMsg::PlaylistError(err) => {
                    self.in_flight_jobs = self.in_flight_jobs.saturating_sub(1);
                    self.status = err;
                }
                WorkerMsg::PlaybackReady {
                    request_id,
                    entry,
                    target,
                    start_position_secs,
                } => {
                    self.in_flight_jobs = self.in_flight_jobs.saturating_sub(1);
                    if let Some(key) = entry.cache_key() {
                        self.cached_keys.insert(key.clone());
                        self.prefetching_keys.remove(&key);
                    }
                    if request_id != self.playback_request_id {
                        continue;
                    }

                    match spawn_mpv_target(&target, self.volume, start_position_secs) {
                        Ok(child) => {
                            let id = self.procs.track(child);
                            self.active_player_id = Some(id);
                            self.active_track = Some(entry.clone());
                            self.current_track_duration = entry.duration.unwrap_or(0.0);
                            self.playback_start = Instant::now();
                            self.playback_offset_secs = start_position_secs.max(0.0);
                            self.paused = false;
                            self.status = format!("Playing: {}", entry.display_title());
                            self.refresh_cache_state();
                            self.maybe_prefetch_next();
                        }
                        Err(e) => {
                            self.status = format!("Failed to launch mpv: {e}");
                        }
                    }
                }
                WorkerMsg::PlaybackError { request_id, error } => {
                    self.in_flight_jobs = self.in_flight_jobs.saturating_sub(1);
                    if request_id == self.playback_request_id {
                        self.status = error;
                    }
                }
                WorkerMsg::PrefetchFinished(key) => {
                    self.in_flight_jobs = self.in_flight_jobs.saturating_sub(1);
                    self.prefetching_keys.remove(&key);
                    if find_cached_audio(&key).is_some() {
                        self.cached_keys.insert(key);
                        self.refresh_cache_state();
                    }
                }
                WorkerMsg::CacheDownloadFinished { key, success } => {
                    self.in_flight_jobs = self.in_flight_jobs.saturating_sub(1);
                    self.cache_downloading_keys.remove(&key);
                    if success {
                        self.cached_keys.insert(key);
                        self.refresh_cache_state();
                        self.status = "Cached track locally.".to_string();
                    } else {
                        self.status = "Download failed.".to_string();
                    }
                }
                WorkerMsg::UpdateCheckFinished(result) => {
                    self.settings_busy = false;
                    match result {
                        Ok(statuses) => {
                            let available = statuses
                                .iter()
                                .filter(|status| status.update_available)
                                .count();
                            for status in statuses {
                                self.set_tool_status(status);
                            }
                            self.settings_message = if available == 0 {
                                "Both player tools are up to date.".to_string()
                            } else {
                                format!("{available} player tool update(s) are available.")
                            };
                        }
                        Err(error) => self.settings_message = error,
                    }
                }
                WorkerMsg::UpdateInstalled(result) => {
                    self.settings_busy = false;
                    match result {
                        Ok(status) => {
                            self.settings_message = format!(
                                "Updated {}. Your library and downloaded songs were kept.",
                                status.kind.label()
                            );
                            self.set_tool_status(status);
                        }
                        Err(error) => self.settings_message = error,
                    }
                }
                WorkerMsg::RollbackFinished(result) => {
                    self.settings_busy = false;
                    match result {
                        Ok(status) => {
                            self.settings_message = format!(
                                "Rolled {} back to the saved version.",
                                status.kind.label()
                            );
                            self.set_tool_status(status);
                        }
                        Err(error) => self.settings_message = error,
                    }
                }
                WorkerMsg::AppUpdateCheckFinished(result) => {
                    self.app_update_busy = false;
                    match result {
                        Ok(info) => {
                            self.app_update_message = if info.update_available {
                                format!(
                                    "SpotiFast {} is available (you have {} from {}).",
                                    info.latest_version, info.current_version, info.repository
                                )
                            } else {
                                format!("SpotiFast {} is up to date.", info.current_version)
                            };
                            self.app_update_info = Some(info);
                        }
                        Err(error) => self.app_update_message = error,
                    }
                }
                WorkerMsg::AppUpdateStaged(result) => match result {
                    Ok(staged) => match updater::launch_app_update(&staged) {
                        Ok(()) => {
                            self.app_update_message =
                                "Restarting SpotiFast to finish the update…".to_string();
                            self.exit_for_app_update = true;
                        }
                        Err(error) => {
                            self.app_update_busy = false;
                            self.app_update_message = error;
                        }
                    },
                    Err(error) => {
                        self.app_update_busy = false;
                        self.app_update_message = error;
                    }
                },
            }
        }

        if self.pending_volume_ipc {
            let due = self
                .last_volume_ipc
                .map(|t| {
                    Instant::now().duration_since(t)
                        >= Duration::from_millis(VOLUME_IPC_DEBOUNCE_MS)
                })
                .unwrap_or(true);
            if due {
                self.flush_volume_ipc_now();
            }
        }

        // Check if active player finished playing
        if let Some(id) = self.active_player_id {
            if self.procs.try_wait_id(id) == Some(true) {
                self.active_player_id = None;
                self.paused = false;
                self.playback_offset_secs = 0.0;
                if self.auto_advance {
                    self.play_next(true);
                } else {
                    self.status = "Track finished.".to_string();
                }
            }
        }
        self.procs.reap_finished();
    }
}

impl eframe::App for PlayerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_background();

        // Keyboard shortcuts: Space toggles play/pause when not in a text box
        if ui.input(|i| i.key_pressed(egui::Key::Space)) && !ui.memory(|m| m.focused().is_some()) {
            self.toggle_pause_resume();
        }
        if ui.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::F)) {
            self.active_view = ActiveView::Search;
        }

        // Keep playback progress and input animation responsive while audio is running.
        // A 150 ms repaint interval made wheel scrolling feel visibly choppy during playback.
        if self.active_player_id.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        } else if self.in_flight_jobs > 0
            || self.pending_volume_ipc
            || self.settings_busy
            || self.app_update_busy
        {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }

        if self.exit_for_app_update {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let pal = self.palette;

        // ---------------------------------------------------------------------
        // LEFT SIDEBAR (Spotify navigation & playlists)
        // ---------------------------------------------------------------------
        Panel::left("spotify_sidebar")
            .exact_size(232.0)
            .frame(
                egui::Frame::new()
                    .fill(pal.bg_sidebar)
                    .inner_margin(Margin::symmetric(12, 16)),
            )
            .show(ui, |ui| {
                brand_header(ui, &pal);
                ui.add_space(10.0);

                // Navigation items
                let nav_items = [
                    (AppIcon::Search, "Search", None, ActiveView::Search),
                    (
                        AppIcon::Library,
                        "Your Library",
                        (!self.tracks.is_empty()).then(|| self.tracks.len().to_string()),
                        ActiveView::Library,
                    ),
                    (
                        AppIcon::Heart,
                        "Liked Songs",
                        (!self.liked_tracks.is_empty())
                            .then(|| self.liked_tracks.len().to_string()),
                        ActiveView::Liked,
                    ),
                    (
                        AppIcon::Download,
                        "Downloaded",
                        (!self.cached_items.is_empty())
                            .then(|| self.cached_items.len().to_string()),
                        ActiveView::Downloaded,
                    ),
                    (
                        AppIcon::Queue,
                        "Queue",
                        (!self.user_queue.is_empty()).then(|| self.user_queue.len().to_string()),
                        ActiveView::Queue,
                    ),
                    (AppIcon::Playlist, "Playlists", None, ActiveView::Playlists),
                    (AppIcon::Settings, "Settings", None, ActiveView::Settings),
                ];

                for (icon, label, badge, view) in nav_items {
                    if sidebar_nav_item(ui, icon, label, badge, self.active_view == view, &pal)
                        .clicked()
                    {
                        self.active_view = view;
                    }
                }

                // Bottom of sidebar: Cache status & Clear Cache
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.add_space(8.0);
                    if ui.button("Clear Cache").clicked() {
                        match clear_audio_cache() {
                            Ok(c) => {
                                self.status = format!("Cleared {c} cached file(s).");
                                self.refresh_cache_state();
                            }
                            Err(e) => self.status = e,
                        }
                    }
                    ui.horizontal(|ui| {
                        draw_inline_icon(ui, AppIcon::Drive, 16.0, pal.text_faint);
                        ui.label(
                            RichText::new(format!(
                                "{} cached ({})",
                                self.cached_items.len(),
                                format_bytes(self.total_cache_bytes)
                            ))
                            .size(11.5)
                            .color(pal.text_faint),
                        );
                    });
                });
            });

        // ---------------------------------------------------------------------
        // BOTTOM PLAYER BAR (Spotify style)
        // ---------------------------------------------------------------------
        Panel::bottom("spotify_player_bar")
            .exact_size(92.0)
            .frame(
                egui::Frame::new()
                    .fill(pal.bg_surface)
                    .stroke(Stroke::new(1.0, pal.bg_border))
                    .inner_margin(Margin::symmetric(20, 10)),
            )
            .show(ui, |ui| {
                let full = ui.available_rect_before_wrap();
                let is_narrow = full.width() < 720.0;

                let center_w = (full.width() * 0.44).clamp(180.0, 600.0);
                let side_w = ((full.width() - center_w) * 0.5).max(0.0);
                let left_w = side_w;
                let right_w = side_w;

                let left_rect = Rect::from_min_size(full.min, vec2(left_w, full.height()));
                let control_center_y = full.center().y - 14.0;
                let track_info_rect = Rect::from_center_size(
                    Pos2::new(left_rect.center().x, full.center().y),
                    left_rect.size(),
                );
                let center_rect = Rect::from_min_size(
                    Pos2::new(left_rect.right(), full.top()),
                    vec2(center_w, full.height()),
                );
                let right_rect = Rect::from_min_size(
                    Pos2::new(center_rect.right(), full.top()),
                    vec2(right_w, full.height()),
                );

                // --- LEFT: Current Track Title and Artist ---
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(track_info_rect)
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        // Keep the title and artist grouped and vertically centered with the
                        // transport controls, without the decorative initial tile.
                        ui.spacing_mut().item_spacing = vec2(12.0, 0.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing = vec2(0.0, 3.0);
                            let (title, artist) = match &self.active_track {
                                Some(t) => (t.display_title(), t.display_artist()),
                                None => {
                                    ("Nothing playing".to_string(), "YouTube Player".to_string())
                                }
                            };

                            ui.add(
                                egui::Label::new(
                                    RichText::new(title)
                                        .font(track_bold_font(15.0))
                                        .strong()
                                        .color(pal.text_bright),
                                )
                                .truncate(),
                            );
                            ui.add(
                                egui::Label::new(
                                    RichText::new(artist)
                                        .font(track_bold_font(12.5))
                                        .color(pal.text_subtle),
                                )
                                .truncate(),
                            );
                        });
                    },
                );

                // --- CENTER: Center both playback rows against the entire player bar. ---
                let control_spacing = if is_narrow { 4.0 } else { 8.0 };
                let controls_width = 32.0 * 4.0 + 34.0 + control_spacing * 4.0;
                let controls_rect = Rect::from_center_size(
                    Pos2::new(full.center().x, control_center_y),
                    vec2(controls_width, 36.0),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(controls_rect)
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing = vec2(control_spacing, 0.0);
                        if transport_button(ui, AppIcon::Shuffle, self.shuffle, &pal).clicked() {
                            self.shuffle = !self.shuffle;
                            let current_track = self
                                .queue_pos
                                .and_then(|p| self.play_order.get(p).copied())
                                .or(self.last_track_index);
                            self.rebuild_order(current_track);
                            self.save_state();
                        }
                        if transport_button(ui, AppIcon::Previous, false, &pal).clicked() {
                            self.play_prev();
                        }
                        let is_playing = self.active_player_id.is_some() && !self.paused;
                        if circle_play_button(ui, is_playing, 34.0, &pal).clicked() {
                            self.toggle_pause_resume();
                        }
                        if transport_button(ui, AppIcon::Next, false, &pal).clicked() {
                            self.play_next(false);
                        }
                        if transport_button(ui, AppIcon::Repeat, self.loop_playlist, &pal).clicked()
                        {
                            self.loop_playlist = !self.loop_playlist;
                            self.save_state();
                        }
                    },
                );

                let pos = self.current_playback_position();
                let dur = self.current_track_duration;
                let pos_text = format_time(pos);
                let total_text = if dur > 0.0 {
                    format_time(dur)
                } else {
                    "--:--".to_string()
                };
                let label_font = FontId::proportional(11.5);
                let pos_width = ui
                    .painter()
                    .layout_no_wrap(pos_text.clone(), label_font.clone(), pal.text_subtle)
                    .size()
                    .x;
                let total_width = ui
                    .painter()
                    .layout_no_wrap(total_text.clone(), label_font, pal.text_subtle)
                    .size()
                    .x;
                let scrubber_w = (center_w - 90.0).max(80.0);
                let time_row_width = pos_width + 16.0 + scrubber_w + total_width;
                let time_rect = Rect::from_center_size(
                    Pos2::new(full.center().x, full.center().y + 22.0),
                    vec2(time_row_width, 16.0),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(time_rect)
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing = vec2(8.0, 0.0);
                        ui.label(RichText::new(pos_text).size(11.5).color(pal.text_subtle));
                        if let Some(target_seek) = seek_scrubber(ui, scrubber_w, pos, dur, &pal) {
                            self.seek_to(target_seek);
                        }
                        ui.label(RichText::new(total_text).size(11.5).color(pal.text_subtle));
                    },
                );

                // --- RIGHT: Volume & Auto-Advance Toggle ---
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(right_rect)
                        .layout(Layout::right_to_left(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing =
                            vec2(if is_narrow { 4.0 } else { 10.0 }, 0.0);
                        if is_narrow {
                            ui.spacing_mut().button_padding = vec2(4.0, 3.0);
                        }

                        // Auto-advance toggle
                        let auto_label = if self.auto_advance {
                            "Auto: ON"
                        } else {
                            "Auto: OFF"
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(auto_label)
                                        .size(if is_narrow { 9.5 } else { 11.0 })
                                        .color(if self.auto_advance {
                                            pal.green_accent
                                        } else {
                                            pal.text_bright
                                        }),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            self.auto_advance = !self.auto_advance;
                            self.save_state();
                        }

                        // Mute control sits to the right of the slider.
                        let (mute_rect, mute_resp) =
                            ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                        if mute_resp.hovered() {
                            ui.painter()
                                .circle_filled(mute_rect.center(), 14.0, pal.bg_highlight);
                        }
                        draw_speaker_icon(
                            ui.painter(),
                            mute_rect.center(),
                            self.volume < 1.0,
                            pal.text_bright,
                        );
                        if mute_resp.clicked() {
                            if self.volume > 0.0 {
                                self.volume = 0.0;
                            } else {
                                self.volume = 80.0;
                            }
                            self.on_volume_changed();
                        }

                        // Volume slider and the like button to its left.
                        let slider_w = if is_narrow { 40.0 } else { 85.0 };
                        if let Some(new_vol) = volume_slider(ui, slider_w, self.volume, &pal) {
                            self.volume = new_vol;
                            self.on_volume_changed();
                        }
                        if let Some(track) = self.active_track.clone() {
                            let (heart_rect, heart_resp) =
                                ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                            if heart_resp.hovered() {
                                ui.painter().circle_filled(
                                    heart_rect.center(),
                                    14.0,
                                    pal.bg_highlight,
                                );
                            }
                            draw_player_heart(
                                ui.painter(),
                                heart_rect.center(),
                                self.is_liked(&track),
                                &pal,
                            );
                            if heart_resp.clicked() {
                                self.toggle_like(track);
                            }
                        }
                    },
                );
            });

        // ---------------------------------------------------------------------
        // CENTRAL PANEL (Views: Search / Library / Liked / Downloaded / Queue / Playlists)
        // ---------------------------------------------------------------------
        CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(pal.bg_base)
                    .inner_margin(Margin::symmetric(28, 22)),
            )
            .show(ui, |ui| match self.active_view {
                ActiveView::Search => self.render_search_view(ui),
                ActiveView::Library => self.render_library_view(ui),
                ActiveView::Liked => self.render_liked_view(ui),
                ActiveView::Downloaded => self.render_downloaded_view(ui),
                ActiveView::Queue => self.render_queue_view(ui),
                ActiveView::Playlists => self.render_playlists_view(ui),
                ActiveView::Settings => self.render_settings_view(ui),
            });
    }

    fn on_exit(&mut self) {
        self.save_state();
        self.stop_active_player(true);
        self.procs.kill_all();
    }
}

// -----------------------------------------------------------------------------
// VIEW IMPLEMENTATIONS
// -----------------------------------------------------------------------------

impl PlayerApp {
    fn render_settings_view(&mut self, ui: &mut egui::Ui) {
        let pal = self.palette;
        let mut check_updates = false;
        let mut install = None;
        let mut rollback = None;
        let mut save_app_source = false;
        let mut check_app = false;
        let mut install_app = false;
        let mut rollback_app = false;

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading(RichText::new("Settings").font(track_bold_font(27.0)));
            ui.label(
                RichText::new("Choose how SpotiFast starts and keep its player tools up to date.")
                    .color(pal.text_subtle),
            );
            ui.add_space(18.0);

            egui::Frame::new()
                .fill(pal.bg_surface)
                .stroke(Stroke::new(1.0, pal.bg_border))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(18))
                .show(ui, |ui| {
                    ui.label(RichText::new("Startup").font(track_bold_font(18.0)));
                    ui.add_space(5.0);
                    ui.label(
                        RichText::new("Open SpotiFast automatically when you sign in to Windows.")
                            .color(pal.text_subtle),
                    );
                    ui.add_space(10.0);
                    let mut enabled = self.settings.run_at_startup;
                    let checkbox = ui.checkbox(&mut enabled, "Run at startup");
                    if enabled {
                        let spacing = ui.spacing();
                        let icon_center = Pos2::new(
                            checkbox.rect.left() + spacing.icon_width * 0.5,
                            checkbox.rect.center().y,
                        );
                        let check_rect = Rect::from_center_size(
                            icon_center,
                            vec2(spacing.icon_width_inner, spacing.icon_width_inner),
                        );
                        ui.painter().line_segment(
                            [
                                Pos2::new(check_rect.left(), check_rect.center().y),
                                Pos2::new(check_rect.center().x, check_rect.bottom()),
                            ],
                            Stroke::new(2.0, pal.green_accent),
                        );
                        ui.painter().line_segment(
                            [
                                Pos2::new(check_rect.center().x, check_rect.bottom()),
                                Pos2::new(check_rect.right(), check_rect.top()),
                            ],
                            Stroke::new(2.0, pal.green_accent),
                        );
                    }
                    if checkbox.changed() {
                        self.set_startup_preference(enabled);
                    }
                });

            ui.add_space(14.0);
            egui::Frame::new()
                .fill(pal.bg_surface)
                .stroke(Stroke::new(1.0, pal.bg_border))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(18))
                .show(ui, |ui| {
                    ui.label(RichText::new("SpotiFast updates").font(track_bold_font(18.0)));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Check GitHub Releases for a newer app version. The updater stages and validates the build, then restarts SpotiFast to install it.")
                            .color(pal.text_subtle),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label("GitHub repository");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.app_update_repository_input)
                                .hint_text("owner/repository")
                                .desired_width(260.0),
                        );
                        if ui.add_enabled(!self.app_update_busy, egui::Button::new("Save")).clicked() {
                            save_app_source = true;
                        }
                    });
                    ui.label(
                        RichText::new("Each release must include SpotiFast.exe and SpotiFastUpdater.exe.")
                            .size(12.0)
                            .color(pal.text_faint),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(!self.app_update_busy, egui::Button::new(if self.app_update_busy { "Working…" } else { "Check app updates" }))
                            .clicked()
                        {
                            check_app = true;
                        }
                        if self.app_update_info.as_ref().is_some_and(|info| info.update_available)
                            && ui.add_enabled(!self.app_update_busy, egui::Button::new("Install update")).clicked()
                        {
                            install_app = true;
                        }
                        if self
                            .app_update_info
                            .as_ref()
                            .map(|info| info.rollback_available)
                            .unwrap_or_else(updater::app_rollback_available)
                            && ui.add_enabled(!self.app_update_busy, egui::Button::new("Roll back app")).clicked()
                        {
                            rollback_app = true;
                        }
                    });
                    ui.label(
                        RichText::new(&self.app_update_message).color(
                            if self.app_update_message.contains("Could not")
                                || self.app_update_message.contains("needs")
                                || self.app_update_message.contains("Enter")
                            {
                                Color32::from_rgb(255, 130, 130)
                            } else {
                                pal.text_subtle
                            },
                        ),
                    );
                });

            ui.add_space(14.0);
            egui::Frame::new()
                .fill(pal.bg_surface)
                .stroke(Stroke::new(1.0, pal.bg_border))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(18))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Player tools").font(track_bold_font(18.0)));
                            ui.label(
                                RichText::new("Check and update yt-dlp and mpv. MPV Windows builds come from shinchiro, listed by the mpv project.")
                                    .color(pal.text_subtle),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let button = if self.settings_busy { "Working…" } else { "Check for updates" };
                            if ui.add_enabled(!self.settings_busy, egui::Button::new(button)).clicked() {
                                check_updates = true;
                            }
                        });
                    });
                    ui.add_space(12.0);
                    ui.separator();

                    for status in self.tool_statuses.clone() {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(status.kind.label()).strong());
                                ui.label(
                                    RichText::new(format!("Installed: {}", status.current_version))
                                        .size(12.0)
                                        .color(pal.text_subtle),
                                );
                                if status.latest_version != "Not checked" {
                                    ui.label(
                                        RichText::new(format!("Latest: {}", status.latest_version))
                                            .size(12.0)
                                            .color(pal.text_faint),
                                    );
                                }
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if status.rollback_available
                                    && ui.add_enabled(!self.settings_busy, egui::Button::new("Roll back" )).clicked()
                                {
                                    rollback = Some(status.kind);
                                }
                                if status.update_available {
                                    if ui.add_enabled(!self.settings_busy, egui::Button::new("Update" )).clicked() {
                                        install = Some(status.kind);
                                    }
                                } else if status.latest_version != "Not checked" {
                                    ui.label(RichText::new("Up to date").color(pal.green_accent));
                                }
                            });
                        });
                        ui.separator();
                    }

                    let message_color = if self.settings_message.contains("failed")
                        || self.settings_message.contains("Could not")
                        || self.settings_message.contains("not found")
                        || self.settings_message.contains("unavailable")
                    {
                        Color32::from_rgb(255, 130, 130)
                    } else {
                        pal.text_subtle
                    };
                    ui.label(RichText::new(&self.settings_message).color(message_color));
                });

            ui.add_space(14.0);
            egui::Frame::new()
                .fill(pal.bg_surface)
                .stroke(Stroke::new(1.0, pal.bg_border))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::same(18))
                .show(ui, |ui| {
                    ui.label(RichText::new("Your music stays safe").font(track_bold_font(16.0)));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Player tools are stored separately from your playlists and downloaded songs. Updating or rolling back yt-dlp or mpv leaves your library and music files in place.")
                            .color(pal.text_subtle),
                    );
                });
        });

        if check_updates {
            self.check_tool_updates();
        }
        if let Some(kind) = install {
            self.install_tool_update(kind);
        }
        if let Some(kind) = rollback {
            self.rollback_tool(kind);
        }
        if save_app_source {
            self.save_app_update_repository();
        }
        if check_app {
            self.check_app_updates();
        }
        if install_app {
            self.download_app_update();
        }
        if rollback_app {
            self.rollback_app_update();
        }
    }

    fn render_search_view(&mut self, ui: &mut egui::Ui) {
        let pal = self.palette;

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Find something to play")
                    .size(27.0)
                    .strong()
                    .color(pal.text_bright),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(&self.status).size(12.0).color(pal.text_faint));
            });
        });
        ui.label(
            RichText::new("Search songs, artists, or paste a YouTube link.")
                .size(13.0)
                .color(pal.text_subtle),
        );
        ui.add_space(14.0);

        // Search Input Bar
        ui.horizontal(|ui| {
            let btn_w = 90.0;
            let clear_w = 32.0;
            let input_w = (ui.available_width() - btn_w - clear_w - 16.0).max(140.0);

            let res = ui.add_sized(
                [input_w, 42.0],
                egui::TextEdit::singleline(&mut self.search_input)
                    .hint_text("What do you want to listen to?")
                    .font(FontId::proportional(15.0)),
            );

            if icon_button(ui, AppIcon::Close, clear_w, pal.text_subtle).clicked() {
                self.search_input.clear();
            }

            let search_clicked = spotify_pill_button(ui, "Search", true, &pal).clicked();
            if search_clicked || (res.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
            {
                self.trigger_search();
            }
        });

        // Quick Suggestion Pills
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Try:")
                    .size(12.0)
                    .strong()
                    .color(pal.text_faint),
            );
            let suggestions = [
                "Top Hits",
                "The Weeknd",
                "Metro Boomin",
                "Travis Scott",
                "Drake",
                "Daft Punk",
                "Doja Cat",
                "Lofi Beats",
                "Synthwave",
                "Rock Classics",
            ];
            for tag in suggestions {
                let btn = ui.add(
                    egui::Button::new(RichText::new(tag).size(12.0).color(pal.text_subtle))
                        .fill(pal.bg_surface)
                        .corner_radius(CornerRadius::same(12)),
                );
                if btn.clicked() {
                    self.search_input = tag.to_string();
                    self.trigger_search();
                }
            }
        });

        ui.add_space(8.0);
        if self.search_in_flight {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(&self.search_status)
                        .size(13.0)
                        .color(pal.green_accent),
                );
            });
        } else {
            ui.label(
                RichText::new(&self.search_status)
                    .size(12.5)
                    .color(pal.text_subtle),
            );
        }

        ui.separator();

        // Search Results Table
        let results_len = self.search_results.len();
        if results_len == 0 {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                draw_inline_icon(ui, AppIcon::Music, 56.0, pal.bg_border);
                ui.label(
                    RichText::new(
                        "Find your favorite songs, artists, or paste any YouTube URL above.",
                    )
                    .color(pal.text_subtle),
                );
            });
            return;
        }

        // Table Header
        self.render_table_header(ui);

        let mut clicked_play: Option<PlaylistEntry> = None;
        let mut clicked_queue: Option<PlaylistEntry> = None;
        let mut clicked_like: Option<PlaylistEntry> = None;
        let mut clicked_cache: Option<PlaylistEntry> = None;
        let mut clicked_playlist: Option<(usize, PlaylistEntry)> = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .wheel_scroll_multiplier(vec2(1.0, 0.8))
            .show_rows(ui, 48.0, results_len, |ui, range| {
                for i in range {
                    let entry = &self.search_results[i];
                    let is_active = self
                        .active_track
                        .as_ref()
                        .map(|t| t.matches_track(entry))
                        .unwrap_or(false);
                    let is_cached = entry
                        .cache_key()
                        .as_deref()
                        .map(|k| self.cached_keys.contains(k))
                        .unwrap_or(false);
                    let is_liked = self.is_liked(entry);

                    let (play, queue, like, cache, playlist_idx) = self.render_track_row(
                        ui,
                        i + 1,
                        entry,
                        is_active,
                        is_cached,
                        is_liked,
                        &self.saved_playlists,
                    );

                    if play {
                        clicked_play = Some(entry.clone());
                    }
                    if queue {
                        clicked_queue = Some(entry.clone());
                    }
                    if like {
                        clicked_like = Some(entry.clone());
                    }
                    if cache {
                        clicked_cache = Some(entry.clone());
                    }
                    if let Some(playlist_idx) = playlist_idx {
                        clicked_playlist = Some((playlist_idx, entry.clone()));
                    }
                }
            });

        if let Some(entry) = clicked_play {
            self.play_entry(entry);
        }
        if let Some(entry) = clicked_queue {
            self.user_queue.push(entry.clone());
            self.status = format!("Added to queue: {}", entry.display_title());
        }
        if let Some(entry) = clicked_like {
            self.toggle_like(entry);
        }
        if let Some(entry) = clicked_cache {
            self.download_to_cache(&entry);
            self.status = format!("Caching: {}", entry.display_title());
        }
        if let Some((playlist_idx, entry)) = clicked_playlist {
            self.add_track_to_saved_playlist(playlist_idx, entry);
        }
    }

    fn render_library_view(&mut self, ui: &mut egui::Ui) {
        let pal = self.palette;
        let current_playlist_name = self
            .selected_saved_idx
            .and_then(|index| self.saved_playlists.get(index))
            .map(|playlist| playlist.name.clone())
            .unwrap_or_else(|| self.playlist_title.clone());
        let mut selected_playlist = None;

        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("library_playlist_selector")
                .selected_text(RichText::new(current_playlist_name).heading().strong())
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (index, playlist) in self.saved_playlists.iter().enumerate() {
                        if ui
                            .selectable_label(
                                self.selected_saved_idx == Some(index),
                                &playlist.name,
                            )
                            .clicked()
                        {
                            selected_playlist = Some(index);
                        }
                    }
                    if self.saved_playlists.is_empty() {
                        ui.label(RichText::new("No saved playlists yet.").color(pal.text_faint));
                    }
                });
            ui.label(
                RichText::new(format!("({} songs)", self.tracks.len()))
                    .size(13.0)
                    .color(pal.text_subtle),
            );

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(&self.status).size(12.0).color(pal.text_faint));
            });
        });
        if let Some(index) = selected_playlist {
            self.open_saved_playlist(index);
        }
        ui.add_space(8.0);

        // Actions & Filter
        ui.horizontal(|ui| {
            if spotify_icon_pill_button(ui, AppIcon::Play, "Play All", true, &pal).clicked() {
                if !self.play_order.is_empty() {
                    self.play_queue_pos(0);
                }
            }
            if spotify_icon_pill_button(ui, AppIcon::Shuffle, "Shuffle", false, &pal).clicked() {
                self.shuffle = true;
                self.rebuild_order(None);
                if !self.play_order.is_empty() {
                    self.play_queue_pos(0);
                }
            }

            ui.add_space(14.0);
            let filter_w = (ui.available_width() - 80.0).max(120.0);
            ui.add_sized(
                [filter_w, 30.0],
                egui::TextEdit::singleline(&mut self.list_filter)
                    .hint_text("Filter tracks in playlist..."),
            );
            if ui.button("Clear").clicked() {
                self.list_filter.clear();
            }
        });

        ui.separator();

        let filter_lower = self.list_filter.trim().to_lowercase();
        let visible_indices: Vec<usize> = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                filter_lower.is_empty()
                    || t.title_lower.contains(&filter_lower)
                    || t.artist_lower.contains(&filter_lower)
            })
            .map(|(idx, _)| idx)
            .collect();

        if visible_indices.is_empty() {
            ui.add_space(20.0);
            ui.label(
                RichText::new("No tracks match your filter, or playlist is empty.")
                    .color(pal.text_subtle),
            );
            return;
        }

        self.render_table_header(ui);

        let mut clicked_play: Option<usize> = None;
        let mut clicked_queue: Option<PlaylistEntry> = None;
        let mut clicked_like: Option<PlaylistEntry> = None;
        let mut clicked_cache: Option<PlaylistEntry> = None;
        let mut clicked_playlist: Option<(usize, PlaylistEntry)> = None;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .wheel_scroll_multiplier(vec2(1.0, 0.8))
            .show_rows(ui, 48.0, visible_indices.len(), |ui, range| {
                for row in range {
                    let track_idx = visible_indices[row];
                    let entry = &self.tracks[track_idx];
                    let is_active = self
                        .active_track
                        .as_ref()
                        .map(|t| t.matches_track(entry))
                        .unwrap_or(false);
                    let is_cached = entry
                        .cache_key()
                        .as_deref()
                        .map(|k| self.cached_keys.contains(k))
                        .unwrap_or(false);
                    let is_liked = self.is_liked(entry);

                    let (play, queue, like, cache, playlist_idx) = self.render_track_row(
                        ui,
                        row + 1,
                        entry,
                        is_active,
                        is_cached,
                        is_liked,
                        &self.saved_playlists,
                    );

                    if play {
                        clicked_play = Some(track_idx);
                    }
                    if queue {
                        clicked_queue = Some(entry.clone());
                    }
                    if like {
                        clicked_like = Some(entry.clone());
                    }
                    if cache {
                        clicked_cache = Some(entry.clone());
                    }
                    if let Some(playlist_idx) = playlist_idx {
                        clicked_playlist = Some((playlist_idx, entry.clone()));
                    }
                }
            });

        if let Some(track_idx) = clicked_play {
            if let Some(pos) = self.play_order.iter().position(|&i| i == track_idx) {
                self.play_queue_pos(pos);
            } else {
                let entry = self.tracks[track_idx].clone();
                self.play_entry(entry);
            }
        }
        if let Some(entry) = clicked_queue {
            self.user_queue.push(entry.clone());
            self.status = format!("Added to queue: {}", entry.display_title());
        }
        if let Some(entry) = clicked_like {
            self.toggle_like(entry);
        }
        if let Some(entry) = clicked_cache {
            self.download_to_cache(&entry);
            self.status = format!("Caching: {}", entry.display_title());
        }
        if let Some((playlist_idx, entry)) = clicked_playlist {
            self.add_track_to_saved_playlist(playlist_idx, entry);
        }
    }

    fn render_liked_view(&mut self, ui: &mut egui::Ui) {
        let pal = self.palette;

        ui.horizontal(|ui| {
            draw_inline_icon(ui, AppIcon::Heart, 28.0, pal.green_accent);
            ui.label(
                RichText::new("Liked Songs")
                    .heading()
                    .strong()
                    .color(pal.green_accent),
            );
            ui.label(
                RichText::new(format!("({} songs)", self.liked_tracks.len()))
                    .size(13.0)
                    .color(pal.text_subtle),
            );
        });
        ui.add_space(8.0);

        if self.liked_tracks.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                draw_inline_icon(ui, AppIcon::Heart, 56.0, pal.bg_border);
                ui.label(
                    RichText::new(
                        "Songs you like will appear here. Click the heart on any track to save it!",
                    )
                    .color(pal.text_subtle),
                );
            });
            return;
        }

        if spotify_icon_pill_button(ui, AppIcon::Play, "Play Liked Songs", true, &pal).clicked() {
            if let Some(first) = self.liked_tracks.first().cloned() {
                self.play_entry(first);
            }
        }
        ui.add_space(8.0);
        ui.separator();

        self.render_table_header(ui);

        let mut clicked_play: Option<PlaylistEntry> = None;
        let mut clicked_queue: Option<PlaylistEntry> = None;
        let mut clicked_unlike: Option<PlaylistEntry> = None;
        let mut clicked_cache: Option<PlaylistEntry> = None;
        let mut clicked_playlist: Option<(usize, PlaylistEntry)> = None;

        let len = self.liked_tracks.len();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .wheel_scroll_multiplier(vec2(1.0, 0.8))
            .show_rows(ui, 48.0, len, |ui, range| {
                for i in range {
                    let entry = &self.liked_tracks[i];
                    let is_active = self
                        .active_track
                        .as_ref()
                        .map(|t| t.matches_track(entry))
                        .unwrap_or(false);
                    let is_cached = entry
                        .cache_key()
                        .as_deref()
                        .map(|k| self.cached_keys.contains(k))
                        .unwrap_or(false);

                    let (play, queue, unlike, cache, playlist_idx) = self.render_track_row(
                        ui,
                        i + 1,
                        entry,
                        is_active,
                        is_cached,
                        true,
                        &self.saved_playlists,
                    );

                    if play {
                        clicked_play = Some(entry.clone());
                    }
                    if queue {
                        clicked_queue = Some(entry.clone());
                    }
                    if unlike {
                        clicked_unlike = Some(entry.clone());
                    }
                    if cache {
                        clicked_cache = Some(entry.clone());
                    }
                    if let Some(playlist_idx) = playlist_idx {
                        clicked_playlist = Some((playlist_idx, entry.clone()));
                    }
                }
            });

        if let Some(entry) = clicked_play {
            self.play_entry(entry);
        }
        if let Some(entry) = clicked_queue {
            self.user_queue.push(entry.clone());
            self.status = format!("Added to queue: {}", entry.display_title());
        }
        if let Some(entry) = clicked_unlike {
            self.toggle_like(entry);
        }
        if let Some(entry) = clicked_cache {
            self.download_to_cache(&entry);
        }
        if let Some((playlist_idx, entry)) = clicked_playlist {
            self.add_track_to_saved_playlist(playlist_idx, entry);
        }
    }

    fn render_downloaded_view(&mut self, ui: &mut egui::Ui) {
        let pal = self.palette;

        ui.horizontal(|ui| {
            draw_inline_icon(ui, AppIcon::Download, 28.0, pal.text_bright);
            ui.label(
                RichText::new("Downloaded Tracks (Offline)")
                    .heading()
                    .strong()
                    .color(pal.text_bright),
            );
            ui.label(
                RichText::new(format!(
                    "({} files, {})",
                    self.cached_items.len(),
                    format_bytes(self.total_cache_bytes)
                ))
                .size(13.0)
                .color(pal.text_subtle),
            );

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Clear All Cache").clicked() {
                    match clear_audio_cache() {
                        Ok(c) => {
                            self.status = format!("Cleared {c} cached file(s).");
                            self.refresh_cache_state();
                        }
                        Err(e) => self.status = e,
                    }
                }
            });
        });

        ui.add_space(8.0);
        if self.cached_items.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                draw_inline_icon(ui, AppIcon::Drive, 56.0, pal.bg_border);
                ui.label(RichText::new("No locally cached songs yet. Songs you play or click Cache on will be saved here for offline playback.").color(pal.text_subtle));
            });
            return;
        }

        ui.separator();

        let mut play_target: Option<PlaylistEntry> = None;
        let items_len = self.cached_items.len();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .wheel_scroll_multiplier(vec2(1.0, 0.8))
            .show_rows(ui, 44.0, items_len, |ui, range| {
                for i in range {
                    let item = &self.cached_items[i];
                    let width = ui.available_width();
                    let (rect, resp) = ui.allocate_exact_size(vec2(width, 40.0), Sense::click());
                    let hovered = resp.hovered();

                    if hovered {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(4), pal.bg_highlight);
                    }

                    // Index & Play icon
                    let c_y = rect.center().y;
                    ui.painter().text(
                        Pos2::new(rect.left() + 10.0, c_y),
                        Align2::LEFT_CENTER,
                        format!("{}.", i + 1),
                        FontId::proportional(12.0),
                        pal.text_faint,
                    );

                    // Title & Artist
                    ui.painter().text(
                        Pos2::new(rect.left() + 45.0, c_y - 8.0),
                        Align2::LEFT_CENTER,
                        &item.title,
                        FontId::proportional(13.5),
                        pal.text_bright,
                    );
                    ui.painter().text(
                        Pos2::new(rect.left() + 45.0, c_y + 8.0),
                        Align2::LEFT_CENTER,
                        &item.artist,
                        FontId::proportional(11.5),
                        pal.text_subtle,
                    );

                    // Size
                    ui.painter().text(
                        Pos2::new(rect.right() - 110.0, c_y),
                        Align2::RIGHT_CENTER,
                        format_bytes(item.file_size_bytes),
                        FontId::proportional(12.0),
                        pal.green_accent,
                    );

                    // Play button
                    let btn_rect = Rect::from_min_size(
                        Pos2::new(rect.right() - 80.0, c_y - 12.0),
                        vec2(60.0, 24.0),
                    );
                    let btn_resp = ui.interact(
                        btn_rect,
                        ui.scope_id().with(("play_cached", i)),
                        Sense::click(),
                    );
                    if btn_resp.hovered() {
                        ui.painter()
                            .rect_filled(btn_rect, CornerRadius::same(12), pal.bg_active);
                    }
                    draw_app_icon(
                        ui.painter(),
                        Pos2::new(btn_rect.left() + 16.0, btn_rect.center().y),
                        AppIcon::Play,
                        14.0,
                        pal.green_accent,
                    );
                    ui.painter().text(
                        Pos2::new(btn_rect.left() + 24.0, btn_rect.center().y),
                        Align2::LEFT_CENTER,
                        "Play",
                        FontId::proportional(12.0),
                        pal.green_accent,
                    );

                    if resp.double_clicked() || btn_resp.clicked() {
                        let entry = PlaylistEntry {
                            id: Some(item.key.clone()),
                            title: Some(item.title.clone()),
                            artist: Some(item.artist.clone()),
                            duration: item.duration,
                            url: Some(format!("https://www.youtube.com/watch?v={}", item.key)),
                            webpage_url: None,
                            channel: Some(item.artist.clone()),
                            uploader: Some(item.artist.clone()),
                            title_lower: item.title.to_lowercase(),
                            artist_lower: item.artist.to_lowercase(),
                        };
                        play_target = Some(entry);
                    }
                }
            });

        if let Some(entry) = play_target {
            self.play_entry(entry);
        }
    }

    fn render_queue_view(&mut self, ui: &mut egui::Ui) {
        let pal = self.palette;

        ui.horizontal(|ui| {
            draw_inline_icon(ui, AppIcon::Queue, 28.0, pal.text_bright);
            ui.label(
                RichText::new("Play Queue")
                    .heading()
                    .strong()
                    .color(pal.text_bright),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Clear Queue").clicked() {
                    self.user_queue.clear();
                    self.status = "Cleared queue.".to_string();
                }
            });
        });
        ui.add_space(8.0);

        // Now playing card
        ui.label(
            RichText::new("NOW PLAYING")
                .size(12.0)
                .strong()
                .color(pal.text_faint),
        );
        if let Some(track) = &self.active_track {
            let card_rect = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 52.0));
            ui.painter()
                .rect_filled(card_rect, CornerRadius::same(6), pal.bg_surface);
            ui.painter().text(
                Pos2::new(card_rect.left() + 16.0, card_rect.center().y - 8.0),
                Align2::LEFT_CENTER,
                track.display_title(),
                FontId::proportional(14.0),
                pal.green_accent,
            );
            ui.painter().text(
                Pos2::new(card_rect.left() + 16.0, card_rect.center().y + 10.0),
                Align2::LEFT_CENTER,
                track.display_artist(),
                FontId::proportional(12.0),
                pal.text_subtle,
            );
            ui.add_space(58.0);
        } else {
            ui.label(RichText::new("Nothing currently playing.").color(pal.text_subtle));
            ui.add_space(8.0);
        }

        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("UP NEXT ({})", self.user_queue.len()))
                .size(12.0)
                .strong()
                .color(pal.text_faint),
        );
        ui.separator();

        if self.user_queue.is_empty() {
            ui.label(
                RichText::new("Queue is empty. Click '+ Queue' on any song to add it here.")
                    .color(pal.text_subtle),
            );
            return;
        }

        let mut remove_idx = None;
        let mut play_idx = None;
        for (i, entry) in self.user_queue.iter().enumerate() {
            let width = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(vec2(width, 36.0), Sense::click());
            if resp.hovered() {
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(4), pal.bg_highlight);
            }
            ui.painter().text(
                Pos2::new(rect.left() + 10.0, rect.center().y),
                Align2::LEFT_CENTER,
                format!("{}.", i + 1),
                FontId::proportional(12.0),
                pal.text_faint,
            );
            ui.painter().text(
                Pos2::new(rect.left() + 40.0, rect.center().y),
                Align2::LEFT_CENTER,
                entry.display_title(),
                FontId::proportional(13.0),
                pal.text_bright,
            );

            // Remove button
            let del_rect = Rect::from_min_size(
                Pos2::new(rect.right() - 30.0, rect.center().y - 10.0),
                vec2(20.0, 20.0),
            );
            let del_resp = ui.interact(
                del_rect,
                ui.scope_id().with(("del_queue", i)),
                Sense::click(),
            );
            draw_app_icon(
                ui.painter(),
                del_rect.center(),
                AppIcon::Close,
                12.0,
                pal.text_subtle,
            );
            if del_resp.clicked() {
                remove_idx = Some(i);
            }
            if resp.double_clicked() {
                play_idx = Some(i);
            }
        }

        if let Some(i) = remove_idx {
            self.user_queue.remove(i);
        }
        if let Some(i) = play_idx {
            let track = self.user_queue.remove(i);
            self.play_entry(track);
        }
    }

    fn render_playlists_view(&mut self, ui: &mut egui::Ui) {
        let pal = self.palette;

        ui.label(
            RichText::new("Manage Playlists")
                .heading()
                .strong()
                .color(pal.text_bright),
        );
        ui.add_space(8.0);

        // Import new playlist from URL
        ui.group(|ui| {
            ui.label(
                RichText::new("Import Playlist from YouTube")
                    .strong()
                    .color(pal.text_bright),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 90.0).max(120.0);
                ui.add_sized(
                    [w, 28.0],
                    egui::TextEdit::singleline(&mut self.url_input)
                        .hint_text("YouTube playlist URL..."),
                );
                if spotify_pill_button(ui, "Load", true, &pal).clicked() {
                    self.load_playlist_url();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.add_sized(
                    [200.0, 24.0],
                    egui::TextEdit::singleline(&mut self.playlist_name_input)
                        .hint_text("Playlist Name"),
                );
                if ui.button("Save to List").clicked() {
                    let url = self.url_input.trim().to_string();
                    let name = if !self.playlist_name_input.trim().is_empty() {
                        self.playlist_name_input.trim().to_string()
                    } else if !self.playlist_title.is_empty() {
                        self.playlist_title.clone()
                    } else {
                        url.clone()
                    };
                    if !url.is_empty() {
                        if let Some(p) = self.saved_playlists.iter_mut().find(|p| p.url == url) {
                            p.name = name;
                            if p.tracks.is_empty() && self.url_input.trim() == url {
                                p.tracks = self.tracks.clone();
                            }
                        } else {
                            let tracks = if self.url_input.trim() == url {
                                self.tracks.clone()
                            } else {
                                Vec::new()
                            };
                            self.saved_playlists
                                .push(SavedPlaylist { name, url, tracks });
                        }
                        self.save_state();
                        self.status = "Saved playlist.".to_string();
                    }
                }
            });
        });

        ui.add_space(14.0);
        ui.label(
            RichText::new("Saved Playlists")
                .strong()
                .color(pal.text_bright),
        );
        ui.separator();

        let mut delete_idx = None;
        let mut load_idx = None;
        for (i, p) in self.saved_playlists.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}.", i + 1)).color(pal.text_faint));
                if ui
                    .link(
                        RichText::new(&p.name)
                            .size(14.0)
                            .strong()
                            .color(pal.text_bright),
                    )
                    .clicked()
                {
                    load_idx = Some(i);
                }
                ui.label(
                    RichText::new(format!("({} tracks)", p.tracks.len()))
                        .size(11.0)
                        .color(pal.text_faint),
                );
                ui.label(RichText::new(&p.url).size(11.0).color(pal.text_faint));

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Delete").clicked() {
                        delete_idx = Some(i);
                    }
                });
            });
        }

        if let Some(i) = delete_idx {
            self.saved_playlists.remove(i);
            self.save_state();
        }
        if let Some(i) = load_idx {
            self.open_saved_playlist(i);
        }
    }

    fn render_table_header(&self, ui: &mut egui::Ui) {
        let pal = self.palette;
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
        let c_y = rect.center().y;

        ui.painter().text(
            Pos2::new(rect.left() + 10.0, c_y),
            Align2::LEFT_CENTER,
            "#",
            FontId::proportional(11.0),
            pal.text_faint,
        );
        ui.painter().text(
            Pos2::new(rect.left() + 45.0, c_y),
            Align2::LEFT_CENTER,
            "TITLE",
            FontId::proportional(11.0),
            pal.text_faint,
        );
        ui.painter().text(
            Pos2::new(rect.right() - 190.0, c_y),
            Align2::LEFT_CENTER,
            "DURATION",
            FontId::proportional(11.0),
            pal.text_faint,
        );
    }

    fn render_track_row(
        &self,
        ui: &mut egui::Ui,
        index: usize,
        entry: &PlaylistEntry,
        is_active: bool,
        is_cached: bool,
        is_liked: bool,
        saved_playlists: &[SavedPlaylist],
    ) -> (bool, bool, bool, bool, Option<usize>) {
        let pal = self.palette;
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 48.0), Sense::click());
        let hovered = resp.hovered();

        if is_active {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), pal.bg_active);
            ui.painter().rect_filled(
                Rect::from_min_size(rect.min, vec2(3.0, rect.height())),
                CornerRadius::same(2),
                pal.green_accent,
            );
        } else if hovered {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), pal.bg_highlight);
        }

        let c_y = rect.center().y;

        // Keep row numbers, and show a crisp green play/pause indicator for the active track.
        if is_active {
            let indicator_center = Pos2::new(rect.left() + 18.0, c_y);
            if self.paused {
                ui.painter().rect_filled(
                    Rect::from_center_size(
                        Pos2::new(indicator_center.x - 3.0, indicator_center.y),
                        vec2(3.0, 12.0),
                    ),
                    CornerRadius::same(1),
                    pal.green_accent,
                );
                ui.painter().rect_filled(
                    Rect::from_center_size(
                        Pos2::new(indicator_center.x + 3.0, indicator_center.y),
                        vec2(3.0, 12.0),
                    ),
                    CornerRadius::same(1),
                    pal.green_accent,
                );
            } else {
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(indicator_center.x - 4.0, indicator_center.y - 6.0),
                        Pos2::new(indicator_center.x - 4.0, indicator_center.y + 6.0),
                        Pos2::new(indicator_center.x + 6.0, indicator_center.y),
                    ],
                    pal.green_accent,
                    Stroke::NONE,
                ));
            }
        } else {
            ui.painter().text(
                Pos2::new(rect.left() + 10.0, c_y),
                Align2::LEFT_CENTER,
                format!("{index}"),
                FontId::proportional(12.0),
                pal.text_faint,
            );
        }

        // Small album tile drawn from native shapes to keep scrolling light.
        let cover = Rect::from_center_size(Pos2::new(rect.left() + 52.0, c_y), vec2(32.0, 32.0));
        let cover_fill = match index % 4 {
            0 => Color32::from_rgb(33, 73, 49),
            1 => Color32::from_rgb(63, 47, 77),
            2 => Color32::from_rgb(82, 61, 37),
            _ => Color32::from_rgb(38, 65, 79),
        };
        ui.painter()
            .rect_filled(cover, CornerRadius::same(5), cover_fill);
        let mark = entry
            .display_title()
            .chars()
            .next()
            .unwrap_or('•')
            .to_uppercase()
            .to_string();
        ui.painter().text(
            cover.center(),
            Align2::CENTER_CENTER,
            mark,
            FontId::proportional(15.0),
            pal.green_accent,
        );

        // Title and Artist
        let title_color = if is_active {
            pal.green_accent
        } else {
            pal.text_bright
        };
        let text_painter = ui.painter().with_clip_rect(Rect::from_min_max(
            Pos2::new(rect.left() + 76.0, rect.top()),
            Pos2::new(rect.right() - 200.0, rect.bottom()),
        ));
        text_painter.text(
            Pos2::new(rect.left() + 78.0, c_y - 8.0),
            Align2::LEFT_CENTER,
            entry.display_title(),
            FontId::proportional(13.5),
            title_color,
        );
        text_painter.text(
            Pos2::new(rect.left() + 78.0, c_y + 9.0),
            Align2::LEFT_CENTER,
            entry.display_artist(),
            FontId::proportional(11.5),
            pal.text_subtle,
        );

        // Duration
        ui.painter().text(
            Pos2::new(rect.right() - 190.0, c_y),
            Align2::LEFT_CENTER,
            entry.formatted_duration(),
            FontId::proportional(12.0),
            pal.text_subtle,
        );

        // Row actions live in a compact three-dot menu. Double-clicking the row still plays it.
        let play = resp.double_clicked();
        let mut queue = false;
        let mut like = false;
        let mut cache = false;
        let mut playlist_idx = None;

        let row_id = ui.scope_id().with(("track_row_actions", index));
        let dots_rect =
            Rect::from_center_size(Pos2::new(rect.right() - 18.0, c_y), vec2(30.0, 30.0));
        let dots_response = ui.interact(dots_rect, row_id.with("more"), Sense::click());
        if dots_response.hovered() {
            ui.painter()
                .circle_filled(dots_rect.center(), 15.0, pal.bg_highlight);
        }
        draw_app_icon(
            ui.painter(),
            dots_rect.center(),
            AppIcon::More,
            18.0,
            if dots_response.hovered() {
                pal.text_bright
            } else {
                pal.text_subtle
            },
        );

        egui::Popup::menu(&dots_response)
            .align(egui::RectAlign::BOTTOM_END)
            .width(220.0)
            .show(|ui| {
                if draw_menu_action_row(ui, AppIcon::Queue, "Add to queue", pal.text_subtle, true)
                    .clicked()
                {
                    queue = true;
                    ui.close();
                }

                let like_label = if is_liked {
                    "Remove from Liked Songs"
                } else {
                    "Like song"
                };
                if draw_menu_action_row(ui, AppIcon::Heart, like_label, pal.green_accent, true)
                    .clicked()
                {
                    like = true;
                    ui.close();
                }

                let cache_label = if is_cached {
                    "Saved offline"
                } else {
                    "Save offline"
                };
                if draw_menu_action_row(
                    ui,
                    AppIcon::Download,
                    cache_label,
                    pal.text_subtle,
                    !is_cached,
                )
                .clicked()
                {
                    cache = true;
                    ui.close();
                }

                ui.separator();
                // Use egui's native submenu control here: a nested Popup::menu is closed by
                // the parent menu on the same click, which made the playlist submenu disappear.
                let playlist_button = egui::Button::new("     Add to playlist")
                    .min_size(vec2(ui.available_width(), 28.0));
                let (playlist_response, _) = egui::containers::menu::SubMenuButton::from_button(
                    playlist_button,
                )
                .ui(ui, |ui| {
                    if saved_playlists.is_empty() {
                        ui.label("No saved playlists yet.");
                    }
                    for (index, playlist) in saved_playlists.iter().enumerate() {
                        if ui.button(&playlist.name).clicked() {
                            playlist_idx = Some(index);
                            ui.close();
                        }
                    }
                });
                draw_app_icon(
                    ui.painter(),
                    Pos2::new(
                        playlist_response.rect.left() + 16.0,
                        playlist_response.rect.center().y,
                    ),
                    AppIcon::Playlist,
                    14.0,
                    pal.text_subtle,
                );
            });

        (play, queue, like, cache, playlist_idx)
    }
}

fn draw_menu_action_row(
    ui: &mut egui::Ui,
    icon: AppIcon,
    label: &str,
    icon_color: Color32,
    enabled: bool,
) -> egui::Response {
    let width = ui.available_width();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), sense);
    if response.hovered() && enabled {
        ui.painter()
            .rect_filled(rect, 4.0, ui.visuals().widgets.hovered.bg_fill);
    }
    let color = if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    draw_app_icon(
        ui.painter(),
        Pos2::new(rect.left() + 16.0, rect.center().y),
        icon,
        14.0,
        if enabled {
            icon_color
        } else {
            ui.visuals().weak_text_color()
        },
    );
    ui.painter().text(
        Pos2::new(rect.left() + 36.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        color,
    );
    response
}

fn draw_inline_icon(ui: &mut egui::Ui, icon: AppIcon, box_size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(box_size, box_size), Sense::hover());
    draw_app_icon(ui.painter(), rect.center(), icon, box_size * 0.72, color);
}

fn main() -> eframe::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        print!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SpotiFast")
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([720.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "SpotiFast",
        options,
        Box::new(|cc| {
            let app = PlayerApp::new(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}
