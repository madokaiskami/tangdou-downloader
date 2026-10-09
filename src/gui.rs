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
    start_time: TimeFields,
    end_time: TimeFields,
    language: Language,
    output_format: OutputFormat,
    keep_original: bool,
    output_directory: String,
    status: String,
    status_kind: StatusKind,
    progress: ProgressState,
    busy: bool,
    worker_receiver: Option<Receiver<WorkerEvent>>,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum Language {
    #[default]
    Chinese,
    English,
}

impl Language {
    fn text(self, english: &'static str, chinese: &'static str) -> &'static str {
        match self {
            Self::Chinese => chinese,
            Self::English => english,
        }
    }
}

#[derive(Default)]
struct TimeFields {
    hours: String,
    minutes: String,
    seconds: String,
}

impl TimeFields {
    fn parse(&self) -> Result<Option<Duration>> {
        let fields = [&self.hours, &self.minutes, &self.seconds];
        if fields.iter().all(|value| value.trim().is_empty()) {
            return Ok(None);
        }
        let fields = fields.map(|value| {
            let value = value.trim();
            if value.is_empty() { "0" } else { value }
        });
        parse_time(&fields.join(":")).map(Some)
    }

    fn ui(&mut self, ui: &mut egui::Ui, language: Language) {
        ui.horizontal(|ui| {
            for (value, label) in [
                (&mut self.hours, language.text("h", "小时")),
                (&mut self.minutes, language.text("min", "分钟")),
                (&mut self.seconds, language.text("s", "秒")),
            ] {
                ui.add(
                    egui::TextEdit::singleline(value)
                        .desired_width(42.0)
                        .hint_text("00"),
                );
                ui.label(label);
            }
        });
    }
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
        install_style(&creation_context.egui_ctx);
        let output_directory = env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .to_string_lossy()
            .into_owned();

