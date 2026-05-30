"""配置管理模块

统一管理应用配置的加载、验证、保存功能。
提供全局 settings 实例供其他模块使用。
"""

import threading
import time
from collections.abc import Callable
from typing import Any, ClassVar

import ujson as json
from loguru import logger

from .paths import get_config_root, get_download_root

CONFIG_DIR = get_config_root() / "config"
SETTINGS_FILE = CONFIG_DIR / "settings.json"

DEFAULT_SETTINGS = {
    "cookie": "",
    "openapi": {
        "clientKey": "",
        "clientSecret": "",
        "redirectUri": "",
        "scopes": [],
        "accessToken": "",
        "refreshToken": "",
        "openId": "",
        "expiresIn": 0,
    },
    "userAgent": "",
    "downloadPath": str(get_download_root()),
    "enableIncrementalFetch": True,
    "enableDownloadTitle": False,
    "enableDownloadCover": False,
    "filenameFields": ["id", "title"],
    "filenameSeparator": "_",
}


class SettingsManager:
    """配置管理器

    负责配置文件的加载、验证、保存和备份。
    支持配置项的自动补全和损坏修复。
    """

    # 配置验证规则: key -> (validator, error_message)
    VALIDATORS: ClassVar[dict[str, tuple[Callable[[Any], bool], str]]] = {
        "cookie": (lambda x: isinstance(x, str), "必须是字符串"),
        "openapi": (lambda x: isinstance(x, dict), "必须是对象"),
        "userAgent": (lambda x: isinstance(x, str), "必须是字符串"),
        "downloadPath": (
            lambda x: isinstance(x, str) and len(x) > 0,
            "必须是非空字符串",
        ),
        "enableDownloadTitle": (lambda x: isinstance(x, bool), "必须是布尔值"),
        "enableDownloadCover": (lambda x: isinstance(x, bool), "必须是布尔值"),
        "filenameFields": (
            lambda x: isinstance(x, list) and all(isinstance(i, str) for i in x),
            "必须是字符串列表",
        ),
        "filenameSeparator": (
            lambda x: isinstance(x, str) and len(x) > 0,
            "必须是非空字符串",
        ),
    }

    def __init__(self, auto_load: bool = True) -> None:
        """初始化配置管理器

        Args:
            auto_load: 是否自动加载配置，默认 True

        """
        self._settings: dict[str, Any] = {}
        self._is_first_run: bool = not SETTINGS_FILE.exists()
        self._lock = threading.Lock()

        # 确保配置目录存在
        CONFIG_DIR.mkdir(parents=True, exist_ok=True)

        if auto_load:
            self.load()

    @property
    def is_first_run(self) -> bool:
        """是否首次运行"""
        return self._is_first_run

    @property
    def data(self) -> dict[str, Any]:
        """获取完整配置字典（只读副本）"""
        return self._settings.copy()

    def get(self, key: str, default: Any = None) -> Any:
        """获取配置项

        Args:
            key: 配置键名
            default: 默认值，如果未指定则从 DEFAULT_SETTINGS 获取

        Returns:
            配置值

        """
        self.ensure_loaded()
        if default is None:
            default = DEFAULT_SETTINGS.get(key)
        return self._settings.get(key, default)

    def ensure_loaded(self) -> None:
        """Load settings once when first needed."""
        if not self._settings:
            self.load(create_if_missing=False)

    def load(self, *, create_if_missing: bool = True) -> dict[str, Any]:
        """加载配置文件

        Returns:
            加载后的配置字典

        """
        settings_path = SETTINGS_FILE
        if not settings_path.exists():
            self._settings = DEFAULT_SETTINGS.copy()
            if create_if_missing:
                self._save_file()
            return self._settings

        try:
            with settings_path.open(encoding="utf-8") as f:
                self._settings = json.load(f)
        except json.JSONDecodeError:
            logger.error("✗ 配置文件损坏，备份并重置")
            self._backup_file()
            self._settings = DEFAULT_SETTINGS.copy()
            self._save_file()
            return self._settings
        except Exception as e:
            logger.error(f"✗ 加载配置失败: {e}")
            self._settings = DEFAULT_SETTINGS.copy()
            return self._settings

        # 验证修复 + 补充缺失
        self._repair_and_complete()

        return self._settings

    def save(self, updates: dict[str, Any]) -> None:
        self.ensure_loaded()
        is_valid, errors = self._validate(updates)
        if not is_valid:
            raise ValueError("配置验证失败:\n" + "\n".join(f"  - {e}" for e in errors))

        with self._lock:
            self._settings.update(updates)
            CONFIG_DIR.mkdir(parents=True, exist_ok=True)
            self._save_file()

    def _validate(self, data: dict[str, Any]) -> tuple[bool, list[str]]:
        """验证配置项"""
        errors = []
        for key, value in data.items():
            if key in self.VALIDATORS:
                check, msg = self.VALIDATORS[key]
                try:
                    if not check(value):
                        errors.append(f"{key}: {msg}")
                except Exception as e:
                    errors.append(f"{key}: 验证出错 - {e}")
        return len(errors) == 0, errors

    def _repair_and_complete(self) -> None:
        need_save = False

        for key, (check, _msg) in self.VALIDATORS.items():
            if key in self._settings:
                try:
                    if not check(self._settings[key]):
                        self._settings[key] = DEFAULT_SETTINGS[key]
                        logger.warning(f"  - 已修复 {key}")
                        need_save = True
                except Exception:
                    self._settings[key] = DEFAULT_SETTINGS[key]
                    logger.warning(f"  - 已修复 {key}")
                    need_save = True

        # 补充缺失项
        for key, default in DEFAULT_SETTINGS.items():
            if key not in self._settings:
                self._settings[key] = default
                need_save = True

        if need_save:
            self._save_file()

    def _save_file(self) -> None:
        """保存配置到文件"""
        with SETTINGS_FILE.open("w", encoding="utf-8") as f:
            json.dump(self._settings, f, ensure_ascii=False, indent=2)

    def _backup_file(self) -> None:
        """备份配置文件"""
        settings_path = SETTINGS_FILE
        if settings_path.exists():
            backup = settings_path.with_name(
                f"{settings_path.name}.backup.{int(time.time())}",
            )
            try:
                settings_path.rename(backup)
                logger.info(f"  - 已备份到: {backup}")
            except Exception as e:
                logger.warning(f"  - 备份失败: {e}")


# 全局实例
settings = SettingsManager(auto_load=False)
