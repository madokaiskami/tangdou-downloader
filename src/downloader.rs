use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, REFERER};

use crate::error::{AppError, Result};
use crate::model::DownloadSource;

#[derive(Clone, Copy, Debug)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

pub fn download_to<F>(
    client: &Client,
    source: &DownloadSource,
    destination_path: &Path,
    destination: &mut File,
    mut on_progress: F,
) -> Result<u64>
where
    F: FnMut(DownloadProgress),
{
    let mut response = build_download_request(client, source)
        .send()
        .map_err(|source| AppError::HttpRequest {
            operation: "downloading the Tangdou video",
            source,
        })?;

    if !response.status().is_success() {
        return Err(AppError::HttpStatus {
            operation: "Tangdou video download",
            status: response.status(),
        });
    }

    let total = response.content_length();
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    on_progress(DownloadProgress { downloaded, total });

    loop {
        let count = response.read(&mut buffer).map_err(|source| AppError::Io {
            operation: "reading the Tangdou video download into",
            path: destination_path.to_owned(),
            source,
        })?;
        if count == 0 {
            break;
        }
        destination
            .write_all(&buffer[..count])
            .map_err(|source| AppError::Io {
                operation: "writing downloaded video to",
                path: destination_path.to_owned(),
                source,
            })?;
        downloaded += count as u64;
        on_progress(DownloadProgress { downloaded, total });
    }

    destination.flush().map_err(|source| AppError::Io {
        operation: "flushing downloaded video at",
        path: destination_path.to_owned(),
        source,
    })?;
    destination.sync_all().map_err(|source| AppError::Io {
        operation: "synchronizing downloaded video at",
        path: destination_path.to_owned(),
        source,
    })?;
    Ok(downloaded)
}

fn build_download_request(
    client: &Client,
    source: &DownloadSource,
) -> reqwest::blocking::RequestBuilder {
    client
        .get(source.url.clone())
        .header(REFERER, &source.referer)
        .header(ACCEPT, &source.accept)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_request_contains_supplied_headers() {
        let source = DownloadSource {
            url: "https://media.example/signed.mp4"
                .parse()
                .expect("test URL should parse"),
            referer: "https://www.tangdoucdn.com/".to_owned(),
            accept: "video/mp4".to_owned(),
        };
        let client = Client::builder().build().expect("test client should build");
        let request = build_download_request(&client, &source)
            .build()
            .expect("download request should build");
        assert_eq!(
            request
                .headers()
                .get(REFERER)
                .and_then(|value| value.to_str().ok()),
            Some("https://www.tangdoucdn.com/")
        );
        assert_eq!(
            request
                .headers()
                .get(ACCEPT)
                .and_then(|value| value.to_str().ok()),
            Some("video/mp4")
        );
    }
}
