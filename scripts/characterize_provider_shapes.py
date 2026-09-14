#!/usr/bin/env python3
"""Metadata-only characterization for Phase 4.2.1.

The script deliberately never selects context bytes, fingerprints, tool names, or raw
payloads. It is safe to point at an operational database opened read-only. Shape
labels are derived from the bounded fields already produced by Context Analysis.
"""

from __future__ import annotations

import argparse
import json
import sqlite3
from pathlib import Path


def bucket(value: int | None) -> str:
    if value is None:
        return "unavailable"
    if value == 0:
        return "0"
    if value == 1:
        return "1"
    if value < 10:
        return "2-9"
    if value < 100:
        return "10-99"
    if value < 1000:
        return "100-999"
    return "1000+"


def characterize(database: Path, session_limit: int) -> dict:
    uri = f"file:{database}?mode=ro"
    connection = sqlite3.connect(uri, uri=True)
    connection.execute("PRAGMA query_only=ON")
    session_ids = [
        row[0]
        for row in connection.execute(
            "SELECT session_id FROM sessions ORDER BY started_at, session_id LIMIT ?",
            (session_limit,),
        )
    ]
    if not session_ids:
        return {"sessions": 0, "blocks": 0, "json": {}, "plain_text": {}, "candidate_forecast": {}}
    placeholders = ",".join("?" for _ in session_ids)
    rows = connection.execute(
        f"""SELECT cbo.detected_kind, cbo.raw_bytes, cbo.estimated_tokens,
                   cbo.line_count, cbo.duplicate_line_ratio, cbo.json_item_count,
                   cbo.json_depth, cbo.origin, cbo.kind
            FROM context_block_occurrences cbo
            JOIN context_snapshots cs ON cs.snapshot_id = cbo.snapshot_id
            WHERE cs.session_id IN ({placeholders})
              AND cbo.origin = 'tool_generated'
              AND cbo.kind = 'tool_result'""",
        session_ids,
    ).fetchall()
    json_shapes: dict[str, dict[str, int]] = {}
    text_shapes: dict[str, dict[str, int]] = {}
    total_json_tokens = 0
    total_text_tokens = 0
    for detected, raw_bytes, estimated_tokens, line_count, duplicate_ratio, item_count, depth, *_ in rows:
        tokens = int(estimated_tokens or 0)
        if detected == "json":
            if item_count is None:
                shape = "object_or_scalar"
            elif depth and depth > 1:
                shape = "nested_array_or_object"
            else:
                shape = "array_or_object"
            key = f"{shape}|items={bucket(item_count)}|depth={bucket(depth)}"
            json_shapes.setdefault(key, {"blocks": 0, "estimated_tokens": 0, "raw_bytes": 0})
            json_shapes[key]["blocks"] += 1
            json_shapes[key]["estimated_tokens"] += tokens
            json_shapes[key]["raw_bytes"] += int(raw_bytes or 0)
            total_json_tokens += tokens
        elif detected == "plain_text":
            if duplicate_ratio is not None and duplicate_ratio > 0:
                shape = "duplicate_lines"
            elif line_count and line_count > 1:
                shape = "multi_line"
            else:
                shape = "single_line_or_empty"
            text_shapes.setdefault(shape, {"blocks": 0, "estimated_tokens": 0, "raw_bytes": 0})
            text_shapes[shape]["blocks"] += 1
            text_shapes[shape]["estimated_tokens"] += tokens
            text_shapes[shape]["raw_bytes"] += int(raw_bytes or 0)
            total_text_tokens += tokens
    forecast: dict[str, dict[str, int]] = {}
    if connection.execute(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='compression_candidates')"
    ).fetchone()[0]:
        for compressor, eligible, applicable, applicable_tokens in connection.execute(
            f"""SELECT c.compressor_id, COUNT(*), SUM(c.status='applicable'),
                       SUM(CASE WHEN c.status='applicable' THEN m.input_estimated_tokens ELSE 0 END)
                FROM compression_candidates c
                JOIN compression_candidate_metrics m ON m.candidate_id = c.candidate_id
                JOIN context_snapshots cs ON cs.snapshot_id = c.snapshot_id
                WHERE cs.session_id IN ({placeholders})
                GROUP BY c.compressor_id ORDER BY c.compressor_id""",
            session_ids,
        ):
            forecast[compressor] = {
                "eligible_blocks": int(eligible or 0),
                "applicable_blocks": int(applicable or 0),
                "applicable_estimated_tokens": int(applicable_tokens or 0),
            }
    connection.close()
    return {
        "sessions": len(session_ids),
        "blocks": len(rows),
        "json": {"total_estimated_tokens": total_json_tokens, "shapes": json_shapes},
        "plain_text": {"total_estimated_tokens": total_text_tokens, "shapes": text_shapes},
        "candidate_forecast": forecast,
        "privacy": "metadata_only_raw_content_not_selected",
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", type=Path, required=True)
    parser.add_argument("--sessions", type=int, default=10)
    parser.add_argument("--json", type=Path, required=True)
    parser.add_argument("--markdown", type=Path, required=True)
    args = parser.parse_args()
    result = characterize(args.db, max(1, min(args.sessions, 10_000)))
    args.json.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    lines = [
        "# Provider-Compatible Shape Characterization",
        "",
        f"Sessions: **{result['sessions']}**  ",
        f"ToolResult blocks: **{result['blocks']}**",
        "",
        "This report is metadata-only. Raw context, tool arguments, response bodies, and fingerprints are not selected or persisted.",
        "",
        "## JSON shapes",
        "",
    ]
    for name, values in result["json"]["shapes"].items():
        lines.append(f"- `{name}`: {values['blocks']} blocks, {values['estimated_tokens']} estimated tokens")
    lines += ["", "## Plain-text shapes", ""]
    for name, values in result["plain_text"]["shapes"].items():
        lines.append(f"- `{name}`: {values['blocks']} blocks, {values['estimated_tokens']} estimated tokens")
    lines += ["", "## Candidate forecast", ""]
    if result["candidate_forecast"]:
        for name, values in result["candidate_forecast"].items():
            lines.append(f"- `{name}`: {values['applicable_blocks']}/{values['eligible_blocks']} applicable")
    else:
        lines.append("No persisted shadow candidate data is available for this cohort.")
    args.markdown.write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
