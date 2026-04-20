#!/usr/bin/env python3
"""Validate feature-intake manifest entries for intelligence-work guardrails."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ALLOWED_STATUS = {"draft", "active", "completed", "deferred"}
REQUIRED_FIELDS = (
    "id",
    "title",
    "status",
    "expected_gain",
    "resource_cost",
    "rollback_strategy",
    "measurement",
)


def _load_manifest(path: Path) -> dict:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def _validate_manifest_shape(manifest: dict, errors: list[str]) -> list[dict]:
    if not isinstance(manifest, dict):
        errors.append("manifest root must be an object")
        return []

    version = manifest.get("version")
    if version != 1:
        errors.append("manifest version must be 1")

    features = manifest.get("features")
    if not isinstance(features, list):
        errors.append("manifest.features must be an array")
        return []
    return [item for item in features if isinstance(item, dict)]


def _validate_feature_entry(index: int, entry: dict, errors: list[str]) -> None:
    prefix = f"features[{index}]"
    for field in REQUIRED_FIELDS:
        value = entry.get(field)
        if not isinstance(value, str) or not value.strip():
            errors.append(f"{prefix}.{field} must be a non-empty string")

    status = str(entry.get("status", "")).strip().lower()
    if status and status not in ALLOWED_STATUS:
        errors.append(
            f"{prefix}.status must be one of {sorted(ALLOWED_STATUS)} (got '{status}')"
        )

    benchmark_metrics = entry.get("benchmark_metrics", [])
    if benchmark_metrics is not None and not isinstance(benchmark_metrics, list):
        errors.append(f"{prefix}.benchmark_metrics must be an array when provided")
    if isinstance(benchmark_metrics, list):
        for metric_idx, metric in enumerate(benchmark_metrics):
            if not isinstance(metric, str) or not metric.strip():
                errors.append(
                    f"{prefix}.benchmark_metrics[{metric_idx}] must be a non-empty string"
                )


def validate_manifest(manifest: dict) -> list[str]:
    errors: list[str] = []
    features = _validate_manifest_shape(manifest, errors)
    seen_ids: set[str] = set()

    for index, entry in enumerate(features):
        _validate_feature_entry(index, entry, errors)
        feature_id = str(entry.get("id", "")).strip()
        if feature_id:
            if feature_id in seen_ids:
                errors.append(f"features[{index}].id duplicates '{feature_id}'")
            seen_ids.add(feature_id)

    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--manifest",
        default="docs/internal/feature-intake/manifest.json",
        help="Path to feature intake manifest JSON.",
    )
    args = parser.parse_args()

    manifest_path = Path(args.manifest).resolve()
    if not manifest_path.exists():
        print(f"error: manifest does not exist: {manifest_path}", file=sys.stderr)
        return 2

    try:
        manifest = _load_manifest(manifest_path)
    except json.JSONDecodeError as err:
        print(f"error: invalid JSON in {manifest_path}: {err}", file=sys.stderr)
        return 2

    errors = validate_manifest(manifest)
    if errors:
        print("Feature intake manifest validation failed:")
        for error in errors:
            print(f"- {error}")
        return 1

    feature_count = len(manifest.get("features", []))
    print(
        "Feature intake manifest validation passed. "
        f"Checked {feature_count} feature entries."
    )
    print(f"Manifest: {manifest_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
