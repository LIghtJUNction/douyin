"""Subtitle generation with optional Whisper backends."""

from __future__ import annotations

import ctypes
import platform
from dataclasses import dataclass
from importlib.resources import files
from pathlib import Path
from typing import Any

import ujson as json


@dataclass(frozen=True)
class SubtitleOptions:
    model: str
    output_format: str
    language: str | None
    backend: str
    device: str
    compute_type: str
    beam_size: int
    model_cache_dir: Path | None
    local_files_only: bool


@dataclass(frozen=True)
class SubtitleSegment:
    start: float
    end: float
    text: str


class SubtitleDependencyError(RuntimeError):
    """Raised when optional subtitle dependencies are missing."""


CUDA_LIBRARY_HINT = (
    "CUDA 字幕依赖不完整。当前 faster-whisper/ctranslate2 需要 CUDA 12 "
    "运行库。请使用: uv tool install 'douyin-cli[subtitle-cuda]'；"
    "或改用 CPU: douyin subtitle video.mp4 --device cpu --compute-type int8"
)
CUDA_LIBRARY_PACKAGES = {
    "nvidia.cublas.lib": ("libcublas.so.12", "libcublasLt.so.12"),
    "nvidia.cuda_nvrtc.lib": ("libnvrtc.so.12",),
    "nvidia.cudnn.lib": ("libcudnn.so.9",),
}
CUDA_LINK_ERROR_MARKERS = (
    "libcublas.so.12",
    "libcublasLt.so.12",
    "libcudnn",
    "CUDA failed",
    "CUDA driver",
    "cuBLAS",
    "cuDNN",
)
MACOS_MLX_INSTALL_HINT = (
    "缺少 macOS Apple Silicon GPU 字幕依赖 mlx-whisper。请使用: "
    "uv tool install 'douyin-cli[subtitle-mac]'；"
    "或改用 CPU: douyin subtitle video.mp4 --backend faster-whisper "
    "--device cpu --compute-type int8"
)
MLX_MODEL_ALIASES = {
    "tiny": "mlx-community/whisper-tiny",
    "base": "mlx-community/whisper-base",
    "small": "mlx-community/whisper-small",
    "medium": "mlx-community/whisper-medium",
    "large": "mlx-community/whisper-large-v3-mlx",
    "large-v3": "mlx-community/whisper-large-v3-mlx",
    "turbo": "mlx-community/whisper-turbo",
}


def transcribe_media(
    media_path: Path,
    options: SubtitleOptions,
) -> list[SubtitleSegment]:
    """Generate timestamped subtitle segments from a media file."""
    backend = resolve_subtitle_backend(options.backend)
    if backend == "mlx-whisper":
        return transcribe_media_with_mlx(media_path, options)
    return transcribe_media_with_faster_whisper(media_path, options)


def resolve_subtitle_backend(backend: str) -> str:
    """Resolve auto backend selection for the current platform."""
    if backend != "auto":
        return backend
    if platform.system() == "Darwin" and platform.machine() == "arm64":
        return "mlx-whisper"
    return "faster-whisper"


def transcribe_media_with_faster_whisper(
    media_path: Path,
    options: SubtitleOptions,
) -> list[SubtitleSegment]:
    """Generate subtitle segments with faster-whisper/CTranslate2."""
    prepare_cuda_libraries(options.device)
    try:
        from faster_whisper import WhisperModel
    except ImportError as exc:
        msg = (
            "缺少字幕依赖 faster-whisper。请使用: "
            "uv tool install 'douyin-cli[subtitle]'"
        )
        raise SubtitleDependencyError(msg) from exc

    try:
        model = WhisperModel(
            options.model,
            device=options.device,
            compute_type=options.compute_type,
            download_root=(
                str(options.model_cache_dir) if options.model_cache_dir else None
            ),
            local_files_only=options.local_files_only,
        )
        segments, _info = model.transcribe(
            str(media_path),
            language=options.language,
            beam_size=options.beam_size,
        )
    except (OSError, RuntimeError) as exc:
        if is_cuda_link_error(exc):
            raise SubtitleDependencyError(CUDA_LIBRARY_HINT) from exc
        raise

    return [
        SubtitleSegment(
            start=segment.start,
            end=segment.end,
            text=segment.text.strip(),
        )
        for segment in segments
        if segment.text.strip()
    ]


def transcribe_media_with_mlx(
    media_path: Path,
    options: SubtitleOptions,
) -> list[SubtitleSegment]:
    """Generate subtitle segments with mlx-whisper on Apple Silicon."""
    try:
        import mlx_whisper
    except ImportError as exc:
        raise SubtitleDependencyError(MACOS_MLX_INSTALL_HINT) from exc

    model = resolve_mlx_model(options.model)
    try:
        result = mlx_whisper.transcribe(
            str(media_path),
            path_or_hf_repo=model,
            language=options.language,
        )
    except TypeError:
        result = mlx_whisper.transcribe(str(media_path), path_or_hf_repo=model)

    raw_segments = result.get("segments") or []
    return [
        SubtitleSegment(
            start=float(segment["start"]),
            end=float(segment["end"]),
            text=str(segment["text"]).strip(),
        )
        for segment in raw_segments
        if str(segment.get("text", "")).strip()
    ]


