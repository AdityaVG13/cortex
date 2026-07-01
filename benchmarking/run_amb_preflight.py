"""Matrix loading, fairness preflight, and baseline helpers."""
from __future__ import annotations

import argparse
import json
import os
import re
from datetime import datetime
from pathlib import Path
from typing import cast

from run_amb_config import (
    BASELINE_FILE_DEFAULT,
    CASE_ERROR_FILENAME,
    FAIR_RUN_PREFLIGHT_FILENAME,
    MATRIX_CASE_TIMEOUT_MAX_SECONDS,
    MATRIX_FILE_DEFAULT,
    MATRIX_TIMEOUT_MAX_SECONDS,
    MEMBENCH_DEFAULT_DATA_PATH,
    MEMBENCH_REQUIRED_FILES,
    REPO_ROOT,
    SINGLE_RUN_TIMEOUT_ENV,
    SINGLE_RUN_TIMEOUT_DEFAULT_SECONDS,
    SINGLE_RUN_TIMEOUT_MAX_SECONDS,
    SINGLE_RUN_TIMEOUT_MIN_SECONDS,
)
from run_amb_runtime import (
    _git_head_short,
    _load_lock_summary,
)
from run_amb_daemon import _resolve_memory_backend

def _resolve_baseline_path(raw_path: str) -> Path:
    path = Path(raw_path)
    if path.is_absolute():
        return path
    return (REPO_ROOT / path).resolve()


def _resolve_matrix_path(raw_path: str) -> Path:
    path = Path(raw_path)
    if path.is_absolute():
        return path
    return (REPO_ROOT / path).resolve()


def _slug_fragment(value: str) -> str:
    fragment = re.sub(r"[^a-zA-Z0-9._-]+", "-", value.strip().lower()).strip("-")
    return fragment or "case"


def _resolve_single_run_timeout_seconds(raw_timeout: int) -> int:
    timeout_seconds = int(raw_timeout)
    if not SINGLE_RUN_TIMEOUT_MIN_SECONDS <= timeout_seconds <= SINGLE_RUN_TIMEOUT_MAX_SECONDS:
        raise ValueError(
            "single-run max-runtime-seconds must be between "
            f"{SINGLE_RUN_TIMEOUT_MIN_SECONDS} and {SINGLE_RUN_TIMEOUT_MAX_SECONDS} seconds "
            "(15-20 minutes)"
        )
    return timeout_seconds


def _single_run_timeout_default() -> int:
    raw_value = os.environ.get(SINGLE_RUN_TIMEOUT_ENV)
    if raw_value is None:
        return SINGLE_RUN_TIMEOUT_DEFAULT_SECONDS
    try:
        parsed = int(raw_value)
    except ValueError as exc:
        raise ValueError(
            f"{SINGLE_RUN_TIMEOUT_ENV} must be an integer number of seconds "
            f"between {SINGLE_RUN_TIMEOUT_MIN_SECONDS} and {SINGLE_RUN_TIMEOUT_MAX_SECONDS}"
        ) from exc
    return _resolve_single_run_timeout_seconds(parsed)