        Self {
            share_url: String::new(),
            vid: None,
            title: None,
            trim_enabled: false,
            start_time: TimeFields::default(),
            end_time: TimeFields::default(),
            language: Language::default(),
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
                start: self.start_time.parse()?,
                end: self.end_time.parse()?,
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
            ProgressState::Idle => {
                egui::ProgressBar::new(0.0).text(self.language.text("Ready", "就绪"))
            }
            ProgressState::Indeterminate(label) => egui::ProgressBar::new(0.0)
                .text(localize_message(label, self.language))
                .animate(true),
            ProgressState::Download { downloaded, total } => {
                let known_total = total.filter(|total| *total > 0);
                let text = match known_total {
                    Some(total) => format!(
                        "{} / {} ({:.0}%)",
                        human_bytes(downloaded),
                        human_bytes(total),
                        download_fraction(downloaded, Some(total)) * 100.0
                    ),
                    None => format!(
                        "{} {}",
                        human_bytes(downloaded),
                        self.language.text("downloaded", "已下载")
                    ),
                };
                egui::ProgressBar::new(download_fraction(downloaded, known_total))
                    .text(text)
                    .animate(known_total.is_none())
            }
            ProgressState::Complete => {
                egui::ProgressBar::new(1.0).text(self.language.text("Complete", "完成"))
            }
            ProgressState::Failed => {
                egui::ProgressBar::new(0.0).text(self.language.text("Failed", "失败"))
            }
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
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading(
                            RichText::new(
                                self.language.text("Tangdou Downloader", "糖豆视频下载器"),
                            )
                            .color(ACCENT)
                            .size(26.0),
                        );
                        ui.label(self.language.text("Language", "语言"));
                        egui::ComboBox::from_id_salt("language")
                            .selected_text(self.language.text("English", "简体中文"))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.language,
                                    Language::Chinese,
                                    "简体中文",
                                );
                                ui.selectable_value(
                                    &mut self.language,
                                    Language::English,
                                    "English",
                                );
                            });
                    });
                    let language = self.language;
                    ui.label(
                        RichText::new(language.text(
                            "Original video · Simple trimming · MP3 export",
                            "原始视频下载 · 轻量裁剪 · MP3 导出",
                        ))
                        .color(Color32::from_rgb(93, 111, 126)),
                    );
                    context.send_viewport_cmd(egui::ViewportCommand::Title(
                        language
                            .text("Tangdou Downloader", "糖豆视频下载器")
                            .to_owned(),
                    ));
                    ui.add_space(10.0);

                    section_card().show(ui, |ui| {
                        ui.strong(language.text("01  Video source", "01  视频来源"));
                        ui.label(language.text("Tangdou share URL", "糖豆分享链接"));
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
                                    egui::Button::new(language.text("Parse", "解析")),
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
                                ui.label(language.text("Title:", "标题："));
                                ui.label(self.title.as_deref().unwrap_or("—"));
                                ui.end_row();
                            });
                    });

                    ui.add_space(12.0);
                    section_card().show(ui, |ui| {
                        ui.strong(language.text("02  Trim & output", "02  裁剪与输出"));
                        ui.add_enabled(
                            !self.busy,
                            egui::Checkbox::new(
                                &mut self.trim_enabled,
                                language.text("Enable trim", "启用裁剪"),
                            ),
                        );
                        ui.add_enabled_ui(self.trim_enabled && !self.busy, |ui| {
                            egui::Grid::new("trim_grid")
                                .num_columns(2)
                                .spacing([12.0, 6.0])
                                .show(ui, |ui| {
                                    ui.label(language.text("Start", "开始时间"));
                                    self.start_time.ui(ui, language);
                                    ui.end_row();
                                    ui.label(language.text("End", "结束时间"));
                                    self.end_time.ui(ui, language);
                                    ui.end_row();
                                });
                        });
                        ui.small(language.text(
                "Blank start means beginning; blank end means end of file. Minutes/seconds: 0–59.",
                "开始留空表示从头开始，结束留空表示到文件末尾。分钟和秒须为 0–59。",
            ));
                        ui.small(language.text(
                "MP4 stream-copy trims can align to keyframes and may not be frame-exact.",
                "MP4 无重编码裁剪可能对齐关键帧，无法保证逐帧精确。",
            ));

                        ui.add_space(8.0);
                        ui.label(language.text("Output format", "输出格式"));
                        ui.add_enabled_ui(!self.busy, |ui| {
                            ui.horizontal(|ui| {
                                ui.radio_value(&mut self.output_format, OutputFormat::Mp4, "MP4");
                                ui.radio_value(&mut self.output_format, OutputFormat::Mp3, "MP3");
                            });
                        });
                        let processing =
                            self.trim_enabled || self.output_format == OutputFormat::Mp3;
                        ui.add_enabled(
                            !self.busy && processing,
                            egui::Checkbox::new(
                                &mut self.keep_original,
                                language.text("Keep original downloaded MP4", "保留下载的原始 MP4"),
                            ),
                        );

                        ui.add_space(8.0);
                        ui.label(language.text("Save directory", "保存目录"));
                        ui.horizontal(|ui| {
                            ui.add_enabled(
                                !self.busy,
                                egui::TextEdit::singleline(&mut self.output_directory)
                                    .desired_width((ui.available_width() - 100.0).max(80.0)),
                            );
                            if ui
                                .add_enabled(
                                    !self.busy,
                                    egui::Button::new(language.text("Browse…", "浏览…")),
                                )
                                .clicked()
                            {
                                self.browse_output_directory();
                            }
                        });
                    });

                    ui.add_space(12.0);
                    let download_clicked = ui
                        .add_enabled(
                            !self.busy && !self.share_url.trim().is_empty(),
                            egui::Button::new(
                                RichText::new(language.text("Download / Convert", "下载 / 转换"))
                                    .color(Color32::WHITE),
                            )
                            .fill(ACCENT)
                            .min_size(egui::vec2(ui.available_width(), 42.0)),
                        )
                        .clicked();
                    if download_clicked {
                        self.begin_download(context.clone());
                    }

                    ui.add_space(8.0);
                    section_card().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if self.busy {
                                ui.spinner();
                            }
                            ui.strong(language.text("Task progress", "任务进度"));
                            ui.label(language.text(
                                "Resolve → Download → Process → Finish",
                                "解析 → 下载 → 处理 → 完成",
                            ));
                        });
                        ui.add(
                            self.progress_bar()
                                .fill(ACCENT)
                                .corner_radius(8)
                                .desired_width(ui.available_width())
                                .desired_height(22.0),
                        );
                        ui.add_space(6.0);
                        let color = match self.status_kind {
                            StatusKind::Normal => ui.visuals().text_color(),
                            StatusKind::Success => Color32::from_rgb(50, 150, 80),
                            StatusKind::Error => ui.visuals().error_fg_color,
                        };
                        ui.add(
                            egui::Label::new(
                                RichText::new(localize_message(&self.status, language))
                                    .color(color),
                            )
                            .wrap(),
                        );
                    });
                });
        });
    }
}

const ACCENT: Color32 = Color32::from_rgb(21, 128, 133);

fn section_card() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::WHITE)
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(222, 230, 236)))
        .corner_radius(12)
        .inner_margin(16)
}

fn install_style(context: &egui::Context) {
    context.set_theme(egui::Theme::Light);
    let mut style = (*context.style_of(egui::Theme::Light)).clone();
    style.visuals = egui::Visuals::light();
    style.visuals.panel_fill = Color32::from_rgb(242, 246, 248);
    style.visuals.selection.bg_fill = ACCENT;
    style.visuals.selection.stroke = egui::Stroke::new(1.0, Color32::WHITE);
    style.visuals.widgets.active.bg_fill = ACCENT;
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(213, 237, 237);
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size.y = 30.0;
    context.set_style_of(egui::Theme::Light, style);
}

