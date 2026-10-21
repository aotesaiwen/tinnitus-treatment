#!/usr/bin/env python3
"""Collect optional music-provider credentials without echoing or committing them."""

import argparse
import getpass
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import warnings


ROOT = Path(__file__).resolve().parents[1]
CREDENTIALS = ROOT / ".runtime/music-access/credentials.json"
PROVIDERS = (
    ("HF_TOKEN", "Hugging Face", "https://huggingface.co/settings/tokens"),
    ("GEMINI_API_KEY", "Google AI Studio", "https://aistudio.google.com/apikey"),
    ("ELEVENLABS_API_KEY", "ElevenLabs", "https://elevenlabs.io/app/api/api-keys"),
)


def load_credentials(path=CREDENTIALS):
    if path.is_symlink():
        raise ValueError("密钥文件不能是符号链接。")
    if not path.exists():
        return {}
    mode = path.stat()
    if mode.st_uid != os.getuid() or stat.S_IMODE(mode.st_mode) & 0o077:
        raise ValueError("密钥文件必须由当前用户拥有，且权限为 600。")
    data = json.loads(path.read_text(encoding="utf-8"))
    allowed = {key for key, _, _ in PROVIDERS}
    if not isinstance(data, dict) or any(
        key not in allowed or not isinstance(value, str) or not value
        or any(character.isspace() for character in value)
        for key, value in data.items()
    ):
        raise ValueError("密钥文件格式不正确；未显示文件内容。")
    return data


def save_credentials(data, path=CREDENTIALS):
    if path.parent.is_symlink() or path.is_symlink():
        raise ValueError("密钥目录和文件不能是符号链接。")
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    os.chmod(path.parent, 0o700)
    descriptor, temporary = tempfile.mkstemp(prefix=".credentials-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            os.fchmod(handle.fileno(), 0o600)
            json.dump(data, handle)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def show_status(data):
    for key, label, _ in PROVIDERS:
        present = bool(data.get(key) or os.environ.get(key))
        print(f"{label}：{'已配置，尚未验证调用权限' if present else '未配置'}")


def main():
    parser = argparse.ArgumentParser(description="音乐模型本机密钥配置；输入不会显示。")
    parser.add_argument("--status", action="store_true", help="仅显示是否配置，不显示密钥")
    args = parser.parse_args()
    data = load_credentials()
    if args.status:
        show_status(data)
        return
    if not sys.stdin.isatty():
        raise ValueError("请在交互式终端运行，不能通过管道或聊天传入密钥。")
    print("请粘贴 API key / token，然后按回车。粘贴时不显示字符是正常的。")
    print("直接回车：保留已有配置，或跳过尚未开通的平台。")
    print("只保存在本机被 Git 忽略的 .runtime/music-access/credentials.json；不会生成音乐或扣费。")
    print("Hugging Face 创建 Read token；还需在 Stable Audio 模型页面同意访问条款。")
    print("ElevenLabs 音乐 API 要求付费账号，免费账号可先跳过。\n")
    with warnings.catch_warnings():
        warnings.simplefilter("error", getpass.GetPassWarning)
        for key, label, url in PROVIDERS:
            print(f"{label} 密钥页面：{url}")
            while True:
                value = getpass.getpass(f"{label} 密钥（隐藏输入）：").strip()
                if not value:
                    break
                if any(character.isspace() for character in value):
                    print("密钥中不应包含空格或换行，请重新输入。")
                    continue
                data[key] = value
                save_credentials(data)
                print("已保存。")
                break
    print()
    show_status(data)
    print("配置完成。可以返回聊天回复“已配置”，不要发送密钥。")


if __name__ == "__main__":
    try:
        main()
    except (KeyboardInterrupt, EOFError):
        print("\n已结束；此前保存的配置仍然有效。", file=sys.stderr)
        sys.exit(130)
    except (OSError, ValueError, getpass.GetPassWarning):
        print("无法安全读取或保存密钥，请检查文件权限并在交互式终端运行。未显示密钥。", file=sys.stderr)
        sys.exit(1)
