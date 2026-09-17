# AGENTS.md

## Project

**Tangdou Downloader**

A lightweight Rust desktop application for Ubuntu/Linux that:

- accepts a Tangdou share URL;
- extracts the Tangdou video ID (`vid`);
- resolves the current signed media URL;
- downloads the original MP4;
- optionally trims the media by start/end time;
- optionally converts the result to MP3;
- exposes the workflow through a simple, low-overhead native GUI.

The primary target is Ubuntu. Keep the application simple, maintainable, and low-resource.

---

## Development priorities

Optimize for the following, in this order:

1. Correctness.
2. Reliability when Tangdou URLs/API responses change.
3. Simple implementation.
4. Low idle CPU and memory usage.
5. Clear error handling.
6. Maintainable module boundaries.
7. Minimal dependency count.

Do not add architectural complexity unless required by an implemented feature.

---

## Recommended stack

Use:

- Rust stable
- `eframe` / `egui` for the native GUI
- `reqwest` with blocking HTTP where practical
- `serde` / `serde_json` for API responses
- `url` for URL parsing
- `std::thread` for background work
- `std::sync::mpsc` or another small standard-library channel for GUI/worker communication
- system `ffmpeg` / `ffprobe` executables for media operations

Avoid introducing Tokio unless asynchronous concurrency becomes clearly necessary.

Do not implement audio/video codecs in Rust.

Do not bind directly to libav/FFmpeg libraries in the initial versions.

---

## External runtime dependencies

The application may depend on:

```bash
ffmpeg
ffprobe
```

Ubuntu installation:

```bash
sudo apt install ffmpeg
```

At startup, or before the first media operation, detect whether `ffmpeg` and `ffprobe` are available.

If missing, display an actionable GUI error instead of crashing.

---

## Core workflow

The expected workflow is:

```text
Tangdou share URL
        |
        v
Extract vid
        |
        v
Resolve current Tangdou API response
        |
        v
Extract video_url / play_url
        |
        v
Download original MP4 to temporary/local file
        |
        +--------------------+
        |                    |
        v                    v
     Keep MP4          Process with ffmpeg
                             |
                     +-------+-------+
                     |               |
                     v               v
                  Trim MP4       Export MP3
```

Signed Tangdou URLs must be treated as temporary.

Do not persist signed media URLs as permanent application state.

Resolve the media URL again when starting a new download.

---

## Tangdou URL handling

The application must accept URLs similar to:

```text
https://www.tangdouddn.com/h5/play?...&vid=20000014175956&...
```

Extract the `vid` query parameter.

Never rely on fixed string slicing when standard URL parsing can be used.

Example target API:

```text
https://api-h5.tangdou.com/sample/share/main?vid={VID}
```

Current known response fields may include:

```text
data.video_url
data.play_url
```

Parsing must tolerate one of these fields being absent.

Prefer:

1. `data.video_url`
2. `data.play_url`

If neither exists, return a structured error containing enough context for debugging.

Tangdou may change this API. Keep all Tangdou-specific behavior isolated in `tangdou.rs`.

---

## HTTP request requirements

Known working requests use headers comparable to:

```text
User-Agent: Mozilla/5.0 ...
Accept: application/json, text/plain, */*
Referer: https://www.tangdoucdn.com/
```

Video downloads should also include a valid Tangdou referer.

Do not assume that a resolved media URL can always be downloaded without headers.

Use redirect following.

Expose HTTP failures as useful messages, including HTTP status where available.

Do not log signed URLs unnecessarily in normal GUI mode.

A debug mode may log them.

---

## Suggested source layout

Use a small module layout:

```text
src/
├── main.rs
├── gui.rs
├── tangdou.rs
├── downloader.rs
├── ffmpeg.rs
├── model.rs
└── error.rs
```

Responsibilities:

### `main.rs`

- application bootstrap;
- eframe startup;
- minimal configuration.

### `gui.rs`

- input controls;
- validation messages;
- progress display;
- task-state transitions;
- save-path selection;
- no blocking network/media work.

### `tangdou.rs`

- parse share URL;
- extract `vid`;
- call Tangdou API;
- decode response;
- return resolved metadata/media URL.

All service-specific behavior belongs here.

### `downloader.rs`

- HTTP media download;
- progress tracking;
- temporary-file creation;
- cancellation support if implemented.

### `ffmpeg.rs`

- probe duration;
- build ffmpeg command lines;
- trim MP4;
- convert to MP3;
- parse process status/progress where practical.