fn localize_message(message: &str, language: Language) -> String {
    if language == Language::English {
        return message.to_owned();
    }
    let mut translated = message.to_owned();
    for (english, chinese) in [
        ("Ready.", "就绪。"),
        ("Resolving Tangdou metadata...", "正在解析糖豆视频信息…"),
        ("Starting download...", "正在启动下载…"),
        (
            "Background worker stopped unexpectedly.",
            "后台任务意外停止，请重试。",
        ),
        (
            "Metadata resolved; preparing download...",
            "视频信息已解析，正在准备下载…",
        ),
        ("Download started.", "下载已开始。"),
        (
            "Download complete; probing media duration...",
            "下载完成，正在检测媒体时长…",
        ),
        ("Trimming MP4 with stream copy...", "正在无重编码裁剪 MP4…"),
        ("Converting audio to MP3...", "正在转换为 MP3…"),
        ("Metadata resolved.", "视频信息已解析。"),
        ("Starting…", "正在启动…"),
        ("Resolving…", "正在解析…"),
        ("Preparing download…", "正在准备下载…"),
        ("Probing…", "正在检测时长…"),
        ("Processing…", "正在处理…"),
        ("Downloading:", "正在下载："),
        ("Finished:", "已完成："),
        ("Original MP4:", "原始 MP4："),
        (
            "select or enter an output directory",
            "请选择或输入保存目录",
        ),
        ("end must be later than start", "结束时间必须晚于开始时间"),
        ("invalid trim range:", "裁剪范围无效："),
        ("invalid time", "时间输入无效"),
        (
            "use HH:MM:SS or MM:SS with non-negative whole numbers",
            "小时须为非负整数，分钟和秒须为 0–59 的整数",
        ),
        ("invalid Tangdou share URL:", "糖豆分享链接无效："),
        (
            "URL host is not a recognized Tangdou domain:",
            "链接不属于支持的糖豆域名：",
        ),
        (
            "Tangdou share URL has no 'vid' query parameter",
            "糖豆分享链接缺少 vid 参数",
        ),
        (
            "Tangdou share URL contains more than one 'vid' parameter",
            "糖豆分享链接包含重复的 vid 参数",
        ),
        ("required program", "缺少必需程序"),
        (
            "was not found; on Ubuntu install it with:",
            "；Ubuntu 安装命令：",
        ),
        ("returned HTTP status", "返回 HTTP 状态码"),
        (
            "Tangdou returned an unexpected API response:",
            "糖豆接口返回了意外的数据：",
        ),
    ] {
        translated = translated.replace(english, chinese);
    }
    translated
}

fn install_cjk_fallback(context: &egui::Context) {
    const FONT_PATHS: &[&str] = &[
        "/usr/share/fonts/truetype/droid/DroidSansFallbackFull.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ];

    let windows_font = env::var_os("WINDIR").and_then(|directory| {
        let directory = PathBuf::from(directory).join("Fonts");
        ["msyh.ttc", "simhei.ttf"]
            .iter()
            .find_map(|name| fs::read(directory.join(name)).ok())
    });
    let Some(font_bytes) = FONT_PATHS
        .iter()
        .find_map(|path| fs::read(path).ok())
        .or(windows_font)
    else {
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
        assert_eq!(TimeFields::default().parse().ok(), Some(None));
        assert_eq!(
            TimeFields {
                hours: String::new(),
                minutes: "01".into(),
                seconds: "30".into()
            }
            .parse()
            .ok(),
            Some(Some(Duration::from_secs(90)))
        );
    }

    #[test]
    fn time_fields_reject_invalid_components() {
        for (hours, minutes, seconds) in [
            ("0", "60", "0"),
            ("0", "0", "60"),
            ("-1", "0", "0"),
            ("x", "0", "0"),
        ] {
            assert!(
                TimeFields {
                    hours: hours.into(),
                    minutes: minutes.into(),
                    seconds: seconds.into()
                }
                .parse()
                .is_err()
            );
        }
        assert_eq!(
            TimeFields {
                hours: "1".into(),
                minutes: "2".into(),
                seconds: "3".into()
            }
            .parse()
            .ok(),
            Some(Some(Duration::from_secs(3723)))
        );
    }

    #[test]
    fn language_switch_preserves_status_details() {
        assert_eq!(
            localize_message("Finished: /tmp/video.mp4", Language::Chinese),
            "已完成： /tmp/video.mp4"
        );
        assert_eq!(localize_message("Ready.", Language::English), "Ready.");
    }

    #[test]
    fn job_request_ignores_disabled_trim_fields() {
        let app = TangdouApp {
            share_url: "https://www.tangdou.com/play?vid=123".to_owned(),
            start_time: TimeFields {
                hours: "invalid".into(),
                ..TimeFields::default()
            },
            end_time: TimeFields {
                hours: "invalid".into(),
                ..TimeFields::default()
            },
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
            start_time: TimeFields {
                minutes: "2".into(),
                ..TimeFields::default()
            },
            end_time: TimeFields {
                minutes: "1".into(),
                ..TimeFields::default()
            },
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
                start_time: TimeFields::default(),
                end_time: TimeFields::default(),
                language: Language::default(),
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
