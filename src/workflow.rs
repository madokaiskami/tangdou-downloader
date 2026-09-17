use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tempfile::NamedTempFile;

use crate::downloader::download_to;
use crate::error::{AppError, Result};
use crate::ffmpeg;
use crate::model::{OutputFormat, TrimRange, sanitize_title};
use crate::tangdou;

#[derive(Clone, Debug)]
pub struct JobRequest {
    pub share_url: String,
    pub output_dir: PathBuf,
    pub format: OutputFormat,
    pub trim_enabled: bool,
    pub trim: TrimRange,
    pub keep_original: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedMetadata {
    pub vid: String,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobResult {
    pub output_path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub duration: Duration,
    pub downloaded_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerEvent {
    Resolving { vid: String },
    MetadataResolved(ResolvedMetadata),
    DownloadStarted,
    DownloadProgress { downloaded: u64, total: Option<u64> },
    Probing,
    Processing { format: OutputFormat },
    Parsed(ResolvedMetadata),
    Finished(JobResult),
    Failed(String),
}

pub fn resolve_metadata(share_url: &str) -> Result<ResolvedMetadata> {
    let vid = tangdou::extract_vid(share_url)?;
    resolve_metadata_for_vid(&vid)
}

fn resolve_metadata_for_vid(vid: &str) -> Result<ResolvedMetadata> {
    let client = tangdou::build_http_client()?;
    let media = tangdou::resolve_media(&client, vid)?;
    Ok(ResolvedMetadata {
        vid: media.vid,
        title: media.title,
    })
}

pub fn run_parse<F>(share_url: &str, mut on_event: F) -> Result<ResolvedMetadata>
where
    F: FnMut(WorkerEvent),
{
    let result = tangdou::extract_vid(share_url).and_then(|vid| {
        on_event(WorkerEvent::Resolving { vid: vid.clone() });
        resolve_metadata_for_vid(&vid)
    });

    match result {
        Ok(metadata) => {
            on_event(WorkerEvent::Parsed(metadata.clone()));
            Ok(metadata)
        }
        Err(error) => {
            on_event(WorkerEvent::Failed(error.to_string()));
            Err(error)
        }
    }
}

pub fn run_job<F>(request: JobRequest, mut on_event: F) -> Result<JobResult>
where
    F: FnMut(WorkerEvent),
{
    let result = execute_job(request, &mut on_event);
    match result {
        Ok(result) => {
            on_event(WorkerEvent::Finished(result.clone()));
            Ok(result)
        }
        Err(error) => {
            on_event(WorkerEvent::Failed(error.to_string()));
            Err(error)
        }
    }
}

fn execute_job<F>(request: JobRequest, on_event: &mut F) -> Result<JobResult>
where
    F: FnMut(WorkerEvent),
{
    let trim = if request.trim_enabled {
        request.trim
    } else {
        TrimRange::default()
    };
    let vid = tangdou::extract_vid(&request.share_url)?;
    on_event(WorkerEvent::Resolving { vid: vid.clone() });

    prepare_output_directory(&request.output_dir)?;
    let needs_ffmpeg = request.format == OutputFormat::Mp3 || request.trim_enabled;
    ffmpeg::check_dependencies(needs_ffmpeg)?;

    let client = tangdou::build_http_client()?;
    let media = tangdou::resolve_media(&client, &vid)?;
    on_event(WorkerEvent::MetadataResolved(ResolvedMetadata {
        vid: media.vid.clone(),
        title: media.title.clone(),
    }));

    let base_name = media
        .title
        .as_deref()
        .and_then(sanitize_title)
        .unwrap_or_else(|| media.vid.clone());
    let final_stem = if request.trim_enabled {
        format!("{base_name}_trimmed")
    } else {
        base_name.clone()
    };
    let final_path =
        next_available_path(&request.output_dir, &final_stem, request.format.extension())?;

    let mut temporary =
        NamedTempFile::new_in(&request.output_dir).map_err(|source| AppError::Io {
            operation: "creating temporary download in",
            path: request.output_dir.clone(),
            source,
        })?;
    let temporary_path = temporary.path().to_owned();
    on_event(WorkerEvent::DownloadStarted);
    let mut last_reported = 0_u64;
    let mut download_total = None;
    let downloaded_bytes = download_to(
        &client,
        &media.source,
        &temporary_path,
        temporary.as_file_mut(),
        |progress| {
            download_total = progress.total;
            let reached_end = progress.total == Some(progress.downloaded);
            if progress.downloaded == 0
                || reached_end
                || progress.downloaded.saturating_sub(last_reported) >= 256 * 1024
            {
                on_event(WorkerEvent::DownloadProgress {
                    downloaded: progress.downloaded,
                    total: progress.total,
                });
                last_reported = progress.downloaded;
            }
        },
    )?;
    if downloaded_bytes != last_reported {
        on_event(WorkerEvent::DownloadProgress {
            downloaded: downloaded_bytes,
            total: download_total,
        });
    }

    on_event(WorkerEvent::Probing);
    let duration = ffmpeg::probe_duration(&temporary_path)?;
    trim.validate(duration)?;

    if request.format == OutputFormat::Mp4 && !request.trim_enabled {
        persist_download(temporary, &final_path)?;
        return Ok(JobResult {
            output_path: final_path,
            original_path: None,
            duration,
            downloaded_bytes,
        });
    }

    let mut original_path = None;
    let input_path = if request.keep_original {
        let path =
            next_available_path(&request.output_dir, &format!("{base_name}_original"), "mp4")?;
        persist_download(temporary, &path)?;
        original_path = Some(path.clone());
        path
    } else {
        temporary_path
    };

    on_event(WorkerEvent::Processing {
        format: request.format,
    });
    match request.format {
        OutputFormat::Mp4 => ffmpeg::trim_mp4(&input_path, &final_path, trim)?,
        OutputFormat::Mp3 => ffmpeg::convert_to_mp3(&input_path, &final_path, trim)?,
    }

    Ok(JobResult {
        output_path: final_path,
        original_path,
        duration,
        downloaded_bytes,
    })
}

fn prepare_output_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|source| AppError::Io {
        operation: "creating output directory",
        path: path.to_owned(),
        source,
    })?;
    if !path.is_dir() {
        return Err(AppError::Arguments(format!(
            "output path '{}' is not a directory",
            path.display()
        )));
    }
    Ok(())
}

fn next_available_path(directory: &Path, stem: &str, extension: &str) -> Result<PathBuf> {
    for suffix in 0..=9999 {
        let filename = if suffix == 0 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem}_{suffix}.{extension}")
        };
        let candidate = directory.join(filename);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(AppError::Io {
        operation: "finding an unused output filename in",
        path: directory.to_owned(),
        source: std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "all filename suffixes from 1 through 9999 are already used",
        ),
    })
}

