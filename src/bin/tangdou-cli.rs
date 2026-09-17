use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use tangdou_downloader::downloader::DownloadProgress;
use tangdou_downloader::model::{OutputFormat, TrimRange, format_time, parse_time};
use tangdou_downloader::workflow::{JobRequest, WorkerEvent, run_job};
use tangdou_downloader::{AppError, Result};

const USAGE: &str = "Tangdou Downloader (Phase 1 CLI)

Usage:
  tangdou-cli <SHARE_URL> [OPTIONS]

Options:
  -o, --output <DIRECTORY>  Save directory (default: current directory)
      --format <mp4|mp3>    Output format (default: mp4)
      --start <HH:MM:SS>    Optional trim start; MM:SS is also accepted
      --end <HH:MM:SS>      Optional trim end; MM:SS is also accepted
      --keep-original       Keep the downloaded original when processing
  -h, --help                Print this help

MP4 trimming uses stream copy, so cuts can align to keyframes rather than being frame-exact.";

#[derive(Debug)]
struct Cli {
    share_url: String,
    output_dir: PathBuf,
    format: OutputFormat,
    trim: TrimRange,
    keep_original: bool,
}

enum ParsedCli {
    Help,
    Run(Cli),
}

fn main() {
    match parse_cli(env::args_os().skip(1).collect()).and_then(|parsed| match parsed {
        ParsedCli::Help => {
            println!("{USAGE}");
            Ok(())
        }
        ParsedCli::Run(cli) => run(cli),
    }) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("Error: {error}");
            eprintln!("\nRun with --help for usage.");
            std::process::exit(1);
        }
    }
}

fn parse_cli(arguments: Vec<OsString>) -> Result<ParsedCli> {
    if arguments
        .iter()
        .any(|argument| argument == "-h" || argument == "--help")
    {
        return Ok(ParsedCli::Help);
    }

    let mut share_url = None;
    let mut output_dir = None;
    let mut format = None;
    let mut start = None;
    let mut end = None;
    let mut keep_original = false;
    let mut index = 0;

    while index < arguments.len() {
        let argument = arguments[index]
            .to_str()
            .ok_or_else(|| AppError::Arguments("arguments must be valid UTF-8".to_owned()))?;
        match argument {
            "-o" | "--output" => {
                ensure_not_set(&output_dir, "--output")?;
                index += 1;
                output_dir = Some(PathBuf::from(value_after(&arguments, index, argument)?));
            }
            "--format" => {
                ensure_not_set(&format, "--format")?;
                index += 1;
                let value = value_after(&arguments, index, argument)?;
                format = Some(match value {
                    "mp4" => OutputFormat::Mp4,
                    "mp3" => OutputFormat::Mp3,
                    _ => {
                        return Err(AppError::Arguments(format!(
                            "unsupported output format '{value}'; use mp4 or mp3"
                        )));
                    }
                });
            }
            "--start" => {
                ensure_not_set(&start, "--start")?;
                index += 1;
                start = Some(parse_time(value_after(&arguments, index, argument)?)?);
            }
            "--end" => {
                ensure_not_set(&end, "--end")?;
                index += 1;
                end = Some(parse_time(value_after(&arguments, index, argument)?)?);
            }
            "--keep-original" => {
                if keep_original {
                    return Err(AppError::Arguments(
                        "option '--keep-original' was supplied more than once".to_owned(),
                    ));
                }
                keep_original = true;
            }
            value if value.starts_with('-') => {
                return Err(AppError::Arguments(format!("unknown option '{value}'")));
            }
            value => {
                if share_url.replace(value.to_owned()).is_some() {
                    return Err(AppError::Arguments(
                        "only one Tangdou share URL may be supplied".to_owned(),
                    ));
                }
            }
        }
        index += 1;
    }

    let share_url = share_url
        .ok_or_else(|| AppError::Arguments("missing Tangdou share URL argument".to_owned()))?;
    let output_dir = match output_dir {
        Some(path) => path,
        None => env::current_dir().map_err(|source| AppError::Io {
            operation: "reading current directory",
            path: PathBuf::from("."),
            source,
        })?,
    };

    Ok(ParsedCli::Run(Cli {
        share_url,
        output_dir,
        format: format.unwrap_or(OutputFormat::Mp4),
        trim: TrimRange { start, end },
        keep_original,
    }))
}