def _load_matrix_spec(path: Path) -> tuple[list[dict[str, object]], dict[str, object]]:
    if not path.exists():
        raise FileNotFoundError(f"matrix file not found: {path}")
    payload = json.loads(path.read_text(encoding="utf-8-sig"))
    execution_profile: dict[str, object] = {}
    if isinstance(payload, dict):
        raw_cases = payload.get("cases")
        if raw_cases is None:
            raw_cases = payload.get("scenarios")
        raw_profile = payload.get("execution_profile")
        if raw_profile is not None:
            if not isinstance(raw_profile, dict):
                raise ValueError("matrix execution_profile must be an object")
            execution_profile = cast(dict[str, object], raw_profile)
    elif isinstance(payload, list):
        raw_cases = payload
    else:
        raw_cases = None
    if not isinstance(raw_cases, list) or not raw_cases:
        raise ValueError("matrix file must contain a non-empty 'cases' array")

    cases: list[dict[str, object]] = []
    for index, raw_case in enumerate(raw_cases, start=1):
        if not isinstance(raw_case, dict):
            raise ValueError(f"matrix case #{index} must be an object")
        dataset = raw_case.get("dataset")
        split = raw_case.get("split")
        if not isinstance(dataset, str) or not dataset.strip():
            raise ValueError(f"matrix case #{index} is missing required string field 'dataset'")
        if not isinstance(split, str) or not split.strip():
            raise ValueError(f"matrix case #{index} is missing required string field 'split'")
        normalized: dict[str, object] = {
            "dataset": dataset.strip(),
            "split": split.strip(),
            "id": str(
                raw_case.get("id")
                or f"{index:02d}-{_slug_fragment(dataset)}-{_slug_fragment(split)}"
            ),
        }
        for key in (
            "mode",
            "category",
            "query_id",
            "run_name",
            "description",
            "retrieval_profile",
            "quality_token_target",
            "memory_backend",
        ):
            value = raw_case.get(key)
            if value is not None:
                normalized[key] = str(value)
        for key in ("query_limit", "doc_limit", "recall_budget"):
            value = raw_case.get(key)
            if value is None:
                continue
            if not isinstance(value, int):
                raise ValueError(f"matrix case #{index} field '{key}' must be an integer")
            normalized[key] = value
        if "oracle" in raw_case:
            normalized["oracle"] = bool(raw_case.get("oracle"))
        cases.append(normalized)
    return cases, execution_profile


def _load_matrix_cases(path: Path) -> list[dict[str, object]]:
    cases, _ = _load_matrix_spec(path)
    return cases


def _profile_int(value: object, *, field_name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"matrix execution_profile field '{field_name}' must be an integer")
    return int(value)


def _profile_float(value: object, *, field_name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"matrix execution_profile field '{field_name}' must be numeric")
    return float(value)


def _profile_bool(value: object, *, field_name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"matrix execution_profile field '{field_name}' must be a boolean")
    return value


def _profile_str(value: object, *, field_name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"matrix execution_profile field '{field_name}' must be a non-empty string")
    return value.strip()


def _apply_matrix_execution_profile(args: argparse.Namespace, profile: dict[str, object]) -> None:
    if not profile:
        return

    int_fields = (
        "max_runtime_seconds",
        "max_case_runtime_seconds",
        "recall_budget",
        "max_recall_tokens",
    )
    float_fields = ("min_accuracy", "max_avg_recall_tokens")
    bool_fields = ("allow_missing_recall_metrics", "no_enforce_gate", "oracle")
    str_fields = (
        "token_gate_mode",
        "provider_profile",
        "retrieval_profile",
        "quality_token_target",
    )

    for field in int_fields:
        if field in profile:
            setattr(args, field, _profile_int(profile[field], field_name=field))
    for field in float_fields:
        if field in profile:
            setattr(args, field, _profile_float(profile[field], field_name=field))
    for field in bool_fields:
        if field in profile:
            setattr(args, field, _profile_bool(profile[field], field_name=field))
    for field in str_fields:
        if field in profile:
            setattr(args, field, _profile_str(profile[field], field_name=field))


def _resolve_matrix_timeout_seconds(raw_timeout: int, *, max_timeout: int, field_name: str) -> int:
    timeout_seconds = int(raw_timeout)
    if timeout_seconds <= 0 or timeout_seconds > max_timeout:
        raise ValueError(
            f"matrix {field_name} must be between 1 and {max_timeout} seconds"
        )
    return timeout_seconds


def _format_preflight_failures(header: str, failures: list[str]) -> str:
    lines = "\n".join(f"- {failure}" for failure in failures)
    return f"{header}:\n{lines}"


