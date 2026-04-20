from __future__ import annotations

import json
from pathlib import Path

from tools.validate_feature_intake import validate_manifest


def test_validate_feature_intake_manifest_passes_for_valid_shape() -> None:
    manifest = {
        "version": 1,
        "features": [
            {
                "id": "feature-a",
                "title": "Feature A",
                "status": "active",
                "expected_gain": "Improve quality.",
                "resource_cost": "Small query overhead.",
                "rollback_strategy": "Revert feature flag and parser branch.",
                "measurement": "Track regression gate pass rate.",
                "benchmark_metrics": ["taskSuccessRate"],
            }
        ],
    }
    assert validate_manifest(manifest) == []


def test_validate_feature_intake_manifest_rejects_missing_required_fields() -> None:
    manifest = {
        "version": 1,
        "features": [
            {
                "id": "feature-a",
                "title": "Feature A",
                "status": "active",
                "expected_gain": "Gain",
                "resource_cost": "",
                "rollback_strategy": "Rollback",
                "measurement": "Metric",
            }
        ],
    }
    errors = validate_manifest(manifest)
    assert any("resource_cost" in item for item in errors)


def test_validate_feature_intake_manifest_rejects_duplicate_ids() -> None:
    manifest = {
        "version": 1,
        "features": [
            {
                "id": "feature-a",
                "title": "Feature A",
                "status": "active",
                "expected_gain": "Gain",
                "resource_cost": "Cost",
                "rollback_strategy": "Rollback",
                "measurement": "Metric",
            },
            {
                "id": "feature-a",
                "title": "Feature A2",
                "status": "draft",
                "expected_gain": "Gain",
                "resource_cost": "Cost",
                "rollback_strategy": "Rollback",
                "measurement": "Metric",
            },
        ],
    }
    errors = validate_manifest(manifest)
    assert any("duplicates 'feature-a'" in item for item in errors)


def test_repository_manifest_validates() -> None:
    manifest_path = Path("docs/internal/feature-intake/manifest.json")
    data = json.loads(manifest_path.read_text(encoding="utf-8"))
    assert validate_manifest(data) == []
