use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, RichText};
use tangdou_downloader::model::{OutputFormat, TrimRange, parse_time};
use tangdou_downloader::tangdou;
use tangdou_downloader::workflow::{JobRequest, WorkerEvent, run_job, run_parse};
use tangdou_downloader::{AppError, Result};

pub struct TangdouApp {
    share_url: String,
    vid: Option<String>,
    title: Option<String>,
    trim_enabled: bool,
    start_time: String,
    end_time: String,
    output_format: OutputFormat,
    keep_original: bool,
    output_directory: String,
    status: String,
    status_kind: StatusKind,
    progress: ProgressState,
    busy: bool,
    worker_receiver: Option<Receiver<WorkerEvent>>,
}

#[derive(Clone, Copy, Default)]
enum StatusKind {
    #[default]
    Normal,
    Success,
    Error,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum ProgressState {
    #[default]
    Idle,
    Indeterminate(&'static str),
    Download {
        downloaded: u64,
        total: Option<u64>,
    },
    Complete,
    Failed,
}

impl TangdouApp {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        install_cjk_fallback(&creation_context.egui_ctx);
        let output_directory = env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .to_string_lossy()
            .into_owned();

        Self {
            share_url: String::new(),
            vid: None,
            title: None,
            trim_enabled: false,
            start_time: String::new(),
            end_time: String::new(),
            output_format: OutputFormat::Mp4,
            keep_original: false,
            output_directory,
            status: "Ready.".to_owned(),
            status_kind: StatusKind::Normal,
            progress: ProgressState::Idle,
            busy: false,
            worker_receiver: None,
        }
    }

    fn begin_parse(&mut self, context: egui::Context) {
        let share_url = self.share_url.trim().to_owned();
        let vid = match tangdou::extract_vid(&share_url) {
            Ok(vid) => vid,
            Err(error) => {
                self.set_error(error);
                return;
            }
        };

        self.vid = Some(vid);
        self.title = None;
        let sender = self.start_worker("Resolving Tangdou metadata...");
        thread::spawn(move || {
            let _result = run_parse(&share_url, |event| {
                send_worker_event(&sender, &context, event);
            });
        });
    }

    fn begin_download(&mut self, context: egui::Context) {
        let request = match self.job_request() {
            Ok(request) => request,
            Err(error) => {
                self.set_error(error);
                return;
            }
        };

        let sender = self.start_worker("Starting download...");
        thread::spawn(move || {
            let _result = run_job(request, |event| {
                send_worker_event(&sender, &context, event);
            });
        });
    }

    fn start_worker(&mut self, status: &str) -> Sender<WorkerEvent> {
        let (sender, receiver) = mpsc::channel();
        self.worker_receiver = Some(receiver);
        self.busy = true;
        self.status = status.to_owned();
        self.status_kind = StatusKind::Normal;
        self.progress = ProgressState::Indeterminate("Starting…");
        sender
    }

    fn job_request(&self) -> Result<JobRequest> {
        let share_url = self.share_url.trim();
        tangdou::extract_vid(share_url)?;

        let output_directory = self.output_directory.trim();
        if output_directory.is_empty() {
            return Err(AppError::Arguments(
                "select or enter an output directory".to_owned(),
            ));
        }

        let trim = if self.trim_enabled {
            TrimRange {
                start: parse_optional_time(&self.start_time)?,
                end: parse_optional_time(&self.end_time)?,
            }
        } else {
            TrimRange::default()
        };

        if let (Some(start), Some(end)) = (trim.start, trim.end)
            && end <= start
        {
            return Err(AppError::InvalidTrim(
                "end must be later than start".to_owned(),
            ));
        }

        Ok(JobRequest {
            share_url: share_url.to_owned(),
            output_dir: PathBuf::from(output_directory),
            format: self.output_format,
            trim_enabled: self.trim_enabled,
            trim,
            keep_original: self.keep_original,
        })
    }

    fn browse_output_directory(&mut self) {
        let mut dialog = rfd::FileDialog::new();
        let current = Path::new(self.output_directory.trim());
        if current.is_dir() {
            dialog = dialog.set_directory(current);
        }
        if let Some(path) = dialog.pick_folder() {
            self.output_directory = path.to_string_lossy().into_owned();
        }
    }

    fn poll_worker(&mut self) {
        let mut messages = Vec::new();
        let mut disconnected = false;
        if let Some(receiver) = &self.worker_receiver {
            loop {
                match receiver.try_recv() {
                    Ok(message) => messages.push(message),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        let mut finished = false;

        for event in messages {
            finished |= self.apply_worker_event(event);
        }

        if disconnected && !finished && self.busy {
            self.status = "Background worker stopped unexpectedly.".to_owned();
            self.status_kind = StatusKind::Error;
            self.progress = ProgressState::Failed;
            finished = true;
        }

        if finished {
            self.busy = false;
            self.worker_receiver = None;
        }
    }

    fn set_error(&mut self, error: AppError) {
        self.status = error.to_string();
        self.status_kind = StatusKind::Error;
        self.progress = ProgressState::Failed;
    }

    fn apply_worker_event(&mut self, event: WorkerEvent) -> bool {
        match event {
            WorkerEvent::Resolving { vid } => {
                self.vid = Some(vid);
                self.title = None;
                self.status = "Resolving Tangdou metadata...".to_owned();
                self.status_kind = StatusKind::Normal;
                self.progress = ProgressState::Indeterminate("Resolving…");
                false
            }
            WorkerEvent::MetadataResolved(metadata) => {
                self.vid = Some(metadata.vid);
                self.title = metadata.title;
                self.status = "Metadata resolved; preparing download...".to_owned();
                self.progress = ProgressState::Indeterminate("Preparing download…");
                false
            }
            WorkerEvent::DownloadStarted => {
                self.status = "Download started.".to_owned();
                self.progress = ProgressState::Download {
                    downloaded: 0,
                    total: None,
                };
                false
            }
            WorkerEvent::DownloadProgress { downloaded, total } => {
                self.status = download_status(downloaded, total);
                self.progress = ProgressState::Download { downloaded, total };
                false
            }
            WorkerEvent::Probing => {
                self.status = "Download complete; probing media duration...".to_owned();
                self.progress = ProgressState::Indeterminate("Probing…");
                false
            }
            WorkerEvent::Processing { format } => {
                self.status = match format {
                    OutputFormat::Mp4 => "Trimming MP4 with stream copy...".to_owned(),
                    OutputFormat::Mp3 => "Converting audio to MP3...".to_owned(),
                };
                self.progress = ProgressState::Indeterminate("Processing…");
                false
            }
            WorkerEvent::Parsed(metadata) => {
                self.vid = Some(metadata.vid);
                self.title = metadata.title;
                self.status = "Metadata resolved.".to_owned();
                self.status_kind = StatusKind::Success;
                self.progress = ProgressState::Complete;
                true
            }
            WorkerEvent::Finished(result) => {
                self.status = format!("Finished: {}", result.output_path.display());
                if let Some(original_path) = result.original_path {
                    self.status
                        .push_str(&format!("\nOriginal MP4: {}", original_path.display()));
                }
                self.status_kind = StatusKind::Success;
                self.progress = ProgressState::Complete;
                true
            }
            WorkerEvent::Failed(error) => {
                self.status = error;
                self.status_kind = StatusKind::Error;
                self.progress = ProgressState::Failed;
                true
            }
        }
    }

    fn progress_bar(&self) -> egui::ProgressBar {
        match self.progress {
            ProgressState::Idle => egui::ProgressBar::new(0.0).text("Ready"),
            ProgressState::Indeterminate(label) => {
                egui::ProgressBar::new(0.0).text(label).animate(true)
            }
            ProgressState::Download { downloaded, total } => {
                let known_total = total.filter(|total| *total > 0);
                let text = match known_total {
                    Some(total) => format!(
                        "{} / {} ({:.0}%)",
                        human_bytes(downloaded),
                        human_bytes(total),
                        download_fraction(downloaded, Some(total)) * 100.0
                    ),
                    None => format!("{} downloaded", human_bytes(downloaded)),
                };
                egui::ProgressBar::new(download_fraction(downloaded, known_total))
                    .text(text)
                    .animate(known_total.is_none())
            }
            ProgressState::Complete => egui::ProgressBar::new(1.0).text("Complete"),
            ProgressState::Failed => egui::ProgressBar::new(0.0).text("Failed"),
        }
    }
}

impl eframe::App for TangdouApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker();
        if self.busy {
            context.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = root_ui.ctx().clone();
        egui::CentralPanel::default().show(root_ui, |ui| {
            ui.heading("Tangdou Downloader");
            ui.add_space(10.0);

            ui.label("Tangdou share URL");
            let url_response = ui.add_enabled(
                !self.busy,
                egui::TextEdit::singleline(&mut self.share_url)
                    .desired_width(f32::INFINITY)
                    .hint_text("https://www.tangdouddn.com/h5/play?...&vid=..."),
            );
            if url_response.changed() {
                self.vid = None;
                self.title = None;
            }

            ui.horizontal(|ui| {
                let parse_clicked = ui
                    .add_enabled(
                        !self.busy && !self.share_url.trim().is_empty(),
                        egui::Button::new("Parse"),
                    )
                    .clicked();
                if parse_clicked {
                    self.begin_parse(context.clone());
                }
                if self.busy {
                    ui.spinner();
                }
            });

            ui.add_space(8.0);
            egui::Grid::new("metadata_grid")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label("VID:");
                    ui.label(self.vid.as_deref().unwrap_or("—"));
                    ui.end_row();
                    ui.label("Title:");
                    ui.label(self.title.as_deref().unwrap_or("—"));
                    ui.end_row();
                });

            ui.separator();
            ui.add_enabled(
                !self.busy,
                egui::Checkbox::new(&mut self.trim_enabled, "Enable trim"),
            );
            ui.add_enabled_ui(self.trim_enabled && !self.busy, |ui| {
                egui::Grid::new("trim_grid")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Start");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.start_time)
                                .hint_text("beginning (HH:MM:SS)"),
                        );
                        ui.end_row();
                        ui.label("End");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.end_time)
                                .hint_text("end of file (HH:MM:SS)"),
                        );
                        ui.end_row();
                    });
            });
            ui.small("MP4 stream-copy trims can align to keyframes and may not be frame-exact.");

            ui.separator();
            ui.label("Output format");
            ui.add_enabled_ui(!self.busy, |ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.output_format, OutputFormat::Mp4, "MP4");
                    ui.radio_value(&mut self.output_format, OutputFormat::Mp3, "MP3");
                });
            });
            let processing = self.trim_enabled || self.output_format == OutputFormat::Mp3;
            ui.add_enabled(
                !self.busy && processing,
                egui::Checkbox::new(&mut self.keep_original, "Keep original downloaded MP4"),
            );

            ui.add_space(8.0);
            ui.label("Save directory");
            ui.horizontal(|ui| {
                ui.add_enabled(
                    !self.busy,
                    egui::TextEdit::singleline(&mut self.output_directory)
                        .desired_width(f32::INFINITY),
                );
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Browse…"))
                    .clicked()
                {
                    self.browse_output_directory();
                }
            });

            ui.add_space(12.0);
            let download_clicked = ui
                .add_enabled(
                    !self.busy && !self.share_url.trim().is_empty(),
                    egui::Button::new("Download / Convert").min_size(egui::vec2(180.0, 34.0)),
                )
                .clicked();
            if download_clicked {
                self.begin_download(context.clone());
            }

            ui.add_space(8.0);
            ui.add(self.progress_bar());

            ui.separator();
            ui.label("Status");
            let color = match self.status_kind {
                StatusKind::Normal => ui.visuals().text_color(),
                StatusKind::Success => Color32::from_rgb(50, 150, 80),
                StatusKind::Error => ui.visuals().error_fg_color,
            };
            ui.label(RichText::new(&self.status).color(color));
        });
    }
}

