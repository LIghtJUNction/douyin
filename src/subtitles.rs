use std::fs::{self, File};
use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use reqwest::blocking::Client;
use serde_json::json;
use symphonia::core::audio::sample::Sample;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::{fs_utils, settings};

const TARGET_SAMPLE_RATE: u32 = 16_000;
const DEFAULT_MODEL: &str = "small";
const MODEL_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

#[derive(Debug, Args)]
pub struct SubtitleArgs {
    /// 本地音频或视频文件；可批量传入
    #[arg(required = true, value_parser = existing_file)]
    media: Vec<PathBuf>,
    /// 输出文件；批量处理时作为输出目录
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// 字幕输出格式
    #[arg(short = 'f', long = "format", value_enum, default_value_t = SubtitleFormat::Srt)]
    output_format: SubtitleFormat,
    /// whisper.cpp GGML 模型名称或本地 .bin 路径
    #[arg(long, default_value = DEFAULT_MODEL)]
    model: String,
    /// 语言代码，例如 zh/en；不传则自动识别
    #[arg(long)]
    language: Option<String>,
    /// 识别后端；Rust 版本使用 whisper.cpp
    #[arg(long, value_enum, default_value_t = Backend::Auto)]
    backend: Backend,
    /// 运行设备；GPU 能力由编译特性决定
    #[arg(long, value_enum, default_value_t = Device::Auto)]
    device: Device,
    /// 保留旧参数；whisper.cpp 精度由 GGML 模型量化格式决定
    #[arg(long, default_value = "default")]
    compute_type: String,
    /// 解码 beam size
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(i32).range(1..))]
    beam_size: i32,
    /// GGML 模型缓存目录
    #[arg(long)]
    model_cache_dir: Option<PathBuf>,
    /// 只使用本地模型，不联网下载
    #[arg(long)]
    local_files_only: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SubtitleFormat {
    Srt,
    Vtt,
    Txt,
    Json,
}

impl SubtitleFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
            Self::Txt => "txt",
            Self::Json => "json",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Backend {
    Auto,
    WhisperCpp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Device {
    Auto,
    Cpu,
    Cuda,
    Metal,
}

#[derive(Clone, Debug, PartialEq)]
struct SubtitleSegment {
    start: f64,
    end: f64,
    text: String,
}

pub fn run(args: SubtitleArgs) -> Result<(), String> {
    let _backend = args.backend;
    validate_device(args.device)?;
    if args.compute_type != "default" {
        eprintln!(
            "提示：Rust whisper.cpp 后端忽略 --compute-type={}；请通过量化 GGML 模型控制精度",
            args.compute_type
        );
    }
    let model_path = resolve_model(
        &args.model,
        args.model_cache_dir.as_deref(),
        args.local_files_only,
    )?;
    let multiple = args.media.len() > 1;
    for media in &args.media {
        let output =
            resolve_output_path(media, args.output.as_deref(), args.output_format, multiple);
        eprintln!("正在解码媒体: {}", media.display());
        let audio = decode_media(media)?;
        eprintln!("正在生成字幕: {}", media.display());
        let segments = transcribe(
            &model_path,
            &audio,
            args.language.as_deref(),
            args.beam_size,
            args.device,
        )?;
        write_subtitle(&segments, &output, args.output_format)?;
        eprintln!("字幕已保存: {}", output.display());
    }
    Ok(())
}

fn existing_file(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("媒体文件不存在: {value}"))
    }
}

fn validate_device(device: Device) -> Result<(), String> {
    match device {
        Device::Cuda if !cfg!(feature = "cuda") => Err(
            "当前二进制未启用 CUDA；请使用 cargo build --release --features cuda，或改用 --device cpu"
                .to_owned(),
        ),
        Device::Metal if !cfg!(target_os = "macos") => {
            Err("Metal 仅支持 macOS；请改用 --device cpu".to_owned())
        }
        _ => Ok(()),
    }
}

