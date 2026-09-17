# Tangdou Downloader

A lightweight Rust desktop application for downloading and processing Tangdou videos.

The intended first platform is Ubuntu/Linux.

The application is designed to:

- accept a Tangdou share link;
- extract the video ID;
- resolve the current signed media URL;
- download the original MP4;
- optionally trim by start/end time;
- optionally convert the output to MP3;
- provide the workflow through a simple native GUI.

> Use the application only for media that you are authorized to download and process.

---

## Phase 3 background worker and progress

The GUI sends all blocking metadata, download, `ffprobe`, and `ffmpeg` work to a `std::thread`
worker. Structured channel events report resolving, download start, byte progress, probing,
processing, completion, and failure. The window displays both a progress bar and human-readable
status such as downloaded MiB, total size, and percentage when the server supplies a content
length.

No Tokio runtime is used.

---

## Phase 2 desktop GUI

The native `eframe`/`egui` desktop interface is implemented. Launch it with:

```bash
cargo run
```

The GUI can parse a Tangdou URL, display its VID and title, select optional trim boundaries and
MP4/MP3 output, preserve the original MP4, choose an output directory, and run the complete media
workflow. Network requests, downloads, `ffprobe`, and `ffmpeg` run on a worker thread so the window
remains responsive. Phase 3 adds structured worker events and download progress reporting.

For a release build:

```bash
cargo build --release
./target/release/tangdou-downloader
```

---

## Phase 1 CLI

The command-line proof of concept is implemented. It resolves a fresh signed URL for each run,
downloads the complete MP4 with Tangdou request headers, probes its duration, and can either trim
the MP4 with stream copy or export MP3 audio.

Build and show the available options:

```bash
cargo build --release
./target/release/tangdou-cli --help
```

Download the original MP4 into the current directory:

```bash
cargo run --bin tangdou-cli -- 'https://www.tangdouddn.com/h5/play?vid=20000014175956'
```

Trim to MP4 (the output directory is created if needed):

```bash
cargo run --bin tangdou-cli -- \
  'https://www.tangdouddn.com/h5/play?vid=20000014175956' \
  --start 00:01:20 \
  --end 00:04:30 \
  --output ./downloads
```

Export a trimmed MP3 and preserve the originally downloaded MP4:

```bash
cargo run --bin tangdou-cli -- \
  'https://www.tangdouddn.com/h5/play?vid=20000014175956' \
  --format mp3 \
  --start 01:20 \
  --end 04:30 \
  --keep-original \
  --output ./downloads
```

`ffprobe` is required for every download. `ffmpeg` is additionally required for trimming and MP3
conversion. On Ubuntu, install both with `sudo apt install ffmpeg`.

Existing output files are not overwritten; numeric suffixes are selected automatically. MP4 trim
uses stream copy for low CPU use, so cut points can align to keyframes and are not frame-exact.

---

## Desktop interface

```text
┌──────────────────────────────────────────────────┐
│ Tangdou Downloader                               │
├──────────────────────────────────────────────────┤
│ Share URL                                        │
│ [ https://www.tangdouddn.com/h5/play?...      ] │
│                                                  │
│ [Parse]                                          │
│                                                  │
│ Title: ...                                       │
│ VID:   20000014175956                            │
│                                                  │
│ Trim                                             │
│ [ ] Enable trim                                  │
│                                                  │
│ Start [00:00:00]     End [00:03:30]              │
│                                                  │
│ Output                                           │
│ (o) MP4              ( ) MP3                     │
│                                                  │
│ [ ] Keep original video                          │
│                                                  │
│ Save directory                                   │
│ [/home/user/Videos/...................] [Browse] │
│                                                  │
│ [           Download / Convert           ]       │
│                                                  │
│ [=====================       ] 72%                │
│ Downloading...                                   │
└──────────────────────────────────────────────────┘
```

---

## Why Rust

Rust is a good fit for this utility because it provides:

- low runtime overhead;
- native binaries;
- predictable memory usage;
- straightforward process control;
- good cross-platform potential;
- strong error handling.

The application does not implement media codecs itself.

Media processing is delegated to FFmpeg.

---

## Architecture

The application consists of four main pieces:

```text
               ┌───────────────┐
               │     egui      │
               │      GUI      │
               └───────┬───────┘
                       │
                       v
               ┌───────────────┐
               │ Worker thread │
               └───────┬───────┘
                       │
          ┌────────────┼────────────┐
          v            v            v
     Tangdou API   HTTP download   FFmpeg
```

The GUI thread must never perform blocking downloads or media processing.

---

## Technology stack

Planned Rust dependencies:

- `eframe` / `egui`
- `reqwest`
- `serde`
- `serde_json`
- `url`

System dependencies:

- `ffmpeg`
- `ffprobe`

The initial implementation should avoid a full async runtime unless it becomes necessary.

---

## Tangdou resolution

Tangdou share links contain a video ID, for example:

```text
https://www.tangdouddn.com/h5/play?ad_switch=0&vid=20000014175956&...
```

The application extracts:

```text
20000014175956
```

A currently known metadata endpoint is:

```text
https://api-h5.tangdou.com/sample/share/main?vid={VID}
```

Known media fields may include:

```text
data.video_url
data.play_url
```

The resolved media URL may look similar to:

```text
https://aqiniushare.tangdou.com/..._H540P.mp4?sign=...&t=...
```

These URLs are signed and may expire.

The application should therefore resolve a fresh URL before starting each new download.

Known working requests may require headers such as:

```text
User-Agent: Mozilla/5.0 ...
Referer: https://www.tangdoucdn.com/
```

Tangdou can change its API at any time. Service-specific logic should remain isolated so it can be updated without changing the GUI or media-processing layers.

---

## Media workflow

Recommended processing path:

```text
Tangdou share URL
        |
        v
Extract VID
        |
        v
Resolve media URL
        |
        v
Download original MP4
        |
        v
Local temporary file
        |
        +--------------------------+
        |                          |
        v                          v
      MP4                     ffmpeg processing
                                   |
                              +----+----+
                              |         |
                              v         v
                         Trimmed MP4    MP3
```

Downloading the full media file first makes error handling easier and prevents an expiring signed URL from interrupting later processing.

---

## MP4 trimming

For low CPU use, the default trim mode can use stream copying:

```bash
ffmpeg \
  -ss 00:01:20 \
  -to 00:04:30 \
  -i input.mp4 \
  -c copy \
  output.mp4
```

Advantages:

- very fast;
- low CPU usage;
- no quality loss from re-encoding.

Limitation:

- cuts may align to keyframes and may not be frame-exact.

A future optional "precise trim" mode could re-encode the video.

---

## MP3 conversion

Example:

```bash
ffmpeg \
  -ss 00:01:20 \
  -to 00:04:30 \
  -i input.mp4 \
  -vn \
  -c:a libmp3lame \
  -q:a 2 \
  output.mp3
```

Without trimming:

```bash
ffmpeg \
  -i input.mp4 \
  -vn \
  -c:a libmp3lame \
  -q:a 2 \
  output.mp3
```

---

## Duration detection

Use `ffprobe` when media duration is needed:

```bash
ffprobe \
  -v error \
  -show_entries format=duration \
  -of default=noprint_wrappers=1:nokey=1 \
  input.mp4
```

The application can use this to validate that:

```text
start < end <= video duration
```

---

## Requirements

### Rust

Install Rust using rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then restart the shell or run:

```bash
source "$HOME/.cargo/env"
```

Check:

```bash
rustc --version
cargo --version
```

### Ubuntu build dependencies

```bash
sudo apt update
sudo apt install -y \
  build-essential \
  pkg-config \
  libssl-dev \
  ffmpeg
```

Verify FFmpeg:

```bash
ffmpeg -version
ffprobe -version
```

---

## Creating the project

```bash
cargo new tangdou-downloader
cd tangdou-downloader
```

Suggested layout:

```text
tangdou-downloader/
├── Cargo.toml
├── AGENTS.md
├── README.md
└── src/
    ├── main.rs
    ├── gui.rs
    ├── tangdou.rs
    ├── downloader.rs
    ├── ffmpeg.rs
    ├── model.rs
    └── error.rs
```

---

## Suggested Cargo dependencies

Use current compatible crate versions rather than copying stale pinned versions blindly.

Conceptually:

```toml
[dependencies]
eframe = "..."
reqwest = { version = "...", features = ["blocking", "json", "rustls-tls"] }
serde = { version = "...", features = ["derive"] }
serde_json = "..."
url = "..."
```

Optional development dependencies may be added for HTTP mocking or fixture tests.

---

## Development

Format:

```bash
cargo fmt
```

Lint:

```bash
cargo clippy --all-targets --all-features
```

Test:

```bash
cargo test
```

Run:

```bash
cargo run
```

Release build:

```bash
cargo build --release
```

The release binary will normally be:

```text
target/release/tangdou-downloader
```

---

## MVP scope

The initial release should include:

- Tangdou URL input;
- VID extraction;
- media URL resolution;
- MP4 download;
- output-directory selection;
- optional start time;
- optional end time;
- MP4 output;
- MP3 output;
- download status/progress;
- processing status;
- missing FFmpeg detection;
- useful error messages;
- safe temporary-file cleanup.

The initial release should not include:

- embedded video playback;
- waveform rendering;
- a graphical editing timeline;
- login/account support;
- download history database;
- multi-window UI;
- DRM bypass;
- browser automation.

---

## Proposed development order

### 1. CLI proof of concept

Implement a reusable Rust core that can:

```text
share URL
  -> VID
  -> API response
  -> signed video URL
  -> MP4 download
```

Then add:

```text
ffprobe
ffmpeg trim
ffmpeg MP3 export
```

Do not start with GUI-specific networking logic.

### 2. GUI shell

Implement:

- URL text field;
- Parse button;
- trim inputs;
- output format;
- save directory;
- Download button;
- status label.

### 3. Worker thread

Move network and FFmpeg jobs off the GUI thread.

Send progress/results back through channels.

### 4. Reliability

Add:

- structured error types;
- output collision handling;
- temporary-file cleanup;
- API fixture tests;
- time parser tests;
- URL parser tests.

### 5. Packaging

After the core application is stable, consider:

- standalone release binaries;
- `.deb`;
- AppImage.

---

## Example current manual flow

A Tangdou video ID can currently be resolved from Ubuntu with a request similar to:

```bash
VID=20000014175956

URL="$(
  curl -fsSL --compressed \
    -A 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/140 Safari/537.36' \
    -H 'Accept: application/json, text/plain, */*' \
    -H 'Referer: https://www.tangdoucdn.com/' \
    "https://api-h5.tangdou.com/sample/share/main?vid=${VID}" |
  jq -r '.data.video_url // .data.play_url // empty'
)"

echo "$URL"
```

The Rust application is intended to automate this workflow.

---

## Safety

Do not pass raw user input into `sh -c`.

Invoke FFmpeg using explicit process arguments.

Treat Tangdou titles/metadata as untrusted strings and sanitize output filenames.

Do not silently overwrite an existing user file.

Do not implement DRM circumvention or credential extraction.

---

## Contributing

Before considering a change complete:

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo test
```

Prefer small changes.

Keep Tangdou-specific code isolated from the GUI.

Avoid adding dependencies when the Rust standard library already provides a simple solution.

See [`AGENTS.md`](./AGENTS.md) for detailed implementation instructions intended for coding agents.