def resolve_mlx_model(model: str) -> str:
    """Map common Whisper model names to MLX Community checkpoints."""
    return MLX_MODEL_ALIASES.get(model, model)


def prepare_cuda_libraries(device: str) -> None:
    """Preload CUDA 12 libraries from optional nvidia wheels when available."""
    if device == "cpu":
        return

    for package, library_names in CUDA_LIBRARY_PACKAGES.items():
        try:
            package_files = files(package)
        except ModuleNotFoundError:
            continue

        for library_name in library_names:
            library_path = package_files / library_name
            if not library_path.is_file():
                continue
            try:
                ctypes.CDLL(str(library_path), mode=ctypes.RTLD_GLOBAL)
            except OSError as exc:
                raise SubtitleDependencyError(CUDA_LIBRARY_HINT) from exc


def is_cuda_link_error(exc: BaseException) -> bool:
    """Return whether an exception looks like a CUDA runtime link failure."""
    message = str(exc)
    return any(marker in message for marker in CUDA_LINK_ERROR_MARKERS)


def write_subtitle(
    segments: list[SubtitleSegment],
    output_path: Path,
    output_format: str,
) -> None:
    """Write subtitle segments to disk."""
    output_path.parent.mkdir(parents=True, exist_ok=True)
    if output_format == "srt":
        content = render_srt(segments)
    elif output_format == "vtt":
        content = render_vtt(segments)
    elif output_format == "txt":
        content = render_txt(segments)
    elif output_format == "json":
        content = render_json(segments)
    else:
        msg = f"不支持的字幕格式: {output_format}"
        raise ValueError(msg)
    output_path.write_text(content, encoding="utf-8")


def default_output_path(media_path: Path, output_format: str) -> Path:
    """Return the default subtitle path next to the media file."""
    return media_path.with_suffix(f".{output_format}")


def resolve_output_path(
    media_path: Path,
    output: Path | None,
    output_format: str,
    *,
    multiple: bool,
) -> Path:
    """Resolve output file path for one media file."""
    if output is None:
        return default_output_path(media_path, output_format)

    if multiple or output.is_dir():
        return output / f"{media_path.stem}.{output_format}"

    return output


def render_srt(segments: list[SubtitleSegment]) -> str:
    """Render segments as SRT."""
    blocks = []
    for index, segment in enumerate(segments, 1):
        blocks.append(
            "\n".join(
                [
                    str(index),
                    f"{format_srt_time(segment.start)} --> "
                    f"{format_srt_time(segment.end)}",
                    segment.text,
                ],
            ),
        )
    return "\n\n".join(blocks) + ("\n" if blocks else "")


def render_vtt(segments: list[SubtitleSegment]) -> str:
    """Render segments as WebVTT."""
    blocks = ["WEBVTT"]
    blocks.extend(
        (
            "\n".join(
                [
                    f"{format_vtt_time(segment.start)} --> "
                    f"{format_vtt_time(segment.end)}",
                    segment.text,
                ],
            )
        )
        for segment in segments
    )
    return "\n\n".join(blocks) + "\n"


def render_txt(segments: list[SubtitleSegment]) -> str:
    """Render segments as plain text."""
    return "\n".join(segment.text for segment in segments) + ("\n" if segments else "")


def render_json(segments: list[SubtitleSegment]) -> str:
    """Render segments as JSON."""
    data: list[dict[str, Any]] = [
        {"start": segment.start, "end": segment.end, "text": segment.text}
        for segment in segments
    ]
    return json.dumps(data, ensure_ascii=False, indent=2)


def format_srt_time(seconds: float) -> str:
    """Format seconds as SRT timestamp."""
    hours, minutes, whole_seconds, milliseconds = split_timestamp(seconds)
    return f"{hours:02d}:{minutes:02d}:{whole_seconds:02d},{milliseconds:03d}"


def format_vtt_time(seconds: float) -> str:
    """Format seconds as WebVTT timestamp."""
    hours, minutes, whole_seconds, milliseconds = split_timestamp(seconds)
    return f"{hours:02d}:{minutes:02d}:{whole_seconds:02d}.{milliseconds:03d}"


def split_timestamp(seconds: float) -> tuple[int, int, int, int]:
    """Split seconds into timestamp parts."""
    milliseconds_total = max(round(seconds * 1000), 0)
    whole_seconds, milliseconds = divmod(milliseconds_total, 1000)
    minutes_total, whole_seconds = divmod(whole_seconds, 60)
    hours, minutes = divmod(minutes_total, 60)
    return hours, minutes, whole_seconds, milliseconds