def _write_fair_run_preflight(run_dir: Path, payload: dict[str, object]) -> None:
    (run_dir / FAIR_RUN_PREFLIGHT_FILENAME).write_text(
        json.dumps(payload, indent=2),
        encoding="utf-8",
    )
    print(json.dumps({"fair_run_preflight": payload}, indent=2))


def _has_query_id(value: object) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _resolve_membench_data_path() -> Path:
    raw = os.environ.get("MEMBENCH_DATA_PATH")
    if raw and raw.strip():
        return Path(raw.strip()).expanduser()
    return MEMBENCH_DEFAULT_DATA_PATH


def _membench_missing_files(data_path: Path) -> list[str]:
    return [name for name in MEMBENCH_REQUIRED_FILES if not (data_path / name).exists()]


def _dataset_prereq_violations(
    *,
    dataset: str,
    split: str | None = None,
    case_id: object | None = None,
) -> list[str]:
    if dataset.strip().lower() != "membench":
        return []
    data_path = _resolve_membench_data_path()
    missing_files = _membench_missing_files(data_path)
    if not missing_files:
        return []
    scope = "membench dataset"
    if split:
        scope = f"membench split '{split}'"
    if case_id is not None:
        scope = f"{scope} (case '{case_id}')"
    missing_str = ", ".join(missing_files)
    resolved_path = data_path.resolve()
    return [
        (
            f"{scope} is missing required files at {resolved_path}: {missing_str}. "
            "Download MemBench and set MEMBENCH_DATA_PATH to the dataset directory."
        )
    ]


def _single_run_fairness_violations(args: argparse.Namespace) -> list[str]:
    violations: list[str] = []
    if bool(getattr(args, "oracle", False)):
        violations.append("oracle=true is not allowed for fair scored runs")
    if _has_query_id(getattr(args, "query_id", None)):
        violations.append("query_id pinning is not allowed for fair scored runs")
    if getattr(args, "doc_limit", None) is not None:
        violations.append("doc_limit shortcuts are not allowed for fair scored runs")
    if bool(getattr(args, "no_enforce_gate", False)):
        violations.append("no_enforce_gate=true bypasses quality/token caps and is not allowed")
    if bool(getattr(args, "allow_missing_recall_metrics", False)):
        violations.append("allow_missing_recall_metrics=true bypasses recall-token accounting and is not allowed")
    return violations


def _build_single_run_preflight(args: argparse.Namespace) -> tuple[dict[str, object], int | None]:
    violations: list[str] = []
    timeout_seconds: int | None = None
    timeout_error: str | None = None
    timeout_value = int(getattr(args, "max_runtime_seconds", 0))
    try:
        timeout_seconds = _resolve_single_run_timeout_seconds(timeout_value)
    except ValueError as exc:
        timeout_error = str(exc)
        violations.append(timeout_error)

    fairness_violations = _single_run_fairness_violations(args)
    violations.extend(fairness_violations)
    dataset_violations = _dataset_prereq_violations(
        dataset=str(getattr(args, "dataset", "")),
        split=cast(str | None, getattr(args, "split", None)),
    )
    violations.extend(dataset_violations)

    payload: dict[str, object] = {
        "command": "run",
        "timestamp": datetime.now().isoformat(),
        "passed": len(violations) == 0,
        "max_runtime_seconds": timeout_value,
        "checks": [
            {
                "name": "runtime_cap_within_15_to_20_minutes",
                "passed": timeout_error is None,
                "value": timeout_value,
                "required_range_seconds": [
                    SINGLE_RUN_TIMEOUT_MIN_SECONDS,
                    SINGLE_RUN_TIMEOUT_MAX_SECONDS,
                ],
                "error": timeout_error,
            },
            {
                "name": "oracle_mode_disabled",
                "passed": not bool(getattr(args, "oracle", False)),
                "value": bool(getattr(args, "oracle", False)),
            },
            {
                "name": "query_id_unset",
                "passed": not _has_query_id(getattr(args, "query_id", None)),
                "value": getattr(args, "query_id", None),
            },
            {
                "name": "doc_limit_unset",
                "passed": getattr(args, "doc_limit", None) is None,
                "value": getattr(args, "doc_limit", None),
            },
            {
                "name": "quality_gate_enforced",
                "passed": not bool(getattr(args, "no_enforce_gate", False)),
                "value": bool(getattr(args, "no_enforce_gate", False)),
            },
            {
                "name": "recall_metrics_required",
                "passed": not bool(getattr(args, "allow_missing_recall_metrics", False)),
                "value": bool(getattr(args, "allow_missing_recall_metrics", False)),
            },
            {
                "name": "dataset_prerequisites_available",
                "passed": len(dataset_violations) == 0,
                "dataset": str(getattr(args, "dataset", "")),
                "split": getattr(args, "split", None),
                "violations": dataset_violations,
            },
        ],
        "violations": violations,
    }
    return payload, timeout_seconds


