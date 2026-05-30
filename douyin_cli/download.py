import subprocess
from pathlib import Path
from shutil import which

from loguru import logger


def download(path: str, aria2_conf: str) -> None:
    if Path(aria2_conf).exists():
        logger.info("开始下载")

        aria2_path = which("aria2c")

        if not aria2_path:
            logger.error("未找到 aria2c 可执行文件")
            logger.error("请先自行安装 aria2，并确保 aria2c 在 PATH 中")
            return

        logger.info(f"使用 aria2c: {aria2_path}")

        command = [
            aria2_path,
            "-c",
            "--console-log-level",
            "warn",
            "-d",
            path,
            "-i",
            aria2_conf,
        ]
        try:
            result = subprocess.run(command, check=False, timeout=3600)
            if result.returncode != 0:
                logger.error(f"aria2c 下载失败，退出码: {result.returncode}")
        except subprocess.TimeoutExpired:
            logger.error("aria2c 下载超时（1小时），已终止")
        except Exception as e:
            logger.error(f"aria2c 执行异常: {e}")
    else:
        logger.error(f"没有发现可下载的配置文件: {aria2_conf}")