fn value_after<'a>(arguments: &'a [OsString], index: usize, option: &str) -> Result<&'a str> {
    arguments
        .get(index)
        .ok_or_else(|| AppError::Arguments(format!("option '{option}' requires a value")))?
        .to_str()
        .ok_or_else(|| AppError::Arguments(format!("value for '{option}' must be valid UTF-8")))
}

fn ensure_not_set<T>(value: &Option<T>, option: &str) -> Result<()> {
    if value.is_some() {
        Err(AppError::Arguments(format!(
            "option '{option}' was supplied more than once"
        )))
    } else {
        Ok(())
    }
}

fn run(cli: Cli) -> Result<()> {
    let mut progress = ConsoleProgress::default();
    let result = run_job(
        JobRequest {
            share_url: cli.share_url,
            output_dir: cli.output_dir,
            format: cli.format,
            trim_enabled: cli.trim.is_enabled(),
            trim: cli.trim,
            keep_original: cli.keep_original,
        },
        |event| match event {
            WorkerEvent::Resolving { vid } => {
                println!("VID: {vid}");
                println!("Resolving current Tangdou media URL...");
            }
            WorkerEvent::MetadataResolved(metadata) => {
                if let Some(title) = metadata.title {
                    println!("Title: {title}");
                }
            }
            WorkerEvent::DownloadStarted => println!("Downloading original MP4..."),
            WorkerEvent::DownloadProgress { downloaded, total } => {
                progress.update(DownloadProgress { downloaded, total });
            }
            WorkerEvent::Probing => println!("Probing media duration..."),
            WorkerEvent::Processing { format } => match format {
                OutputFormat::Mp4 => println!("Trimming MP4 with stream copy..."),
                OutputFormat::Mp3 => println!("Converting audio to MP3..."),
            },
            WorkerEvent::Parsed(_) | WorkerEvent::Finished(_) | WorkerEvent::Failed(_) => {}
        },
    )?;

    eprintln!("Downloaded {} bytes.", result.downloaded_bytes);
    println!("Duration: {}", format_duration(result.duration));
    println!(
        "Saved {}: {}",
        cli.format.extension().to_uppercase(),
        result.output_path.display()
    );
    if let Some(path) = result.original_path {
        println!("Kept original MP4: {}", path.display());
    }
    Ok(())
}

fn format_duration(duration: Duration) -> String {
    let base = format_time(duration);
    if duration.subsec_millis() == 0 {
        base
    } else {
        format!("{base}.{:03}", duration.subsec_millis())
    }
}

#[derive(Default)]
struct ConsoleProgress {
    last_percentage: Option<u64>,
    next_byte_report: u64,
}

impl ConsoleProgress {
    fn update(&mut self, progress: DownloadProgress) {
        if let Some(total) = progress.total.filter(|total| *total > 0) {
            let percentage = progress.downloaded.saturating_mul(100) / total;
            if self
                .last_percentage
                .is_none_or(|last| percentage >= last.saturating_add(5) || percentage == 100)
            {
                eprintln!("Download: {percentage}%");
                self.last_percentage = Some(percentage);
            }
        } else if progress.downloaded >= self.next_byte_report {
            eprintln!("Download: {} MiB", progress.downloaded / (1024 * 1024));
            self.next_byte_report = progress.downloaded.saturating_add(5 * 1024 * 1024);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_cli_options() {
        let parsed = parse_cli(strings(&[
            "https://www.tangdou.com/play?vid=123",
            "--format",
            "mp3",
            "--start",
            "01:30",
            "--end",
            "00:03:00",
            "--output",
            "/tmp/videos",
            "--keep-original",
        ]));
        let Ok(ParsedCli::Run(cli)) = parsed else {
            panic!("CLI should parse");
        };
        assert_eq!(cli.format, OutputFormat::Mp3);
        assert_eq!(cli.trim.start, Some(Duration::from_secs(90)));
        assert_eq!(cli.trim.end, Some(Duration::from_secs(180)));
        assert_eq!(cli.output_dir, PathBuf::from("/tmp/videos"));
        assert!(cli.keep_original);
    }

    #[test]
    fn rejects_unknown_and_duplicate_options() {
        assert!(parse_cli(strings(&["url", "--unknown"])).is_err());
        assert!(parse_cli(strings(&["url", "--format", "mp4", "--format", "mp3"])).is_err());
    }
}