def _matrix_fairness_violations(
    args: argparse.Namespace,
    cases: list[dict[str, object]],
    *,
    requested_shortcuts: dict[str, bool] | None = None,
) -> list[str]:
    violations: list[str] = []
    requested = requested_shortcuts or {}
    requested_oracle = bool(requested.get("oracle", False))
    requested_no_enforce_gate = bool(requested.get("no_enforce_gate", False))
    requested_allow_missing_metrics = bool(requested.get("allow_missing_recall_metrics", False))

    if requested_oracle and not bool(args.oracle):
        violations.append(
            "matrix invocation requested oracle=true; oracle shortcuts are not allowed"
        )
    if requested_no_enforce_gate and not bool(getattr(args, "no_enforce_gate", False)):
        violations.append(
            "matrix invocation requested no_enforce_gate=true; gate bypass shortcuts are not allowed"
        )
    if requested_allow_missing_metrics and not bool(
        getattr(args, "allow_missing_recall_metrics", False)
    ):
        violations.append(
            "matrix invocation requested allow_missing_recall_metrics=true; missing-metrics shortcuts are not allowed"
        )

    if bool(args.oracle):
        violations.append("matrix mode does not permit oracle runs")
    if _has_query_id(getattr(args, "query_id", None)):
        violations.append("matrix mode does not permit default query_id pinning")
    if getattr(args, "doc_limit", None) is not None:
        violations.append("matrix mode does not permit default doc_limit shortcuts")
    if bool(getattr(args, "no_enforce_gate", False)):
        violations.append("matrix mode does not permit no_enforce_gate=true")
    if bool(getattr(args, "allow_missing_recall_metrics", False)):
        violations.append("matrix mode does not permit allow_missing_recall_metrics=true")
    for index, case in enumerate(cases, start=1):
        if bool(case.get("oracle", False)):
            violations.append(
                f"matrix case #{index} sets oracle=true; oracle is not allowed in matrix mode"
            )
        if _has_query_id(case.get("query_id")):
            violations.append(
                f"matrix case #{index} sets query_id; pinned query shortcuts are not allowed"
            )
        if "doc_limit" in case:
            violations.append(
                f"matrix case #{index} sets doc_limit; ingestion shortcuts are not allowed"
            )
    return violations


def _resolve_matrix_dataset_prereq_skips(
    selected_cases: list[dict[str, object]],
) -> tuple[list[dict[str, object]], list[dict[str, object]]]:
    runnable_cases: list[dict[str, object]] = []
    skipped_cases: list[dict[str, object]] = []
    for case in selected_cases:
        case_violations = _dataset_prereq_violations(
            dataset=str(case.get("dataset", "")),
            split=cast(str | None, case.get("split")),
            case_id=case.get("id"),
        )
        if case_violations:
            skipped_cases.append(
                {
                    "id": str(case.get("id", "")),
                    "dataset": str(case.get("dataset", "")),
                    "split": str(case.get("split", "")),
                    "violations": case_violations,
                }
            )
            continue
        runnable_cases.append(case)
    return runnable_cases, skipped_cases


