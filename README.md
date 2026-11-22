# SpotiFast

SpotiFast is a Windows music player with YouTube search and playlists, a local music library, and playback through mpv.

## Download

Download and run `SpotiFast-Setup.exe` from the latest [GitHub Release](https://github.com/trulysweyn/SpotiFast/releases/latest). It installs SpotiFast and its updater helper, adds a Start menu shortcut, and offers an optional desktop shortcut. It installs per user and does not require administrator access.

The Settings page can check for SpotiFast, yt-dlp, and mpv updates. App updates replace the executable after SpotiFast closes and keep one previous version for rollback. Your playlists, settings, and downloaded songs stay under `%LOCALAPPDATA%\yt-playlist-gui`.

## Build

Install the stable Rust toolchain, then run:

```powershell
cargo build --release --bins
```

The app and update helper are created in `target\release`.

## Publish a release

Push a version tag such as `v0.5.5` to GitHub. The release workflow builds both Windows executables, packages them into `SpotiFast-Setup.exe`, and attaches all three files to a GitHub Release. The app's update source is compiled from the repository name automatically.
