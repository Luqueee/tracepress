#!/usr/bin/env python3
"""Merge identity-bound scheduler sidecars into an analyzer-compatible document."""

from __future__ import annotations

import argparse
import importlib.util
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parent
COLLECTOR_PATH = ROOT / "collect_scheduler_sidecar.py"
SPEC = importlib.util.spec_from_file_location("collect_scheduler_sidecar", COLLECTOR_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {COLLECTOR_PATH}")
COLLECTOR = importlib.util.module_from_spec(SPEC)
sys.modules[COLLECTOR_PATH.stem] = COLLECTOR
SPEC.loader.exec_module(COLLECTOR)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("sidecars", nargs="+", type=Path)
    return parser


def main(arguments: list[str] | None = None) -> int:
    options = build_parser().parse_args(arguments)
    try:
        merged = COLLECTOR.merge_sidecars(options.sidecars)
        COLLECTOR.write_atomic_json(options.output, merged)
    except (OSError, ValueError) as error:
        print(f"sidecar merge failed: {error}", file=sys.stderr)
        return 1
    print(f"merged_sidecar={options.output}")
    print(f"sessions={len(merged['sessions'])}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
