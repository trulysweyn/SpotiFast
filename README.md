# SpotiFast

SpotiFast is a Windows music player with YouTube search and playlists, a local music library, and playback through mpv.

## Download

Download `SpotiFast.exe` and `SpotiFastUpdater.exe` from the latest [GitHub Release](https://github.com/trulysweyn/SpotiFast/releases/latest). Keep both files together in a folder you can write to.

The Settings page can check for SpotiFast, yt-dlp, and mpv updates. App updates replace the executable after SpotiFast closes and keep one previous version for rollback. Your playlists, settings, and downloaded songs stay under `%LOCALAPPDATA%\yt-playlist-gui`.

## Build

Install the stable Rust toolchain, then run:

```powershell
cargo build --release --bins
```

The app and update helper are created in `target\release`.

## Publish a release

Push a version tag such as `v0.5.2` to GitHub. The release workflow builds both Windows executables and attaches them to a GitHub Release. The app's update source is compiled from the repository name automatically.
