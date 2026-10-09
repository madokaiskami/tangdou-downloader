use std::io;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

use crate::error::{AppError, Result};
use crate::model::{TrimRange, format_time};

pub fn check_dependencies(needs_ffmpeg: bool) -> Result<()> {
    check_tool("ffprobe")?;
    if needs_ffmpeg {
        check_tool("ffmpeg")?;
    }
    Ok(())
}

fn check_tool(tool: &'static str) -> Result<()> {
    match media_command(tool).arg("-version").output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(tool_failure(tool, output)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(AppError::MissingTool(tool)),
        Err(source) => Err(AppError::ToolStart { tool, source }),
    }
}

pub fn probe_duration(input: &Path) -> Result<Duration> {
    let output = media_command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(input)
        .output()
        .map_err(|source| map_start_error("ffprobe", source))?;
    if !output.status.success() {
        return Err(tool_failure("ffprobe", output));
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let seconds: f64 = raw
        .parse()
        .map_err(|_| AppError::InvalidDuration(raw.clone()))?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return Err(AppError::InvalidDuration(raw));
    }
    Duration::try_from_secs_f64(seconds).map_err(|_| AppError::InvalidDuration(seconds.to_string()))
}

pub fn trim_mp4(input: &Path, output: &Path, trim: TrimRange) -> Result<()> {
    let mut command = media_command("ffmpeg");
    command.args(["-hide_banner", "-n"]);
    add_trim_arguments(&mut command, trim);
    command
        .arg("-i")
        .arg(input)
        .args(["-c", "copy"])
        .arg(output);
    run_ffmpeg(command)
}

pub fn convert_to_mp3(input: &Path, output: &Path, trim: TrimRange) -> Result<()> {
    let mut command = media_command("ffmpeg");
    command.args(["-hide_banner", "-n"]);
    add_trim_arguments(&mut command, trim);
    command
        .arg("-i")
        .arg(input)
        .args(["-vn", "-c:a", "libmp3lame", "-q:a", "2"])
        .arg(output);
    run_ffmpeg(command)
}

fn media_command(tool: &str) -> Command {
    let command = Command::new(tool);
    #[cfg(windows)]
    let command = {
        use std::os::windows::process::CommandExt;
        let mut command = command;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        command
    };
    command
}

fn add_trim_arguments(command: &mut Command, trim: TrimRange) {
    if let Some(start) = trim.start {
        command.arg("-ss").arg(format_time(start));
    }
    if let Some(end) = trim.end {
        command.arg("-to").arg(format_time(end));
    }
}

fn run_ffmpeg(mut command: Command) -> Result<()> {
    let output = command
        .output()
        .map_err(|source| map_start_error("ffmpeg", source))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(tool_failure("ffmpeg", output))
    }
}

fn map_start_error(tool: &'static str, source: io::Error) -> AppError {
    if source.kind() == io::ErrorKind::NotFound {
        AppError::MissingTool(tool)
    } else {
        AppError::ToolStart { tool, source }
    }
}

fn tool_failure(tool: &'static str, output: Output) -> AppError {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    AppError::ToolFailed {
        tool,
        status: output.status.code(),
        stderr,
    }
}
