from pathlib import Path

from douyin_cli.subtitles import (
    SubtitleSegment,
    default_output_path,
    format_srt_time,
    is_cuda_link_error,
    render_srt,
    render_vtt,
    resolve_output_path,
)


def test_format_srt_time_rounds_milliseconds() -> None:
    assert format_srt_time(3661.2345) == "01:01:01,234"


def test_render_srt() -> None:
    segments = [
        SubtitleSegment(start=0, end=1.5, text="你好"),
        SubtitleSegment(start=1.5, end=3, text="世界"),
    ]

    assert render_srt(segments) == (
        "1\n"
        "00:00:00,000 --> 00:00:01,500\n"
        "你好\n\n"
        "2\n"
        "00:00:01,500 --> 00:00:03,000\n"
        "世界\n"
    )


def test_render_vtt() -> None:
    segments = [SubtitleSegment(start=0, end=1.5, text="hello")]

    assert render_vtt(segments) == ("WEBVTT\n\n00:00:00.000 --> 00:00:01.500\nhello\n")


def test_resolve_output_path_for_batch_output_dir() -> None:
    output = resolve_output_path(
        Path("video.mp4"),
        Path("subtitles"),
        "srt",
        multiple=True,
    )

    assert output == Path("subtitles/video.srt")


def test_default_output_path_uses_requested_format() -> None:
    assert default_output_path(Path("video.mp4"), "vtt") == Path("video.vtt")


def test_is_cuda_link_error_detects_missing_cublas() -> None:
    error = OSError("libcublas.so.12: cannot open shared object file")

    assert is_cuda_link_error(error)