fn persist_download(temporary: NamedTempFile, destination: &Path) -> Result<()> {
    temporary
        .persist_noclobber(destination)
        .map(|_| ())
        .map_err(|error| AppError::Io {
            operation: "saving downloaded MP4 to",
            path: destination.to_owned(),
            source: error.error,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_collision_free_output_path() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        fs::write(directory.path().join("video.mp4"), b"existing")
            .expect("fixture should be written");
        assert_eq!(
            next_available_path(directory.path(), "video", "mp4").ok(),
            Some(directory.path().join("video_1.mp4"))
        );
    }

    #[test]
    fn invalid_job_emits_structured_failure() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let request = JobRequest {
            share_url: "not a URL".to_owned(),
            output_dir: directory.path().to_owned(),
            format: OutputFormat::Mp4,
            trim_enabled: false,
            trim: TrimRange::default(),
            keep_original: false,
        };
        let mut events = Vec::new();

        assert!(run_job(request, |event| events.push(event)).is_err());
        assert_eq!(events.len(), 1);
        assert!(matches!(events.first(), Some(WorkerEvent::Failed(_))));
    }

    #[test]
    fn invalid_parse_emits_structured_failure() {
        let mut events = Vec::new();
        assert!(run_parse("not a URL", |event| events.push(event)).is_err());
        assert_eq!(events.len(), 1);
        assert!(matches!(events.first(), Some(WorkerEvent::Failed(_))));
    }
}
