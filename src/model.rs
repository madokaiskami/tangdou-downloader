use std::time::Duration;

use url::Url;

pub struct DownloadSource {
    pub url: Url,
    pub referer: String,
    pub accept: String,
}

pub struct MediaInfo {
    pub vid: String,
    pub title: Option<String>,
    pub source: DownloadSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Mp4,
    Mp3,
}

impl OutputFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mp3 => "mp3",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TrimRange {
    pub start: Option<Duration>,
    pub end: Option<Duration>,
}

impl TrimRange {
    pub fn is_enabled(self) -> bool {
        self.start.is_some() || self.end.is_some()
    }

    pub fn validate(self, media_duration: Duration) -> crate::Result<()> {
        let start = self.start.unwrap_or_default();
        let end = self.end.unwrap_or(media_duration);

        if end <= start {
            return Err(crate::AppError::InvalidTrim(
                "end must be later than start".to_owned(),
            ));
        }
        if start >= media_duration {
            return Err(crate::AppError::InvalidTrim(format!(
                "start ({}) must be before media duration ({})",
                format_time(start),
                format_time(media_duration)
            )));
        }
        if end > media_duration {
            return Err(crate::AppError::InvalidTrim(format!(
                "end ({}) exceeds media duration ({})",
                format_time(end),
                format_time(media_duration)
            )));
        }
        Ok(())
    }
}

pub fn parse_time(value: &str) -> crate::Result<Duration> {
    if value.is_empty() || value.starts_with('-') {
        return Err(crate::AppError::InvalidTime(value.to_owned()));
    }

    let fields: Vec<&str> = value.split(':').collect();
    let (hours, minutes, seconds) = match fields.as_slice() {
        [minutes, seconds] => (
            0,
            parse_component(minutes, value)?,
            parse_component(seconds, value)?,
        ),
        [hours, minutes, seconds] => (
            parse_component(hours, value)?,
            parse_component(minutes, value)?,
            parse_component(seconds, value)?,
        ),
        _ => return Err(crate::AppError::InvalidTime(value.to_owned())),
    };

    if seconds >= 60 || (fields.len() == 3 && minutes >= 60) {
        return Err(crate::AppError::InvalidTime(value.to_owned()));
    }

    let total = hours
        .checked_mul(3600)
        .and_then(|total| {
            minutes
                .checked_mul(60)
                .and_then(|minutes| total.checked_add(minutes))
        })
        .and_then(|total| total.checked_add(seconds))
        .ok_or_else(|| crate::AppError::InvalidTime(value.to_owned()))?;

    Ok(Duration::from_secs(total))
}

fn parse_component(component: &str, complete: &str) -> crate::Result<u64> {
    if component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(crate::AppError::InvalidTime(complete.to_owned()));
    }
    component
        .parse()
        .map_err(|_| crate::AppError::InvalidTime(complete.to_owned()))
}

pub fn format_time(duration: Duration) -> String {
    let total = duration.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

pub fn sanitize_title(title: &str) -> Option<String> {
    const MAX_BYTES: usize = 120;

    let mut sanitized = String::new();
    let mut previous_was_space = false;

    for character in title.trim().chars() {
        let replacement = match character {
            '/' | '\\' | '\0' | '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            character if character.is_control() => '_',
            character => character,
        };

        let replacement = if replacement.is_whitespace() {
            if previous_was_space {
                continue;
            }
            previous_was_space = true;
            ' '
        } else {
            previous_was_space = false;
            replacement
        };

        if sanitized.len() + replacement.len_utf8() > MAX_BYTES {
            break;
        }
        sanitized.push(replacement);
    }

    let mut sanitized = sanitized.trim_matches([' ', '.']).to_owned();
    let stem = sanitized
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        sanitized.insert(0, '_');
    }
    (!sanitized.is_empty()).then_some(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_time_formats() {
        assert_eq!(parse_time("00:00:00").ok(), Some(Duration::ZERO));
        assert_eq!(parse_time("00:01:30").ok(), Some(Duration::from_secs(90)));
        assert_eq!(parse_time("01:02:03").ok(), Some(Duration::from_secs(3723)));
        assert_eq!(parse_time("90:00").ok(), Some(Duration::from_secs(5400)));
    }

    #[test]
    fn rejects_malformed_times() {
        for value in ["invalid", "-1", "", "00:60", "00:60:00", "1:2:3:4"] {
            assert!(parse_time(value).is_err(), "{value} should be rejected");
        }
    }

    #[test]
    fn sanitizes_unsafe_filename_characters() {
        assert_eq!(
            sanitize_title("one/two\\three\0four"),
            Some("one_two_three_four".to_owned())
        );
        assert_eq!(sanitize_title("line\nname"), Some("line_name".to_owned()));
    }

    #[test]
    fn sanitizes_long_and_blank_titles() {
        assert!(sanitize_title(" \t\n ").is_none());
        let title = "糖".repeat(200);
        assert_eq!(sanitize_title(&title).map(|value| value.len()), Some(120));
    }

    #[test]
    fn sanitizes_windows_filenames() {
        assert_eq!(sanitize_title("a:b?c*"), Some("a_b_c_".to_owned()));
        assert_eq!(sanitize_title("CON"), Some("_CON".to_owned()));
        assert_eq!(sanitize_title("lpt1.mp4"), Some("_lpt1.mp4".to_owned()));
        assert_eq!(sanitize_title("COM10"), Some("COM10".to_owned()));
    }

    #[test]
    fn validates_trim_ranges() {
        let duration = Duration::from_secs(100);
        assert!(
            TrimRange {
                start: None,
                end: None
            }
            .validate(duration)
            .is_ok()
        );
        assert!(
            TrimRange {
                start: Some(Duration::from_secs(10)),
                end: Some(Duration::from_secs(90)),
            }
            .validate(duration)
            .is_ok()
        );
        assert!(
            TrimRange {
                start: Some(Duration::from_secs(90)),
                end: Some(Duration::from_secs(10)),
            }
            .validate(duration)
            .is_err()
        );
        assert!(
            TrimRange {
                start: None,
                end: Some(Duration::from_secs(101)),
            }
            .validate(duration)
            .is_err()
        );
    }
}