fn parse_optional_time(value: &str) -> Result<Option<Duration>> {
    let value = value.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        parse_time(value).map(Some)
    }
}

fn install_cjk_fallback(context: &egui::Context) {
    const FONT_PATHS: &[&str] = &[
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ];

    let Some(font_bytes) = FONT_PATHS.iter().find_map(|path| fs::read(path).ok()) else {
        return;
    };
    let mut fonts = FontDefinitions::default();
    let font_name = "system-cjk".to_owned();
    fonts
        .font_data
        .insert(font_name.clone(), FontData::from_owned(font_bytes).into());
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(font_name.clone());
    }
    context.set_fonts(fonts);
}

fn download_fraction(downloaded: u64, total: Option<u64>) -> f32 {
    match total.filter(|total| *total > 0) {
        Some(total) => (downloaded as f64 / total as f64).clamp(0.0, 1.0) as f32,
        None => 0.0,
    }
}

fn download_status(downloaded: u64, total: Option<u64>) -> String {
    match total.filter(|total| *total > 0) {
        Some(total) => format!(
            "Downloading: {} / {} ({:.0}%)",
            human_bytes(downloaded),
            human_bytes(total),
            download_fraction(downloaded, Some(total)) * 100.0
        ),
        None => format!("Downloading: {}", human_bytes(downloaded)),
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn send_worker_event(sender: &Sender<WorkerEvent>, context: &egui::Context, event: WorkerEvent) {
    if sender.send(event).is_ok() {
        context.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_time_treats_blank_as_open_boundary() {
        assert_eq!(parse_optional_time("  ").ok(), Some(None));
        assert_eq!(
            parse_optional_time("01:30").ok(),
            Some(Some(Duration::from_secs(90)))
        );
    }

    #[test]
    fn job_request_ignores_disabled_trim_fields() {
        let app = TangdouApp {
            share_url: "https://www.tangdou.com/play?vid=123".to_owned(),
            start_time: "invalid".to_owned(),
            end_time: "invalid".to_owned(),
            output_directory: "/tmp".to_owned(),
            ..TangdouApp::new_for_test()
        };
        let request = app.job_request().expect("disabled trim should be ignored");
        assert_eq!(request.trim, TrimRange::default());
        assert!(!request.trim_enabled);
    }

    #[test]
    fn job_request_rejects_reversed_enabled_trim() {
        let app = TangdouApp {
            share_url: "https://www.tangdou.com/play?vid=123".to_owned(),
            trim_enabled: true,
            start_time: "00:02:00".to_owned(),
            end_time: "00:01:00".to_owned(),
            output_directory: "/tmp".to_owned(),
            ..TangdouApp::new_for_test()
        };
        assert!(matches!(app.job_request(), Err(AppError::InvalidTrim(_))));
    }

    #[test]
    fn download_progress_event_updates_bar_and_status() {
        let mut app = TangdouApp::new_for_test();
        let finished = app.apply_worker_event(WorkerEvent::DownloadProgress {
            downloaded: 5 * 1024 * 1024,
            total: Some(10 * 1024 * 1024),
        });

        assert!(!finished);
        assert_eq!(
            app.progress,
            ProgressState::Download {
                downloaded: 5 * 1024 * 1024,
                total: Some(10 * 1024 * 1024),
            }
        );
        assert!(app.status.contains("50%"));
        assert!(app.status.contains("5.0 MiB / 10.0 MiB"));
    }

    #[test]
    fn formats_known_and_unknown_download_sizes() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(download_fraction(5, Some(10)), 0.5);
        assert_eq!(download_fraction(20, Some(10)), 1.0);
        assert_eq!(download_fraction(5, None), 0.0);
    }

    impl TangdouApp {
        fn new_for_test() -> Self {
            Self {
                share_url: String::new(),
                vid: None,
                title: None,
                trim_enabled: false,
                start_time: String::new(),
                end_time: String::new(),
                output_format: OutputFormat::Mp4,
                keep_original: false,
                output_directory: String::new(),
                status: String::new(),
                status_kind: StatusKind::Normal,
                progress: ProgressState::Idle,
                busy: false,
                worker_receiver: None,
            }
        }
    }
}