def _build_matrix_preflight(
    *,
    args: argparse.Namespace,
    cases: list[dict[str, object]],
    selected_cases: list[dict[str, object]],
    runnable_cases: list[dict[str, object]],
    skipped_cases: list[dict[str, object]],
    start_index: int,
    max_runtime_seconds: int,
    max_case_runtime_seconds: int,
    requested_shortcuts: dict[str, bool] | None = None,
) -> dict[str, object]:
    requested = requested_shortcuts or {}
    requested_oracle = bool(requested.get("oracle", False))
    requested_no_enforce_gate = bool(requested.get("no_enforce_gate", False))
    requested_allow_missing_metrics = bool(requested.get("allow_missing_recall_metrics", False))
    fairness_violations = _matrix_fairness_violations(
        args,
        cases,
        requested_shortcuts=requested,
    )
    violations = [*fairness_violations]
    oracle_case_count = sum(1 for case in cases if bool(case.get("oracle", False)))
    query_id_case_count = sum(1 for case in cases if _has_query_id(case.get("query_id")))
    doc_limit_case_count = sum(1 for case in cases if "doc_limit" in case)
    payload: dict[str, object] = {
        "command": "matrix",
        "timestamp": datetime.now().isoformat(),
        "passed": len(violations) == 0,
        "case_count_total": len(cases),
        "case_count_selected": len(selected_cases),
        "case_count_runnable": len(runnable_cases),
        "case_count_skipped_prereq": len(skipped_cases),
        "start_index": start_index,
        "max_runtime_seconds": max_runtime_seconds,
        "max_case_runtime_seconds": max_case_runtime_seconds,
        "checks": [
            {
                "name": "matrix_runtime_cap_within_20_minutes",
                "passed": 1 <= max_runtime_seconds <= MATRIX_TIMEOUT_MAX_SECONDS,
                "value": max_runtime_seconds,
                "max_allowed_seconds": MATRIX_TIMEOUT_MAX_SECONDS,
            },
            {
                "name": "matrix_case_runtime_cap_within_15_minutes",
                "passed": 1 <= max_case_runtime_seconds <= MATRIX_CASE_TIMEOUT_MAX_SECONDS,
                "value": max_case_runtime_seconds,
                "max_allowed_seconds": MATRIX_CASE_TIMEOUT_MAX_SECONDS,
            },
            {
                "name": "oracle_mode_disabled",
                "passed": not bool(args.oracle),
                "value": bool(args.oracle),
            },
            {
                "name": "requested_oracle_flag_disabled",
                "passed": not requested_oracle,
                "value": requested_oracle,
            },
            {
                "name": "default_query_id_unset",
                "passed": not _has_query_id(getattr(args, "query_id", None)),
                "value": getattr(args, "query_id", None),
            },
            {
                "name": "default_doc_limit_unset",
                "passed": getattr(args, "doc_limit", None) is None,
                "value": getattr(args, "doc_limit", None),
            },
            {
                "name": "quality_gate_enforced",
                "passed": not bool(getattr(args, "no_enforce_gate", False)),
                "value": bool(getattr(args, "no_enforce_gate", False)),
            },
            {
                "name": "requested_no_enforce_gate_flag_disabled",
                "passed": not requested_no_enforce_gate,
                "value": requested_no_enforce_gate,
            },
            {
                "name": "recall_metrics_required",
                "passed": not bool(getattr(args, "allow_missing_recall_metrics", False)),
                "value": bool(getattr(args, "allow_missing_recall_metrics", False)),
            },
            {
                "name": "requested_allow_missing_recall_metrics_flag_disabled",
                "passed": not requested_allow_missing_metrics,
                "value": requested_allow_missing_metrics,
            },
            {
                "name": "cases_without_oracle",
                "passed": oracle_case_count == 0,
                "violating_cases": oracle_case_count,
            },
            {
                "name": "cases_without_query_id",
                "passed": query_id_case_count == 0,
                "violating_cases": query_id_case_count,
            },
            {
                "name": "cases_without_doc_limit",
                "passed": doc_limit_case_count == 0,
                "violating_cases": doc_limit_case_count,
            },
            {
                "name": "dataset_prerequisites_resolved_via_skip",
                "passed": True,
                "advisory_only": True,
                "runnable_cases": len(runnable_cases),
                "skipped_cases": len(skipped_cases),
                "skipped": skipped_cases,
            },
        ],
        "requested_shortcuts": {
            "oracle": requested_oracle,
            "no_enforce_gate": requested_no_enforce_gate,
            "allow_missing_recall_metrics": requested_allow_missing_metrics,
        },
        "dataset_skips": skipped_cases,
        "violations": violations,
    }
    return payload


