use std::fmt;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug)]
pub enum AppError {
    Arguments(String),
    InvalidShareUrl(String),
    UnsupportedTangdouHost(String),
    MissingVid,
    DuplicateVid,
    InvalidVid(String),
    HttpRequest {
        operation: &'static str,
        source: reqwest::Error,
    },
    HttpStatus {
        operation: &'static str,
        status: reqwest::StatusCode,
    },
    InvalidApiResponse(String),
    MissingMediaUrl(String),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    MissingTool(&'static str),
    ToolStart {
        tool: &'static str,
        source: std::io::Error,
    },
    ToolFailed {
        tool: &'static str,
        status: Option<i32>,
        stderr: String,
    },
    InvalidDuration(String),
    InvalidTime(String),
    InvalidTrim(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Arguments(message) => write!(f, "{message}"),
            Self::InvalidShareUrl(reason) => write!(f, "invalid Tangdou share URL: {reason}"),
            Self::UnsupportedTangdouHost(host) => {
                write!(f, "URL host is not a recognized Tangdou domain: {host}")
            }
            Self::MissingVid => write!(f, "Tangdou share URL has no 'vid' query parameter"),
            Self::DuplicateVid => write!(
                f,
                "Tangdou share URL contains more than one 'vid' parameter"
            ),
            Self::InvalidVid(vid) => {
                write!(f, "invalid Tangdou video ID '{vid}': expected digits only")
            }
            Self::HttpRequest { operation, source } => write!(f, "{operation} failed: {source}"),
            Self::HttpStatus { operation, status } => {
                write!(f, "{operation} returned HTTP status {status}")
            }
            Self::InvalidApiResponse(reason) => {
                write!(f, "Tangdou returned an unexpected API response: {reason}")
            }
            Self::MissingMediaUrl(context) => {
                write!(
                    f,
                    "Tangdou response contains neither data.video_url nor data.play_url ({context})"
                )
            }
            Self::Io {
                operation,
                path,
                source,
            } => write!(f, "{operation} '{}': {source}", path.display()),
            Self::MissingTool(tool) => write!(
                f,
                "required program '{tool}' was not found; on Ubuntu install it with: sudo apt install ffmpeg"
            ),
            Self::ToolStart { tool, source } => write!(f, "could not start {tool}: {source}"),
            Self::ToolFailed {
                tool,
                status,
                stderr,
            } => {
                let status = status.map_or_else(|| "signal".to_owned(), |code| code.to_string());
                write!(f, "{tool} failed (exit {status})")?;
                if !stderr.is_empty() {
                    write!(f, ": {stderr}")?;
                }
                Ok(())
            }
            Self::InvalidDuration(value) => {
                write!(f, "ffprobe returned an invalid duration: {value}")
            }
            Self::InvalidTime(value) => write!(
                f,
                "invalid time '{value}': use HH:MM:SS or MM:SS with non-negative whole numbers"
            ),
            Self::InvalidTrim(reason) => write!(f, "invalid trim range: {reason}"),
        }
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::HttpRequest { source, .. } => Some(source),
            Self::Io { source, .. } | Self::ToolStart { source, .. } => Some(source),
            _ => None,
        }
    }
}
