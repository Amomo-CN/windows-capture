use std::{
    io::{self, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use clap::Parser;

use windows_capture::{
    capture::{Context, GraphicsCaptureApiHandler},
    encoder::{AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor,
    settings::{ColorFormat, CursorCaptureSettings, DrawBorderSettings, Settings},
    window::Window,
};

use windows::Graphics::Capture::GraphicsCaptureItem;

// 用于存储捕获设置的结构体
struct CaptureSettings {
    stop_flag: Arc<AtomicBool>, // 用于控制捕获停止的标志
    width: u32,                 // 捕获视频的宽度
    height: u32,                // 捕获视频的高度
    path: String,               // 输出文件路径
    bitrate: u32,               // 视频比特率
    frame_rate: u32,            // 视频帧率
}

// 用于处理捕获事件的结构体
struct Capture {
    encoder: Option<VideoEncoder>, // 用于编码帧的视频编码器
    start: Instant,                // 捕获开始的时间
    frame_count_since_reset: u64,  // 自上次重置以来捕获的帧数
    last_reset: Instant,           // 上次重置的时间
    settings: CaptureSettings,     // 捕获设置
}

impl GraphicsCaptureApiHandler for Capture {
    type Flags = CaptureSettings;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    // 创建 Capture 结构体的函数
    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        println!("捕获开始。");

        let video_settings = VideoSettingsBuilder::new(ctx.flags.width, ctx.flags.height)
            .bitrate(ctx.flags.bitrate)
            .frame_rate(ctx.flags.frame_rate);

        let encoder = VideoEncoder::new(
            video_settings,
            AudioSettingsBuilder::default().disabled(true),
            ContainerSettingsBuilder::default(),
            &ctx.flags.path,
        )?;

        Ok(Self {
            encoder: Some(encoder),
            start: Instant::now(),
            frame_count_since_reset: 0,
            last_reset: Instant::now(),
            settings: ctx.flags,
        })
    }

    // 每当有新帧到达时调用的函数
    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        self.frame_count_since_reset += 1;

        // 计算自上次重置以来经过的时间
        let elapsed_since_reset = self.last_reset.elapsed();
        let fps = self.frame_count_since_reset as f64 / elapsed_since_reset.as_secs_f64();
        // 打印 FPS
        print!(
            "\r录制时间: {:.2} 秒 | FPS: {:.2}",
            self.start.elapsed().as_secs_f64(),
            fps
        );
        io::stdout().flush()?;

        // 将帧发送到视频编码器
        self.encoder.as_mut().unwrap().send_frame(frame)?;

        // 如果 stop_flag 被设置，则停止捕获
        if self.settings.stop_flag.load(Ordering::SeqCst) {
            // 完成编码器并保存视频
            self.encoder.take().unwrap().finish()?;
            capture_control.stop();
            println!("\n录制由用户停止。");
        }

        if elapsed_since_reset >= Duration::from_secs(1) {
            // 重置帧计数和上次重置时间
            self.frame_count_since_reset = 0;
            self.last_reset = Instant::now();
        }

        Ok(())
    }

    // 当捕获项（通常是窗口）关闭时调用的可选处理程序
    fn on_closed(&mut self) -> Result<(), Self::Error> {
        println!("捕获会话已关闭");

        Ok(())
    }
}

#[derive(Parser)]
#[command(name = "屏幕捕获")]
#[command(version = "1.0")]
#[command(author = "你的名字")]
#[command(about = "捕获屏幕")]
struct Cli {
    /// 要捕获的窗口名称
    #[arg(long, conflicts_with = "monitor_index")]
    window_name: Option<String>,

    /// 要捕获的显示器索引
    #[arg(long, conflicts_with = "window_name")]
    monitor_index: Option<u32>,

    /// 光标捕获设置：always, never, default
    #[arg(long, default_value = "default")]
    cursor_capture: String,

    /// 绘制边框设置：always, never, default
    #[arg(long, default_value = "default")]
    draw_border: String,

    /// 输出文件路径
    #[arg(long, default_value = "video.mp4")]
    path: String,

    /// 视频比特率（每秒比特数）
    #[arg(long, default_value_t = 15_000_000)]
    bitrate: u32,

    /// 视频帧率
    #[arg(long, default_value_t = 60)]
    frame_rate: u32,
}

// 解析光标捕获设置
fn parse_cursor_capture(s: &str) -> CursorCaptureSettings {
    match s.to_lowercase().as_str() {
        "always" => CursorCaptureSettings::WithCursor,
        "never" => CursorCaptureSettings::WithoutCursor,
        "default" => CursorCaptureSettings::Default,
        _ => {
            eprintln!("无效的光标捕获值: {}", s);
            std::process::exit(1);
        }
    }
}

// 解析绘制边框设置
fn parse_draw_border(s: &str) -> DrawBorderSettings {
    match s.to_lowercase().as_str() {
        "always" => DrawBorderSettings::WithBorder,
        "never" => DrawBorderSettings::WithoutBorder,
        "default" => DrawBorderSettings::Default,
        _ => {
            eprintln!("无效的绘制边框值: {}", s);
            std::process::exit(1);
        }
    }
}

// 启动捕获
fn start_capture<T>(
    capture_item: T,
    cursor_capture: CursorCaptureSettings,
    draw_border: DrawBorderSettings,
    settings: CaptureSettings,
) where
    T: TryInto<GraphicsCaptureItem>,
{
    let capture_settings = Settings::new(
        capture_item,
        cursor_capture,
        draw_border,
        ColorFormat::Rgba8,
        settings,
    );

    // 启动捕获并控制当前线程
    // 处理程序 trait 中的错误将在这里结束
    Capture::start(capture_settings).expect("屏幕捕获失败");
}

fn main() {
    let cli = Cli::parse();

    let cursor_capture = parse_cursor_capture(&cli.cursor_capture);
    let draw_border = parse_draw_border(&cli.draw_border);

    let stop_flag = Arc::new(AtomicBool::new(false));

    // 设置 Ctrl+C 处理程序
    {
        let stop_flag = stop_flag.clone();
        ctrlc::set_handler(move || {
            stop_flag.store(true, Ordering::SeqCst);
        })
        .expect("设置 Ctrl-C 处理程序出错");
    }

    if let Some(window_name) = cli.window_name {
        // 使用包含指定名称的窗口
        let capture_item = Window::from_contains_name(&window_name).expect("未找到窗口！");

        // 自动检测窗口的宽度和高度
        let rect = capture_item.rect().expect("获取窗口矩形失败");
        let width = (rect.right - rect.left) as u32;
        let height = (rect.bottom - rect.top) as u32;

        let capture_settings = CaptureSettings {
            stop_flag: stop_flag.clone(),
            width,
            height,
            path: cli.path.clone(),
            bitrate: cli.bitrate,
            frame_rate: cli.frame_rate,
        };

        println!(
            "窗口标题: {}",
            capture_item.title().expect("获取窗口标题失败")
        );
        println!("窗口大小: {}x{}", width, height);

        start_capture(capture_item, cursor_capture, draw_border, capture_settings);
    } else if let Some(index) = cli.monitor_index {
        // 使用指定索引的显示器
        let capture_item =
            Monitor::from_index(usize::try_from(index).unwrap()).expect("未找到显示器！");

        // 自动检测显示器的宽度和高度
        let width = capture_item.width().expect("获取显示器宽度失败");
        let height = capture_item.height().expect("获取显示器高度失败");

        let capture_settings = CaptureSettings {
            stop_flag: stop_flag.clone(),
            width,
            height,
            path: cli.path.clone(),
            bitrate: cli.bitrate,
            frame_rate: cli.frame_rate,
        };

        println!("显示器索引: {}", index);
        println!("显示器大小: {}x{}", width, height);

        start_capture(capture_item, cursor_capture, draw_border, capture_settings);
    } else {
        // 默认捕获桌面
        let capture_item = Monitor::primary().expect("未找到主显示器！");

        // 自动检测主显示器的宽度和高度
        let width = capture_item.width().expect("获取主显示器宽度失败");
        let height = capture_item.height().expect("获取主显示器高度失败");

        let capture_settings = CaptureSettings {
            stop_flag: stop_flag.clone(),
            width,
            height,
            path: cli.path.clone(),
            bitrate: cli.bitrate,
            frame_rate: cli.frame_rate,
        };

        println!("默认捕获主显示器");
        println!("主显示器大小: {}x{}", width, height);

        start_capture(capture_item, cursor_capture, draw_border, capture_settings);
    }
}