def _validate_matrix_fairness(args: argparse.Namespace, cases: list[dict[str, object]]) -> None:
    violations = _matrix_fairness_violations(args, cases)
    if violations:
        raise ValueError(_format_preflight_failures("matrix fair-run preflight failed", violations))


def _build_matrix_run_args(args: argparse.Namespace, case: dict[str, object]) -> argparse.Namespace:
    run_name = case.get("run_name")
    if run_name is None and args.run_name_prefix:
        run_name = f"{args.run_name_prefix}-{case['id']}"
    default_backend = _resolve_memory_backend(args)
    return argparse.Namespace(
        dataset=str(case["dataset"]),
        split=str(case["split"]),
        mode=str(case.get("mode", args.mode)),
        category=case.get("category", args.category),
        query_limit=case.get("query_limit", args.query_limit),
        query_id=case.get("query_id", args.query_id),
        doc_limit=case.get("doc_limit", args.doc_limit),
        oracle=bool(case.get("oracle", args.oracle)),
        run_name=run_name,
        description=case.get("description", args.description),
        retrieval_profile=case.get("retrieval_profile", args.retrieval_profile),
        quality_token_target=case.get("quality_token_target", args.quality_token_target),
        memory_backend=str(case.get("memory_backend", default_backend)),
        token_gate_mode=args.token_gate_mode,
        provider_profile=args.provider_profile,
        baseline_file=args.baseline_file,
        disable_baseline_gates=args.disable_baseline_gates,
        no_auto_tighten_baseline=args.no_auto_tighten_baseline,
        min_queries_for_baseline_update=args.min_queries_for_baseline_update,
        baseline_token_headroom_pct=args.baseline_token_headroom_pct,
        baseline_accuracy_headroom=args.baseline_accuracy_headroom,
        recall_budget=int(case.get("recall_budget", args.recall_budget)),
        min_accuracy=args.min_accuracy,
        max_recall_tokens=args.max_recall_tokens,
        max_avg_recall_tokens=args.max_avg_recall_tokens,
        allow_missing_recall_metrics=args.allow_missing_recall_metrics,
        no_enforce_gate=args.no_enforce_gate,
    )


def _load_baseline_store(path: Path) -> dict:
    if not path.exists():
        return {
            "version": 1,
            "updated_at": None,
            "profiles": {},
        }
    payload = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise ValueError(f"baseline file must contain a JSON object: {path}")
    payload.setdefault("version", 1)
    payload.setdefault("updated_at", None)
    payload.setdefault("profiles", {})
    return payload


