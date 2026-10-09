# 糖豆视频下载器 · Tangdou Downloader

轻量 Rust 原生桌面工具：粘贴糖豆分享链接，下载接口提供的原始 MP4，
按需裁剪或导出 MP3。支持简体中文 / English，面向 Linux 与 Windows x86_64。

## 下载与安装

从 [GitHub Releases](https://github.com/madokaiskami/tangdou-downloader/releases)
下载对应平台的压缩包。包内包含 GUI、CLI、README 和 MIT 许可证，不包含 FFmpeg。
SHA256SUMS 校验文件随版本提供。

| 平台 | 发布包 | 启动 |
| --- | --- | --- |
| Linux x86_64 | `tangdou-downloader-linux-x86_64.tar.gz` | 解压后运行 `./tangdou-downloader` |
| Windows x86_64 | `tangdou-downloader-windows-x86_64.zip` | 解压后双击 `tangdou-downloader.exe` |

Linux 发布包在 Ubuntu 22.04 构建，适用于具有图形桌面、glibc 2.35 或更新版本的
Linux x86_64 系统；其他发行版仍可能需要安装 OpenGL、X11 / Wayland 运行库。
Windows 发布包使用原生 MSVC 工具链构建；目前尚未进行 Windows 桌面的人工端到端验证。
发布包是便携压缩包，不是安装程序、AppImage 或 .deb。

### Linux 依赖

Ubuntu / Debian：

```bash
sudo apt install ffmpeg fonts-noto-cjk libgl1 libxkbcommon0
tar -xzf tangdou-downloader-linux-x86_64.tar.gz
cd tangdou-downloader-linux-x86_64
./tangdou-downloader
```

其他发行版请通过对应包管理器安装 FFmpeg、中文字体和图形运行库。

### Windows 依赖

从 [FFmpeg 官方下载页](https://ffmpeg.org/download.html) 选择 Windows 构建，
解压后将包含 `ffmpeg.exe` 和 `ffprobe.exe` 的 `bin` 目录加入用户 `PATH`，
再重新启动下载器。PowerShell 中可验证：

```powershell
ffmpeg -version
ffprobe -version
.\tangdou-downloader.exe
```

GUI 使用系统微软雅黑或黑体显示中文。Windows GUI 不开启控制台窗口，
调用 FFmpeg / ffprobe 时也不会弹出命令窗口。
若 Windows 提示缺少 MSVC 运行库，请安装微软官方
[Visual C++ x64 Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)。

## 使用方法

1. 粘贴糖豆分享链接，点击“解析”查看标题和 VID，或直接点击“下载 / 转换”。
2. 按需启用裁剪；开始和结束各有“小时 / 分钟 / 秒”三格。
3. 选择 MP4 或 MP3，指定保存目录，按需勾选保留原始 MP4。
4. 点击“下载 / 转换”，通过任务卡片查看解析、下载、处理和完成状态。

默认界面为简体中文，可在顶部语言下拉框切换 English。
语言选择仅在当前会话有效。外部工具和网络的技术诊断可能仍为英文。

时间规则：小时是非负整数，分钟与秒为 0–59。整行开始留空表示从文件开头开始；
整行结束留空表示到文件末尾。只填部分格时，其余空格视为零。
结束须晚于开始且不能超过检测到的媒体时长；无效输入会明确报错。

界面采用浅色卡片、青绿色操作按钮与进度条；窗口较小时可以滚动。
下载显示字节数及服务器提供的总大小；未知大小和媒体处理使用不定进度动画，
不显示虚构的处理百分比。网络与媒体任务运行在后台线程，界面保持响应。

每次下载重新解析临时签名 URL，不保存账号、Cookie 或下载历史。
同名输出自动添加数字后缀，成功后清理临时文件。
MP4 裁剪采用 FFmpeg 流复制，可能对齐关键帧，不能保证逐帧精确。
MP3 直接通过 FFmpeg 导出，不产生中间 WAV。
即使只下载 MP4 也需要 ffprobe；裁剪及 MP3 导出还需要 ffmpeg。

## 命令行

```bash
./tangdou-cli 'https://www.tangdouddn.com/h5/play?vid=20000014175956' -o ./downloads
./tangdou-cli 'https://www.tangdouddn.com/h5/play?vid=20000014175956' \
  --format mp3 --start 00:00:10 --end 00:01:00 --keep-original -o ./downloads
./tangdou-cli --help
```

Windows 使用 `tangdou-cli.exe`。CLI 时间参数支持 HH:MM:SS 或 MM:SS。
示例 VID 只演示输入格式，服务端视频的可用性可能变化。

## 从源码构建

需要 Rust stable（Edition 2024）和桌面环境。Ubuntu 构建依赖：

```bash
sudo apt install build-essential pkg-config libx11-dev libxi-dev libxrandr-dev \
  libxcursor-dev libxinerama-dev libgl1-mesa-dev libwayland-dev libxkbcommon-dev
cargo build --locked --release --bins
./target/release/tangdou-downloader
```

Windows 安装 Rust stable MSVC 工具链和 Visual Studio Build Tools 的
“使用 C++ 的桌面开发”组件，然后执行：

```powershell
cargo build --locked --release --bins
.	arget\release\tangdou-downloader.exe
```

检查命令：

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked
```

[构建工作流](.github/workflows/build.yml) 在 main 推送或手动触发时，
分别在 Ubuntu 与 Windows runner 上检查、测试、构建和打包。
产物可在 [Actions](https://github.com/madokaiskami/tangdou-downloader/actions)
中下载；正式版本额外上传到 Releases。
Linux 包保留可执行权限，Windows 包包含两个 .exe，各自附 SHA-256 校验文件。

## 模块与维护

- `gui.rs`：界面、语言、时间输入、任务状态。
- `tangdou.rs`：链接解析、VID、糖豆 API 及媒体 URL。
- `workflow.rs`：后台下载、检测、转换流程和结构化事件。
- `downloader.rs`：带 Referer 的 HTTP 下载与字节进度。
- `ffmpeg.rs`：依赖检查、时长检测、MP4 裁剪与 MP3 导出。
- `model.rs` / `error.rs`：数据、校验、跨平台文件名与错误。

解析测试使用保存的 JSON fixture，不依赖实时 API。
糖豆接口发生变化时优先更新 `tangdou.rs`。
系统缺少工具、服务接口异常、网络失败或无写入权限时，界面会显示错误。
目前没有取消任务、精确重编码裁剪或自动更新功能。

## English

A lightweight native Rust desktop downloader for Tangdou share URLs, with MP4 downloads,
optional stream-copy trimming and MP3 export. Switch between Simplified Chinese and English
at the top of the window. Downloads and media processing run on a background thread.

Download the Linux x86_64 tarball or Windows x86_64 ZIP from Releases.
Install FFmpeg and ffprobe separately and make them available on PATH.
Linux packages are built on Ubuntu 22.04; Windows packages are built with MSVC.
Windows desktop end-to-end testing has not yet been performed.
Each trim boundary has three fields (hours, minutes, seconds); an entirely blank boundary
means beginning/end of file. Minutes and seconds must be 0–59.

## 许可证与使用范围

[MIT License](LICENSE)。仅用于你有权下载和处理的媒体，不提供 DRM 绕过。
