#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import plistlib
import shutil
import sys
from pathlib import Path

PROFILE_NAME = "Rasputin OCR"
FONT_DISPLAY_NAME = "OCR-A BT"
FONT_FILENAME = "OCR-A-BT.ttf"
FONT_SIZE = 14.0
SYSTEM_PROFILE_CANDIDATES = (
    Path(
        "/System/Applications/Utilities/Terminal.app/Contents/Resources/Initial Settings/Pro.terminal"
    ),
    Path(
        "/Applications/Utilities/Terminal.app/Contents/Resources/Initial Settings/Pro.terminal"
    ),
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Install Rasputin's bundled OCR font and Terminal profile."
    )
    parser.add_argument("--quiet", action="store_true", help="Suppress non-error output.")
    parser.add_argument(
        "--prefs-path",
        type=Path,
        default=Path("~/Library/Preferences/com.apple.Terminal.plist").expanduser(),
        help="Override the Terminal preferences plist path.",
    )
    parser.add_argument(
        "--fonts-dir",
        type=Path,
        default=Path("~/Library/Fonts").expanduser(),
        help="Override the destination font directory.",
    )
    return parser.parse_args()


def repo_root() -> Path:
    return Path(__file__).resolve().parent.parent


def resolve_font_source(root: Path) -> Path:
    candidates = (
        root / "assets" / "fonts" / FONT_FILENAME,
        root / "OCR-a___.ttf",
    )
    for candidate in candidates:
        if candidate.exists():
            return candidate
    raise FileNotFoundError("Bundled OCR font asset is missing from the repository.")


def load_base_profile() -> dict:
    for candidate in SYSTEM_PROFILE_CANDIDATES:
        if candidate.exists():
            return plistlib.loads(candidate.read_bytes())
    raise FileNotFoundError("macOS Terminal Pro profile was not found on this system.")


def build_font_archive() -> bytes:
    font_archive = {
        "$version": 100000,
        "$objects": [
            "$null",
            {
                "$class": plistlib.UID(3),
                "NSfFlags": 16,
                "NSName": plistlib.UID(2),
                "NSSize": FONT_SIZE,
            },
            FONT_DISPLAY_NAME,
            {
                "$classname": "NSFont",
                "$classes": ["NSFont", "NSObject"],
            },
        ],
        "$archiver": "NSKeyedArchiver",
        "$top": {"root": plistlib.UID(1)},
    }
    return plistlib.dumps(font_archive, fmt=plistlib.FMT_BINARY, sort_keys=False)


def build_profile() -> dict:
    profile = load_base_profile()
    profile["name"] = PROFILE_NAME
    profile["Font"] = build_font_archive()
    return profile


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def install_font(font_src: Path, fonts_dir: Path) -> tuple[Path, bool]:
    fonts_dir.mkdir(parents=True, exist_ok=True)
    font_dest = fonts_dir / FONT_FILENAME

    changed = not font_dest.exists() or sha256(font_src) != sha256(font_dest)
    if changed:
        shutil.copy2(font_src, font_dest)

    return font_dest, changed


def install_profile(profile: dict, prefs_path: Path) -> bool:
    prefs = {}
    if prefs_path.exists():
        prefs = plistlib.loads(prefs_path.read_bytes())

    window_settings = dict(prefs.get("Window Settings", {}))
    changed = window_settings.get(PROFILE_NAME) != profile
    if not changed:
        return False

    window_settings[PROFILE_NAME] = profile
    prefs["Window Settings"] = window_settings

    prefs_path.parent.mkdir(parents=True, exist_ok=True)
    prefs_path.write_bytes(
        plistlib.dumps(prefs, fmt=plistlib.FMT_BINARY, sort_keys=False)
    )
    return True


def log(message: str, quiet: bool) -> None:
    if not quiet:
        print(message)


def main() -> int:
    args = parse_args()

    if sys.platform != "darwin":
        if not args.quiet:
            print("Skipping Terminal profile install: macOS Terminal.app is required.")
        return 0

    try:
        font_src = resolve_font_source(repo_root())
        font_dest, font_changed = install_font(font_src, args.fonts_dir)
        profile_changed = install_profile(build_profile(), args.prefs_path)
    except Exception as exc:  # pragma: no cover - launcher fallback handles failures
        print(f"Failed to install Rasputin Terminal assets: {exc}", file=sys.stderr)
        return 1

    if font_changed:
        log(f"Installed OCR font to {font_dest}", args.quiet)
    else:
        log(f"OCR font already installed at {font_dest}", args.quiet)

    if profile_changed:
        log(f"Registered Terminal profile '{PROFILE_NAME}'", args.quiet)
        log("Restart Terminal if it was already open to pick up the new profile.", args.quiet)
    else:
        log(f"Terminal profile '{PROFILE_NAME}' is already up to date", args.quiet)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