def _save_baseline_store(path: Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload["updated_at"] = datetime.now().isoformat()
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")


def _scenario_key(args: argparse.Namespace) -> str:
    category = args.category if args.category else "*"
    backend = _resolve_memory_backend(args)
    return f"{args.dataset}::{args.split}::{args.mode}::{category}::{backend}"


def _get_baseline_entry(store: dict, provider_profile: str, scenario_key: str) -> dict | None:
    profiles = store.get("profiles", {})
    profile_entries = profiles.get(provider_profile, {})
    entry = profile_entries.get(scenario_key)
    if entry is None and scenario_key.count("::") >= 4:
        legacy_key = "::".join(scenario_key.split("::")[:-1])
        entry = profile_entries.get(legacy_key)
    if isinstance(entry, dict):
        return entry
    return None


def _derive_effective_constraints(
    *,
    args: argparse.Namespace,
    token_limits: dict[str, object],
    baseline_entry: dict | None,
    min_accuracy_override: float | None = None,
) -> dict[str, object]:
    min_accuracy = float(args.min_accuracy if min_accuracy_override is None else min_accuracy_override)
    max_tokens = token_limits.get("max_recall_tokens")
    avg_tokens = token_limits.get("max_avg_recall_tokens")
    baseline_applied = False
    if baseline_entry is not None and not args.disable_baseline_gates:
        baseline_applied = True
        baseline_min_accuracy = baseline_entry.get("min_accuracy")
        if baseline_min_accuracy is not None:
            min_accuracy = max(min_accuracy, float(baseline_min_accuracy))
        baseline_max_tokens = baseline_entry.get("max_recall_tokens")
        if max_tokens is not None and baseline_max_tokens is not None:
            max_tokens = min(float(max_tokens), float(baseline_max_tokens))
        baseline_avg_tokens = baseline_entry.get("max_avg_recall_tokens")
        if avg_tokens is not None and baseline_avg_tokens is not None:
            avg_tokens = min(float(avg_tokens), float(baseline_avg_tokens))
    return {
        "baseline_applied": baseline_applied,
        "min_accuracy": round(min_accuracy, 4),
        "max_recall_tokens": None if max_tokens is None else int(round(float(max_tokens))),
        "max_avg_recall_tokens": None if avg_tokens is None else round(float(avg_tokens), 2),
    }


def _tighten_baseline_entry(
    *,
    store: dict,
    provider_profile: str,
    scenario_key: str,
    accuracy: float,
    recall_stats: dict[str, float | int],
    args: argparse.Namespace,
) -> tuple[dict, bool]:
    profiles = store.setdefault("profiles", {})
    profile_entries = profiles.setdefault(provider_profile, {})
    current = profile_entries.get(scenario_key)
    if not isinstance(current, dict):
        current = {}
    max_tokens_observed = int(recall_stats.get("max_recall_tokens", 0))
    avg_tokens_observed = float(recall_stats.get("avg_recall_tokens", 0.0))
    candidate_min_accuracy = max(0.0, accuracy - args.baseline_accuracy_headroom)
    candidate_max_tokens = max(
        1,
        int(round(max_tokens_observed * (1.0 + args.baseline_token_headroom_pct))),
    )
    candidate_avg_tokens = max(
        1.0,
        round(avg_tokens_observed * (1.0 + args.baseline_token_headroom_pct), 2),
    )
    current_min_accuracy = float(current.get("min_accuracy", 0.0))
    current_max_tokens = current.get("max_recall_tokens")
    current_avg_tokens = current.get("max_avg_recall_tokens")
    new_entry = {
        "min_accuracy": round(max(current_min_accuracy, candidate_min_accuracy), 4),
        "max_recall_tokens": (
            candidate_max_tokens
            if current_max_tokens is None
            else int(min(int(current_max_tokens), candidate_max_tokens))
        ),
        "max_avg_recall_tokens": (
            candidate_avg_tokens
            if current_avg_tokens is None
            else round(min(float(current_avg_tokens), candidate_avg_tokens), 2)
        ),
        "runs": int(current.get("runs", 0)) + 1,
        "last_accuracy": round(accuracy, 4),
        "last_max_recall_tokens": max_tokens_observed,
        "last_avg_recall_tokens": round(avg_tokens_observed, 2),
        "updated_at": datetime.now().isoformat(),
    }
    changed = current != new_entry
    profile_entries[scenario_key] = new_entry
    return new_entry, changed


