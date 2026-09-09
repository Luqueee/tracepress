#!/usr/bin/env python3
from pathlib import Path
import sys
import re


REQUIRED_HEADINGS = (
    "## Critical review and resolutions",
    "## Architectural invariants",
    "## Bounded resource contract",
    "## SQLite v1 schema",
    "## Principal Rust traits",
    "## Exact request flow",
    "## Crash consistency contracts",
    "## Fail-open contracts",
    "## Privacy and redaction boundary",
    "## Scope exclusions",
)
SECTION_REQUIREMENTS = {
    "## Critical review and resolutions": ("Resolution:", "Phase 1"),
    "## Architectural invariants": ("UUIDv7", "SHA-256(raw bytes)", "SQL NULL", "frozen", "single daemon writer"),
    "## SQLite v1 schema": ("handwritten, numbered migrations", "WAL", "foreign_keys", "busy_timeout"),
    "## Principal Rust traits": ("trait IpcTransport", "trait BlobStore", "trait StorageWriter", "trait ProviderForwarder"),
    "## Exact request flow": (
        "OpenAI-compatible `/v1/chat/completions`", "request body bytes are opaque", "response body bytes are opaque",
        "no Phase 1 provider usage capture", "no semantic response inspection", "deterministically rejects an oversized request",
        "deterministically terminates an oversized response", "incomplete",
    ),
    "## Crash consistency contracts": (
        "foreign-key-valid atomic transaction", "raw object", "decision", "recovery mapping", "binding", "event",
        "committed before output eligibility", "stale session", "misleading success",
    ),
    "## Fail-open contracts": ("forward the original request", "unchanged body bytes", "CAS failure", "SQLite", "queue full"),
    "## Bounded resource contract": (
        "max_request_body_bytes", "max_response_body_bytes", "max_ipc_frame_bytes", "max_ipc_queue_items",
        "max_processing_time_ms", "max_line_bytes", "max_json_nesting", "max_json_items",
        "Startup rejects missing or non-positive limits", "deterministically rejected",
        "deterministically terminated", "incomplete", "No path buffers infinite output",
    ),
    "## Privacy and redaction boundary": (
        "Raw bytes remain local", "metadata only", "Authorization", "Bearer", "API keys", "cookies",
        "AWS secrets", "GitHub tokens", "OpenAI keys", "Anthropic keys", "environment values",
        "absolute paths", "prompts", "tool outputs", "source code", "Never persist complete provider headers",
        "HMAC-SHA-256",
    ),
    "## Scope exclusions": ("no lossy transformation", "no compression"),
}
REQUIRED_TABLES = (
    "schema_metadata", "sessions", "operations", "causal_edges", "provider_requests", "provider_attempts",
    "provider_usage", "content_objects", "content_occurrences", "content_bindings", "compression_decisions",
    "recoveries", "policy_assignments", "evaluations", "events",
)
FORBIDDEN_MARKERS = ("TODO", "FIXME", "TBD", "unresolved decision")
REQUIRED_HEADING_RE = re.compile(r"^## .+$", re.MULTILINE)


def check_text(text: str, path: Path) -> list[str]:
    if not path.is_file():
        return [f"missing design document: {path}"]
    heading_matches = list(REQUIRED_HEADING_RE.finditer(text))
    headings = [match.group(0) for match in heading_matches]
    missing_headings = [heading for heading in REQUIRED_HEADINGS if heading not in headings]
    duplicate_headings = [heading for heading in REQUIRED_HEADINGS if headings.count(heading) > 1]
    forbidden = [marker for marker in FORBIDDEN_MARKERS if marker in text]
    errors = [f"missing heading: {heading}" for heading in missing_headings]
    errors.extend(f"duplicate heading: {heading}" for heading in duplicate_headings)
    sections: dict[str, str] = {}
    for index, match in enumerate(heading_matches):
        heading = match.group(0)
        start = match.end()
        end = heading_matches[index + 1].start() if index + 1 < len(heading_matches) else len(text)
        sections[heading] = text[start:end]
    for heading, phrases in SECTION_REQUIREMENTS.items():
        section = sections.get(heading, "")
        errors.extend(f"missing contract in {heading}: {phrase}" for phrase in phrases if phrase not in section)
    schema_section = sections.get("## SQLite v1 schema", "")
    errors.extend(f"missing SQLite table: {table}" for table in REQUIRED_TABLES if not re.search(rf"\b{table}\s*\(", schema_section))
    cargo_command = re.compile(r"(?i)\bcargo\s+(build|check|clippy|fmt|run|test|bench|metadata|publish)\b|\bCargo\.toml\b")
    if any(cargo_command.search(line) for line in text.splitlines()):
        errors.append("design requires Cargo or a Cargo product command")
    errors.extend(f"forbidden marker: {marker}" for marker in forbidden)
    return errors


def check_design(path: Path) -> list[str]:
    if not path.is_file():
        return [f"missing design document: {path}"]
    return check_text(path.read_text(encoding="utf-8"), path)


def main(arguments: list[str]) -> int:
    document = Path(__file__).resolve().parents[1] / "docs" / "design" / "phase-0-1.md"
    if arguments == ["--fixture", "missing-fail-open-section"]:
        source = document.read_text(encoding="utf-8")
        start = source.index("## Fail-open contracts")
        end = source.find("\n## ", start + 3)
        document_text = source[:start] + source[end:]
        errors = check_text(document_text, document)
        print_result = "fixture missing-fail-open-section"
        if errors:
            print("Phase design check failed:")
            print("\n".join(f"- {error}" for error in errors))
            return 1
        print(f"Phase design check passed: {print_result}")
        return 0
    elif arguments == ["--probe", "token-soup"]:
        document = Path(__file__).resolve().parent / "fixtures" / "token-soup.md"
    elif arguments == ["--probe", "cargo"]:
        document = Path(__file__).resolve().parent / "fixtures" / "cargo-requirement.md"
    elif arguments == ["--probe", "mixed-cargo"]:
        document = Path(__file__).resolve().parent / "fixtures" / "mixed-cargo-requirement.md"
    elif arguments == ["--probe", "weakened-bounds-privacy"]:
        document = Path(__file__).resolve().parent / "fixtures" / "weakened-bounds-privacy.md"
    elif arguments:
        print("usage: check_phase_design.py [--fixture missing-fail-open-section | --probe token-soup | --probe cargo | --probe mixed-cargo | --probe weakened-bounds-privacy]", file=sys.stderr)
        return 2
    errors = check_design(document)
    if errors:
        print("Phase design check failed:")
        print("\n".join(f"- {error}" for error in errors))
        return 1
    print(f"Phase design check passed: {document}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