fn resolve_model(
    model: &str,
    cache_dir: Option<&Path>,
    local_only: bool,
) -> Result<PathBuf, String> {
    let direct = PathBuf::from(model);
    if direct.is_file() {
        return Ok(direct);
    }
    let model_name = model_alias(model)
        .ok_or_else(|| format!("模型路径不存在，且不是支持的模型别名: {model}"))?;
    let filename = format!("ggml-{model_name}.bin");
    let directory = cache_dir
        .map(Path::to_owned)
        .unwrap_or_else(|| settings::config_root().join("models"));
    let path = directory.join(&filename);
    if path.is_file() {
        return Ok(path);
    }
    if local_only {
        return Err(format!("本地未找到模型: {}", path.display()));
    }
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let url = format!("{MODEL_BASE_URL}/{filename}");
    eprintln!("正在下载 whisper.cpp 模型: {url}");
    download_model(&url, &path)?;
    Ok(path)
}

fn model_alias(value: &str) -> Option<&'static str> {
    match value {
        "tiny" => Some("tiny"),
        "tiny.en" => Some("tiny.en"),
        "base" => Some("base"),
        "base.en" => Some("base.en"),
        "small" | "Systran/faster-whisper-small" => Some("small"),
        "small.en" => Some("small.en"),
        "medium" => Some("medium"),
        "medium.en" => Some("medium.en"),
        "large-v1" => Some("large-v1"),
        "large-v2" => Some("large-v2"),
        "large-v3" | "large" => Some("large-v3"),
        "large-v3-turbo" | "turbo" => Some("large-v3-turbo"),
        _ => None,
    }
}

fn download_model(url: &str, path: &Path) -> Result<(), String> {
    let client = Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(3_600))
        .build()
        .map_err(|error| error.to_string())?;
    let mut response = client.get(url).send().map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("模型下载失败: {}", response.status()));
    }
    fs_utils::atomic_copy(&mut response, path)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn decode_media(path: &Path) -> Result<Vec<f32>, String> {
    let source = File::open(path).map_err(|error| error.to_string())?;
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|error| format!("无法识别媒体格式: {error}"))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| "媒体文件没有可解码的音轨".to_owned())?;
    let track_id = track.id;
    let parameters = track
        .codec_params
        .as_ref()
        .and_then(|parameters| parameters.audio())
        .ok_or_else(|| "音轨缺少解码参数".to_owned())?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(parameters, &AudioDecoderOptions::default())
        .map_err(|error| format!("不支持的音频编码: {error}"))?;
    let mut mono = Vec::new();
    let mut sample_rate = None;
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::ResetRequired) => {
                return Err("媒体音轨在解码过程中发生变化，暂不支持".to_owned());
            }
            Err(error) => return Err(format!("读取媒体数据失败: {error}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) | Err(SymphoniaError::IoError(_)) => continue,
            Err(error) => return Err(format!("解码音频失败: {error}")),
        };
        let rate = decoded.spec().rate();
        if sample_rate.is_some_and(|existing| existing != rate) {
            return Err("媒体采样率在解码过程中发生变化，暂不支持".to_owned());
        }
        sample_rate = Some(rate);
        let channels = decoded.spec().channels().count();
        if channels == 0 {
            continue;
        }
        let mut interleaved = vec![f32::MID; decoded.samples_interleaved()];
        decoded.copy_to_slice_interleaved(&mut interleaved);
        mono.extend(
            interleaved
                .chunks_exact(channels)
                .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32),
        );
    }
    if mono.is_empty() {
        return Err("未解码到音频样本".to_owned());
    }
    Ok(resample_linear(
        &mono,
        sample_rate.unwrap_or(TARGET_SAMPLE_RATE),
        TARGET_SAMPLE_RATE,
    ))
}

