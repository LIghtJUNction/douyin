"""Subtitle command."""

from __future__ import annotations

from pathlib import Path

import click

from douyin_cli.subtitles import (
    SubtitleDependencyError,
    SubtitleOptions,
    resolve_output_path,
    transcribe_media,
    write_subtitle,
)


@click.command("subtitle")
@click.argument(
    "media",
    nargs=-1,
    required=True,
    type=click.Path(
        exists=True,
        dir_okay=False,
        path_type=Path,
    ),
)
@click.option(
    "-o",
    "--output",
    type=click.Path(file_okay=True, dir_okay=True, path_type=Path),
    help="输出文件；批量处理时作为输出目录",
)
@click.option(
    "-f",
    "--format",
    "output_format",
    type=click.Choice(["srt", "vtt", "txt", "json"], case_sensitive=False),
    default="srt",
    show_default=True,
    help="字幕输出格式",
)
@click.option(
    "--model",
    default="small",
    show_default=True,
    help="Whisper 模型名称或路径",
)
@click.option("--language", help="语言代码，例如 zh/en；不传则自动识别")
@click.option(
    "--backend",
    type=click.Choice(["auto", "faster-whisper", "mlx-whisper"], case_sensitive=False),
    default="auto",
    show_default=True,
    help=(
        "识别后端: auto/faster-whisper/mlx-whisper；"
        "macOS Apple Silicon 默认使用 mlx-whisper"
    ),
)
@click.option(
    "--device",
    default="auto",
    show_default=True,
    help="faster-whisper 运行设备: auto/cpu/cuda",
)
@click.option(
    "--compute-type",
    default="default",
    show_default=True,
    help="计算精度，例如 default/int8/float16",
)
@click.option("--beam-size", default=5, show_default=True, help="解码 beam size")
@click.option(
    "--model-cache-dir",
    type=click.Path(file_okay=False, dir_okay=True, path_type=Path),
    help="Hugging Face 模型缓存目录",
)
@click.option(
    "--local-files-only",
    is_flag=True,
    help="只使用本地已缓存模型，不联网下载",
)
def subtitle(
    media: tuple[Path, ...],
    output: Path | None,
    output_format: str,
    model: str,
    language: str | None,
    backend: str,
    device: str,
    compute_type: str,
    beam_size: int,
    model_cache_dir: Path | None,
    local_files_only: bool,
) -> None:
    """从本地视频/音频生成字幕."""
    output_format = output_format.lower()
    options = SubtitleOptions(
        model=model,
        output_format=output_format,
        language=language,
        backend=backend,
        device=device,
        compute_type=compute_type,
        beam_size=beam_size,
        model_cache_dir=model_cache_dir,
        local_files_only=local_files_only,
    )
    multiple = len(media) > 1

    for media_path in media:
        output_path = resolve_output_path(
            media_path,
            output,
            output_format,
            multiple=multiple,
        )
        click.echo(f"正在生成字幕: {media_path}")
        if not local_files_only:
            click.echo(f"模型会在需要时自动从 Hugging Face 下载: {model}")
        try:
            segments = transcribe_media(media_path, options)
            write_subtitle(segments, output_path, output_format)
        except (SubtitleDependencyError, ValueError) as exc:
            raise click.ClickException(str(exc)) from exc
        click.echo(f"字幕已保存: {output_path}")
