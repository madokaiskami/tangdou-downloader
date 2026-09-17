use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, REFERER};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use url::Url;

use crate::error::{AppError, Result};
use crate::model::{DownloadSource, MediaInfo};

const API_ENDPOINT: &str = "https://api-h5.tangdou.com/sample/share/main";
const REFERER_VALUE: &str = "https://www.tangdoucdn.com/";
const API_ACCEPT: &str = "application/json, text/plain, */*";
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140 Safari/537.36";

#[derive(Deserialize)]
struct ApiEnvelope {
    data: Option<Value>,
}

pub fn build_http_client() -> Result<Client> {
    Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|source| AppError::HttpRequest {
            operation: "building the HTTP client",
            source,
        })
}

pub fn extract_vid(share_url: &str) -> Result<String> {
    let parsed =
        Url::parse(share_url).map_err(|error| AppError::InvalidShareUrl(error.to_string()))?;

    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::InvalidShareUrl(
            "only http and https URLs are supported".to_owned(),
        ));
    }

    let host = parsed.host_str().ok_or_else(|| {
        AppError::InvalidShareUrl("the URL does not contain a hostname".to_owned())
    })?;
    if !is_tangdou_host(host) {
        return Err(AppError::UnsupportedTangdouHost(host.to_owned()));
    }

    let vids: Vec<_> = parsed
        .query_pairs()
        .filter_map(|(key, value)| (key == "vid").then_some(value.into_owned()))
        .collect();

    let vid = match vids.as_slice() {
        [] => return Err(AppError::MissingVid),
        [vid] => vid,
        _ => return Err(AppError::DuplicateVid),
    };

    if vid.is_empty() || !vid.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AppError::InvalidVid(vid.clone()));
    }

    Ok(vid.clone())
}

fn is_tangdou_host(host: &str) -> bool {
    ["tangdou.com", "tangdoucdn.com", "tangdouddn.com"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

pub fn resolve_media(client: &Client, vid: &str) -> Result<MediaInfo> {
    let response = client
        .get(API_ENDPOINT)
        .query(&[("vid", vid)])
        .header(ACCEPT, API_ACCEPT)
        .header(REFERER, REFERER_VALUE)
        .send()
        .map_err(|source| AppError::HttpRequest {
            operation: "requesting Tangdou metadata",
            source,
        })?;

    if !response.status().is_success() {
        return Err(AppError::HttpStatus {
            operation: "Tangdou metadata request",
            status: response.status(),
        });
    }

    let body = response.text().map_err(|source| AppError::HttpRequest {
        operation: "reading the Tangdou metadata response",
        source,
    })?;
    parse_api_response(vid, &body)
}

pub fn parse_api_response(vid: &str, body: &str) -> Result<MediaInfo> {
    let raw: Value = serde_json::from_str(body)
        .map_err(|error| AppError::InvalidApiResponse(error.to_string()))?;
    let envelope: ApiEnvelope = serde_json::from_value(raw.clone())
        .map_err(|error| AppError::InvalidApiResponse(error.to_string()))?;
    let data = envelope.data.ok_or_else(|| {
        AppError::MissingMediaUrl(response_context(&raw, "the 'data' object is absent"))
    })?;
    let data = data.as_object().ok_or_else(|| {
        AppError::InvalidApiResponse("the 'data' field is not an object".to_owned())
    })?;

    let media_url = string_field(data, "video_url")
        .or_else(|| string_field(data, "play_url"))
        .ok_or_else(|| {
            AppError::MissingMediaUrl(response_context(
                &raw,
                "both known fields are absent or empty",
            ))
        })?;
    let url = Url::parse(media_url).map_err(|error| {
        AppError::InvalidApiResponse(format!("media URL is not valid: {error}"))
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::InvalidApiResponse(
            "media URL must use http or https".to_owned(),
        ));
    }

    Ok(MediaInfo {
        vid: vid.to_owned(),
        title: ["title", "video_title", "name", "share_title"]
            .iter()
            .find_map(|field| string_field(data, field))
            .map(str::to_owned),
        source: DownloadSource {
            url,
            referer: REFERER_VALUE.to_owned(),
            accept: "video/mp4,video/*;q=0.9,*/*;q=0.8".to_owned(),
        },
    })
}

fn string_field<'a>(data: &'a serde_json::Map<String, Value>, field: &str) -> Option<&'a str> {
    data.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn response_context(raw: &Value, detail: &str) -> String {
    let code = raw.get("code").and_then(Value::as_i64);
    let message = raw
        .get("msg")
        .or_else(|| raw.get("message"))
        .and_then(Value::as_str);
    let data_keys = raw
        .get("data")
        .and_then(Value::as_object)
        .map(|object| object.keys().cloned().collect::<Vec<_>>().join(", "));

    format!(
        "{detail}; code={}; message={}; data keys=[{}]",
        code.map_or_else(|| "unknown".to_owned(), |value| value.to_string()),
        message.unwrap_or("unknown"),
        data_keys.as_deref().unwrap_or("not an object")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_vid_from_valid_share_url() {
        let url = "https://www.tangdouddn.com/h5/play?ad_switch=0&vid=20000014175956&foo=bar";
        assert!(matches!(extract_vid(url).as_deref(), Ok("20000014175956")));
    }

    #[test]
    fn rejects_missing_vid() {
        assert!(matches!(
            extract_vid("https://www.tangdouddn.com/h5/play?foo=bar"),
            Err(AppError::MissingVid)
        ));
    }

    #[test]
    fn rejects_malformed_and_unrelated_urls() {
        assert!(matches!(
            extract_vid("not a url"),
            Err(AppError::InvalidShareUrl(_))
        ));
        assert!(matches!(
            extract_vid("https://example.com/watch?vid=123"),
            Err(AppError::UnsupportedTangdouHost(_))
        ));
    }

    #[test]
    fn rejects_duplicate_vid_parameters() {
        assert!(matches!(
            extract_vid("https://www.tangdou.com/play?vid=123&vid=456"),
            Err(AppError::DuplicateVid)
        ));
    }

    #[test]
    fn parses_video_url_fixture_and_prefers_video_url() {
        let body = include_str!("../tests/fixtures/tangdou_video_url.json");
        let media = parse_api_response("123", body).expect("fixture should parse");
        assert_eq!(
            media.source.url.as_str(),
            "https://media.example/video.mp4?temporary=1"
        );
        assert_eq!(media.title.as_deref(), Some("测试视频"));
    }

    #[test]
    fn falls_back_to_play_url_fixture() {
        let body = include_str!("../tests/fixtures/tangdou_play_url.json");
        let media = parse_api_response("456", body).expect("fixture should parse");
        assert_eq!(
            media.source.url.as_str(),
            "https://media.example/fallback.mp4"
        );
    }

    #[test]
    fn reports_context_when_media_url_is_missing() {
        let error = match parse_api_response(
            "789",
            r#"{"code": 7, "msg": "changed", "data": {"title": "x"}}"#,
        ) {
            Ok(_) => panic!("response should not parse"),
            Err(error) => error,
        };
        let message = error.to_string();
        assert!(message.contains("code=7"));
        assert!(message.contains("title"));
    }
}