fn resample_linear(input: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    if input.is_empty() || source_rate == 0 || target_rate == 0 {
        return Vec::new();
    }
    if source_rate == target_rate {
        return input.to_vec();
    }
    let output_len = input.len().saturating_mul(target_rate as usize) / source_rate as usize;
    let mut output = Vec::with_capacity(output_len);
    for index in 0..output_len {
        let position = index as f64 * source_rate as f64 / target_rate as f64;
        let left = position.floor() as usize;
        let right = (left + 1).min(input.len() - 1);
        let fraction = (position - left as f64) as f32;
        output.push(input[left] * (1.0 - fraction) + input[right] * fraction);
    }
    output
}

fn transcribe(
    model_path: &Path,
    audio: &[f32],
    language: Option<&str>,
    beam_size: i32,
    device: Device,
) -> Result<Vec<SubtitleSegment>, String> {
    let mut context_parameters = WhisperContextParameters::default();
    if device == Device::Cpu {
        context_parameters.use_gpu(false);
    }
    let context = WhisperContext::new_with_params(model_path, context_parameters)
        .map_err(|error| format!("加载 GGML 模型失败: {error}"))?;
    let mut state = context
        .create_state()
        .map_err(|error| format!("创建 Whisper 状态失败: {error}"))?;
    let mut parameters = FullParams::new(SamplingStrategy::BeamSearch {
        beam_size,
        patience: -1.0,
    });
    parameters.set_n_threads(
        std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1)
            .min(i32::MAX as usize) as i32,
    );
    parameters.set_language(language);
    parameters.set_translate(false);
    parameters.set_print_special(false);
    parameters.set_print_progress(false);
    parameters.set_print_realtime(false);
    parameters.set_print_timestamps(false);
    state
        .full(parameters, audio)
        .map_err(|error| format!("Whisper 推理失败: {error}"))?;
    Ok(state
        .as_iter()
        .filter_map(|segment| {
            let text = segment.to_string().trim().to_owned();
            (!text.is_empty()).then(|| SubtitleSegment {
                start: segment.start_timestamp() as f64 / 100.0,
                end: segment.end_timestamp() as f64 / 100.0,
                text,
            })
        })
        .collect())
}

fn resolve_output_path(
    media: &Path,
    output: Option<&Path>,
    format: SubtitleFormat,
    multiple: bool,
) -> PathBuf {
    let extension = format.extension();
    match output {
        None => media.with_extension(extension),
        Some(output) if multiple || output.is_dir() => output.join(
            media
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
                + "."
                + extension,
        ),
        Some(output) => output.to_owned(),
    }
}

fn write_subtitle(
    segments: &[SubtitleSegment],
    path: &Path,
    format: SubtitleFormat,
) -> Result<(), String> {
    let content = match format {
        SubtitleFormat::Srt => render_srt(segments),
        SubtitleFormat::Vtt => render_vtt(segments),
        SubtitleFormat::Txt => render_txt(segments),
        SubtitleFormat::Json => render_json(segments)?,
    };
    fs_utils::atomic_write(path, content.as_bytes()).map_err(|error| error.to_string())
}

fn render_srt(segments: &[SubtitleSegment]) -> String {
    let blocks: Vec<_> = segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            format!(
                "{}\n{} --> {}\n{}",
                index + 1,
                format_timestamp(segment.start, ','),
                format_timestamp(segment.end, ','),
                segment.text
            )
        })
        .collect();
    if blocks.is_empty() {
        String::new()
    } else {
        blocks.join("\n\n") + "\n"
    }
}

fn render_vtt(segments: &[SubtitleSegment]) -> String {
    let blocks: Vec<_> = segments
        .iter()
        .map(|segment| {
            format!(
                "{} --> {}\n{}",
                format_timestamp(segment.start, '.'),
                format_timestamp(segment.end, '.'),
                segment.text
            )
        })
        .collect();
    if blocks.is_empty() {
        "WEBVTT\n".to_owned()
    } else {
        format!("WEBVTT\n\n{}\n", blocks.join("\n\n"))
    }
}