### `model.rs`

Shared data types, for example:

```rust
struct MediaInfo { ... }
struct TrimRange { ... }
enum OutputFormat { Mp4, Mp3 }
enum JobStatus { ... }
```

### `error.rs`

Application-specific error enum and user-facing error formatting.

---

## GUI requirements

The first release should use one simple window.

Suggested controls:

```text
Tangdou URL
[........................................]

[Parse]

Title: ...
VID: ...

Trim
[ ] Enable trim

Start: [00:00:00]
End:   [00:00:00]

Output
(o) MP4
( ) MP3

[ ] Keep original downloaded MP4

Save directory
[/path/to/output..................] [Browse]

[Download / Convert]

[====================      ] 72%

Status text
```

Do not add:

- embedded video playback;
- waveform rendering;
- timeline editors;
- user accounts;
- databases;
- download history;
- browser automation;
- multi-window UI;

unless explicitly requested later.

---

## GUI threading rules

Never perform blocking HTTP downloads or ffmpeg processing on the GUI thread.

Use a worker thread.

Suggested flow:

```text
GUI
 |
 | JobCommand
 v
Worker thread
 |
 |--- resolve URL
 |--- download
 |--- ffprobe
 |--- ffmpeg
 |
 | ProgressEvent / Result
 v
GUI
```

The GUI should remain responsive throughout downloads and conversions.

The worker should send structured events rather than directly mutating GUI state.

Example events:

```rust
enum WorkerEvent {
    Resolving,
    DownloadStarted,
    DownloadProgress { downloaded: u64, total: Option<u64> },
    Processing,
    Finished(PathBuf),
    Failed(String),
}
```

---

## Time input

Initial UI should accept:

```text
HH:MM:SS
```

Optional support for:

```text
MM:SS
```

is acceptable.

Convert entered values into a duration type internally.

Validation rules:

- start >= 0;
- end > start when end is supplied;
- end <= media duration when duration is known.

An empty start means the beginning of the file.

An empty end means the end of the file.

Do not silently reinterpret invalid input.

---

## ffprobe usage

Use `ffprobe` to determine media duration when needed.

Example:

```bash
ffprobe \
  -v error \
  -show_entries format=duration \
  -of default=noprint_wrappers=1:nokey=1 \
  input.mp4
```

Parse the numeric duration defensively.

---

## MP4 trimming

The low-resource/default trimming path should prefer stream copy when acceptable:

```bash
ffmpeg \
  -ss START \
  -to END \
  -i input.mp4 \
  -c copy \
  output.mp4
```

This avoids re-encoding and minimizes CPU use.

Document that stream-copy trimming may align around keyframes and is not guaranteed to be frame-exact.

If a future "precise trim" option is implemented, re-encode explicitly rather than silently changing behavior.

Example future precise path:

```bash
ffmpeg \
  -ss START \
  -to END \
  -i input.mp4 \
  -c:v libx264 \
  -preset veryfast \
  -c:a aac \
  output.mp4
```

---

## MP3 conversion

For MP3 output, use ffmpeg.

Example:

```bash
ffmpeg \
  -ss START \
  -to END \
  -i input.mp4 \
  -vn \
  -c:a libmp3lame \
  -q:a 2 \
  output.mp3
```

If no trim is enabled, omit `-ss` and `-to`.

Do not create intermediate WAV files.

---

## Temporary files

Preferred strategy:

```text
download signed remote MP4
        |
        v
temporary/original local MP4
        |
        v
ffmpeg processing
        |
        v
final output
```

Use a temporary directory or an application-specific cache directory.

After a successful conversion:

- delete temporary original files by default;
- preserve them if "Keep original" is selected.

On failure, preserve enough state for useful diagnosis when reasonable.

Never delete a user's pre-existing file.

---

## Output naming

Prefer a sanitized Tangdou title when available.

Fallback:

```text
{vid}.mp4
{vid}.mp3
```

If trimmed:

```text
{title}_trimmed.mp4
{title}_trimmed.mp3
```

Sanitize filenames for Linux filesystem compatibility.

If the destination already exists, do not overwrite silently.

Prefer:

```text
name.mp4
name_1.mp4
name_2.mp4
```

or explicitly ask/confirm through the GUI.

---

## Error handling

No `unwrap()` or `expect()` in normal runtime paths unless an invariant is truly internal and proven.

Expected errors include:

- invalid share URL;
- missing `vid`;
- Tangdou API unavailable;
- unexpected JSON response;
- missing media URL;
- signed URL expired;
- network interrupted;
- output directory not writable;
- disk full;
- ffmpeg missing;
- ffprobe missing;
- malformed time input;
- end <= start;
- trim exceeds duration;
- ffmpeg process failure.

Return errors to the GUI and render concise actionable messages.

Debug details may be logged separately.

---

## Security and privacy

Do not execute user-provided shell strings.

Use `std::process::Command` with explicit arguments.

Incorrect:

```rust
Command::new("sh")
    .arg("-c")
    .arg(format!("ffmpeg {}", user_input));
```

Preferred:

```rust
Command::new("ffmpeg")
    .args([...])
```

Treat remote titles and metadata as untrusted input.

Sanitize filenames.

Do not store browsing cookies, account credentials, or authentication tokens.

This project is intended for media the user is authorized to download and process.

Do not implement DRM circumvention.

---

## Logging

Keep default logging concise.

Useful levels:

- INFO: job started/completed;
- WARN: recoverable API/media issues;
- ERROR: failed job;
- DEBUG: response structures and command details.

Avoid logging full signed media URLs at INFO level.

---

## Testing

At minimum, write unit tests for:

### URL parsing

Valid Tangdou link:

```text
...?vid=20000014175956&...
```

Expected:

```text
20000014175956
```

Cases:

- missing `vid`;
- malformed URL;
- unrelated URL;
- duplicate query parameters.

### Time parsing

Test:

```text
00:00:00
00:01:30
01:02:03
90:00
invalid
-1
```

### Filename sanitization

Test:

- slash;
- null-like/invalid characters;
- long title;
- whitespace-only title.

### Tangdou JSON parsing

Use saved fixture JSON rather than relying on the live API in unit tests.

Live API tests must be opt-in integration tests.

---

## Development phases

### Phase 1 — CLI proof of concept

Before GUI work, ensure Rust can:

1. accept a share URL;
2. extract VID;
3. resolve media URL;
4. download MP4;
5. invoke ffprobe;
6. trim with ffmpeg;
7. convert to MP3.

Keep this code reusable by the GUI.

### Phase 2 — Minimal GUI

Implement:

- URL input;
- Parse button;
- metadata display;
- start/end fields;
- MP4/MP3 selection;
- save directory;
- run button;
- status text.

### Phase 3 — Background worker

Move blocking work off the GUI thread.

Implement progress events.

### Phase 4 — Reliability

Add:

- robust error types;
- temporary-file cleanup;
- output collision handling;
- API fixture tests;
- missing ffmpeg detection.

### Phase 5 — Packaging

Target Ubuntu first.

Possible formats:

- standalone executable;
- `.deb`;
- AppImage.

Do not make packaging block core application development.

---

## MVP acceptance criteria

The MVP is complete when all of the following work on Ubuntu:

1. The application launches as a native GUI.
2. A Tangdou share URL can be pasted.
3. The VID is extracted.
4. The current signed media URL is resolved.
5. The complete MP4 is downloaded successfully.
6. The GUI stays responsive during download.
7. Start/end trim values are optional.
8. MP4 output works.
9. MP3 output works.
10. ffmpeg errors are surfaced to the user.
11. Missing ffmpeg is detected.
12. Temporary files are cleaned after success.
13. Existing output files are not silently overwritten.

---

## Agent behavior

When modifying this repository:

1. Inspect existing code before introducing new abstractions.
2. Preserve the module boundaries described above unless there is a concrete reason not to.
3. Make the smallest coherent change that completes the requested task.
4. Run `cargo fmt`.
5. Run `cargo clippy --all-targets --all-features`.
6. Run `cargo test`.
7. Report any commands that could not be completed.
8. Do not claim a feature works unless it was compiled/tested where practical.
9. Do not replace working code merely for stylistic preference.
10. Avoid speculative features.

When Tangdou behavior changes, update only the Tangdou adapter where possible.

Prefer testable pure functions for parsing and validation logic.

---

## Suggested initial commands

Create the project:

```bash
cargo new tangdou-downloader
cd tangdou-downloader
```

Install system dependencies:

```bash
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev ffmpeg
```

During development:

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo test
cargo run
```

---

## Definition of done for each change

A change is done when:

- code compiles;
- formatting passes;
- relevant tests pass;
- errors are handled without panic;
- GUI work does not block the main thread;
- behavior is documented when user-visible;
- no unrelated dependencies/features were introduced.
