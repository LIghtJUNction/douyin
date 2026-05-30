"""Path helpers for user-owned app data."""

import os
import sys
from pathlib import Path


def _first_writable_dir(candidates: list[Path]) -> Path:
    """Return the first candidate whose parent directory is writable."""
    for candidate in candidates:
        try:
            candidate.mkdir(parents=True, exist_ok=True)
            return candidate
        except OSError:
            continue
    return Path("/tmp/douyin-cli")


APP_DIR_NAME = "douyin-cli"


def get_config_root() -> Path:
    """Return the user configuration root."""
    if douyin_home := os.environ.get("DOUYIN_HOME"):
        return Path(douyin_home).expanduser()

    if sys.platform.startswith("win"):
        return _first_writable_dir(
            [
                Path(os.environ.get("APPDATA", "")) / APP_DIR_NAME,
                Path(os.environ.get("LOCALAPPDATA", "")) / APP_DIR_NAME,
                Path.home() / f".{APP_DIR_NAME}",
            ],
        )

    config_home = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    return _first_writable_dir(
        [
            config_home / APP_DIR_NAME,
            Path.home() / f".{APP_DIR_NAME}",
        ],
    )


def get_download_root() -> Path:
    """Return the default download directory for douyin files."""
    return _system_downloads_dir() / "douyin"


def _system_downloads_dir() -> Path:
    if sys.platform.startswith("win"):
        return Path(os.environ.get("USERPROFILE", str(Path.home()))) / "Downloads"

    if xdg_download_dir := _read_xdg_user_dir("XDG_DOWNLOAD_DIR"):
        return xdg_download_dir

    return Path.home() / "Downloads"


def _read_xdg_user_dir(key: str) -> Path | None:
    config_home = Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config"))
    user_dirs_file = config_home / "user-dirs.dirs"
    if not user_dirs_file.exists():
        return None

    home = str(Path.home())
    prefix = f"{key}="
    for raw_line in user_dirs_file.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line.startswith(prefix):
            continue
        value = line.removeprefix(prefix).strip().strip('"')
        return Path(value.replace("$HOME", home))
    return None