fn render_txt(segments: &[SubtitleSegment]) -> String {
    if segments.is_empty() {
        String::new()
    } else {
        segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }
}

fn render_json(segments: &[SubtitleSegment]) -> Result<String, String> {
    let values: Vec<_> = segments
        .iter()
        .map(|segment| {
            json!({
                "start": segment.start, "end": segment.end, "text": segment.text
            })
        })
        .collect();
    serde_json::to_string_pretty(&values).map_err(|error| error.to_string())
}

fn format_timestamp(seconds: f64, separator: char) -> String {
    let total = (seconds.max(0.0) * 1_000.0).round() as u64;
    let milliseconds = total % 1_000;
    let total_seconds = total / 1_000;
    let seconds = total_seconds % 60;
    let total_minutes = total_seconds / 60;
    let minutes = total_minutes % 60;
    let hours = total_minutes / 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}{separator}{milliseconds:03}")
}

#[cfg(test)]
mod tests {
    use super::{
        SubtitleFormat, SubtitleSegment, decode_media, format_timestamp, model_alias, render_json,
        render_srt, render_txt, render_vtt, resample_linear, resolve_output_path,
    };
    use std::fs;
    use std::path::Path;

    fn segments() -> Vec<SubtitleSegment> {
        vec![SubtitleSegment {
            start: 1.25,
            end: 3.5,
            text: "你好".to_owned(),
        }]
    }

    #[test]
    fn renders_all_subtitle_formats() {
        assert!(render_srt(&segments()).contains("00:00:01,250 --> 00:00:03,500"));
        assert!(render_vtt(&segments()).starts_with("WEBVTT\n"));
        assert_eq!(render_txt(&segments()), "你好\n");
        assert!(
            render_json(&segments())
                .unwrap()
                .contains("\"start\": 1.25")
        );
        assert_eq!(format_timestamp(3661.002, '.'), "01:01:01.002");
    }

    #[test]
    fn resolves_single_and_batch_output_paths() {
        assert_eq!(
            resolve_output_path(Path::new("voice.mp3"), None, SubtitleFormat::Srt, false),
            Path::new("voice.srt")
        );
        assert_eq!(
            resolve_output_path(
                Path::new("voice.mp3"),
                Some(Path::new("out")),
                SubtitleFormat::Vtt,
                true
            ),
            Path::new("out/voice.vtt")
        );
    }

    #[test]
    fn resamples_linearly_to_sixteen_khz() {
        let input: Vec<_> = (0..8_000).map(|value| value as f32 / 8_000.0).collect();
        let output = resample_linear(&input, 8_000, 16_000);
        assert_eq!(output.len(), 16_000);
        assert!((output[2] - input[1]).abs() < 0.0001);
    }

    #[test]
    fn maps_legacy_default_to_native_model() {
        assert_eq!(model_alias("Systran/faster-whisper-small"), Some("small"));
        assert_eq!(model_alias("turbo"), Some("large-v3-turbo"));
    }

    #[test]
    fn decodes_and_resamples_pcm_wav() {
        let sample_rate = 8_000_u32;
        let samples: Vec<i16> = (0..800)
            .map(|index| if index % 2 == 0 { 1_000 } else { -1_000 })
            .collect();
        let data_size = (samples.len() * 2) as u32;
        let mut wav = Vec::with_capacity(44 + data_size as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_size).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16_u32.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&1_u16.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2_u16.to_le_bytes());
        wav.extend_from_slice(&16_u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_size.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        let path = std::env::temp_dir().join(format!(
            "douyin-subtitle-{}-{}.wav",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::write(&path, wav).unwrap();
        let audio = decode_media(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(audio.len(), 1_600);
        assert!(audio.iter().any(|sample| sample.abs() > 0.01));
    }
}
