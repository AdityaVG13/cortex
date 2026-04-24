#!/usr/bin/env python3
"""
Triangle judge for Cortex recall-quality eval.

Canonical protocol per `docs/internal/v060/research/judge-reliability-adversarial-eval.md`:
    - 3 independent judges (cross-family: OpenAI + Anthropic + local Qwen3-30B)
    - Answerer must differ from judge family (enforced by caller; documented here)
    - Reference-guided binary verdicts (correct / incorrect / ambiguous)
    - Judge temperature = 0
    - Per-judge verdict logged; consensus via majority; κ reported pairwise

Usage
-----
    python benchmarking/judges/triangle.py \\
        --answers results/cas100-answers.jsonl \\
        --gold benchmarking/adversarial/cas-100.jsonl \\
        --judges gpt-4o,claude-opus-4-6,qwen3-30b-local \\
        --output results/cas100-triangle-$(date +%Y%m%d).json

Inputs
------
`--answers` JSONL schema per line:
    {"id": "cas-001", "answer": "...", "backend": "cortex-http-pure"}

`--gold` JSONL schema (CAS-100 format):
    {"id": "cas-001", "category": "...", "q": "...", "gold": [...],
     "tier": "easy|medium|hard", ...}

Outputs
-------
Single JSON report:
    {
      "schema_version": "triangle.v1",
      "run_metadata": {...},
      "per_item": [{"id", "question", "answers_by_judge", "consensus", ...}],
      "aggregate": {"correct_rate", "per_judge_rate", "pairwise_kappa", ...}
    }

References
----------
- Judge prompt: judge-reliability research §3.6
- Cohen's kappa: Cohen 1960
- Qwen3-30B as local judge: κ=0.813 > human-human 0.801 per research
- Purity: no gold_answers are exposed to the answerer upstream of this script
"""
from __future__ import annotations

import argparse
import asyncio
import dataclasses
import json
import os
import sys
import time
from pathlib import Path
from typing import Any, Iterable, Protocol

# --- Dependencies ------------------------------------------------------------
# External SDKs are imported lazily so --help works without full install.

SCHEMA_VERSION = "triangle.v1"
JUDGE_PROMPT_HASH = "v0.6.0-research-3.6"  # bump when prompt changes

JUDGE_PROMPT_TEMPLATE = """You are a strict, fair evaluator for a long-term memory recall benchmark.

# Task
Decide whether the model's ANSWER is correct, using the GOLD_ANSWERS as ground truth.

# Correctness rules
1. An answer is CORRECT if it contains every fact in gold_answers OR a clearly
   equivalent formulation (semantically identical, numerically identical, or same
   named entity).
2. An answer is INCORRECT if any fact in gold_answers is contradicted, missing,
   or replaced with an unsupported claim.
3. Surrounding polite phrasing, hedges, or intermediate reasoning are allowed;
   judge only the factual content.
4. If gold_answers represents a set ("any one of X, Y, Z"), any single member
   suffices.
5. If the question has category "abstention" OR gold_answers is empty, CORRECT
   means the model declined to answer or said "I don't know"; any confident
   answer is INCORRECT.

# Bias safeguards
- Ignore answer length. A 3-word correct answer is as valid as a 30-word one.
- Ignore stylistic polish. Plain text and elaborate prose are equal.
- Do not reward answers that mirror your own writing style.
- Do not penalize answers that are curt.

# Output format (strict JSON, no preamble)
{{
  "reasoning": "1-3 sentence factual check against gold_answers",
  "verdict": "correct" | "incorrect" | "ambiguous",
  "ambiguity_reason": "string, only if verdict is ambiguous"
}}

# Input
QUESTION: {question}
GOLD_ANSWERS: {gold_answers}
CATEGORY: {category}
MODEL_ANSWER: {answer}
"""

VERDICTS = ("correct", "incorrect", "ambiguous")


# --- Judge interface ---------------------------------------------------------

@dataclasses.dataclass
class JudgeCall:
    judge_name: str
    verdict: str           # "correct" | "incorrect" | "ambiguous"
    reasoning: str
    ambiguity_reason: str | None
    latency_ms: int
    raw_response: str
    error: str | None = None


class Judge(Protocol):
    name: str
    family: str  # openai | anthropic | qwen | gemini | ...

    async def judge(self, prompt: str) -> JudgeCall:
        ...


# --- OpenAI GPT-4o judge -----------------------------------------------------

class OpenAIJudge:
    family = "openai"

    def __init__(self, model: str = "gpt-4o"):
        self.name = model
        self.model = model
        try:
            from openai import AsyncOpenAI
        except ImportError:
            raise RuntimeError("openai package not installed; pip install openai")
        self._client = AsyncOpenAI(api_key=os.environ.get("OPENAI_API_KEY"))

    async def judge(self, prompt: str) -> JudgeCall:
        t0 = time.monotonic()
        try:
            resp = await self._client.chat.completions.create(
                model=self.model,
                messages=[{"role": "user", "content": prompt}],
                temperature=0,
                response_format={"type": "json_object"},
                max_tokens=512,
            )
            text = resp.choices[0].message.content or ""
            parsed = json.loads(text)
            return JudgeCall(
                judge_name=self.name,
                verdict=_normalize_verdict(parsed.get("verdict", "")),
                reasoning=parsed.get("reasoning", ""),
                ambiguity_reason=parsed.get("ambiguity_reason"),
                latency_ms=int((time.monotonic() - t0) * 1000),
                raw_response=text,
            )
        except Exception as e:  # noqa: BLE001
            return JudgeCall(
                judge_name=self.name, verdict="ambiguous",
                reasoning=f"JUDGE ERROR: {e}", ambiguity_reason=str(e),
                latency_ms=int((time.monotonic() - t0) * 1000),
                raw_response="", error=str(e),
            )


# --- Anthropic Claude judge --------------------------------------------------

class AnthropicJudge:
    family = "anthropic"

    def __init__(self, model: str = "claude-opus-4-6"):
        self.name = model
        self.model = model
        try:
            from anthropic import AsyncAnthropic
        except ImportError:
            raise RuntimeError("anthropic package not installed; pip install anthropic")
        self._client = AsyncAnthropic(api_key=os.environ.get("ANTHROPIC_API_KEY"))

    async def judge(self, prompt: str) -> JudgeCall:
        t0 = time.monotonic()
        try:
            resp = await self._client.messages.create(
                model=self.model,
                max_tokens=512,
                temperature=0,
                messages=[{"role": "user", "content": prompt}],
            )
            text = "".join(
                block.text for block in resp.content if getattr(block, "type", "") == "text"
            )
            parsed = _extract_json(text)
            return JudgeCall(
                judge_name=self.name,
                verdict=_normalize_verdict(parsed.get("verdict", "")),
                reasoning=parsed.get("reasoning", ""),
                ambiguity_reason=parsed.get("ambiguity_reason"),
                latency_ms=int((time.monotonic() - t0) * 1000),
                raw_response=text,
            )
        except Exception as e:  # noqa: BLE001
            return JudgeCall(
                judge_name=self.name, verdict="ambiguous",
                reasoning=f"JUDGE ERROR: {e}", ambiguity_reason=str(e),
                latency_ms=int((time.monotonic() - t0) * 1000),
                raw_response="", error=str(e),
            )


# --- Local Qwen3-30B judge (Ollama / llama-cpp-server) -----------------------

class LocalQwenJudge:
    family = "qwen"

    def __init__(
        self,
        model: str = "qwen3:30b-instruct-q4_K_M",
        endpoint: str | None = None,
    ):
        self.name = f"local/{model}"
        self.model = model
        self.endpoint = endpoint or os.environ.get(
            "CORTEX_LOCAL_JUDGE_URL", "http://127.0.0.1:11434"
        )
        try:
            import httpx
        except ImportError:
            raise RuntimeError("httpx package not installed; pip install httpx")
        self._httpx = httpx
        self._client = httpx.AsyncClient(timeout=httpx.Timeout(120.0))

    async def judge(self, prompt: str) -> JudgeCall:
        t0 = time.monotonic()
        try:
            resp = await self._client.post(
                f"{self.endpoint}/api/generate",
                json={
                    "model": self.model,
                    "prompt": prompt,
                    "stream": False,
                    "format": "json",
                    "options": {"temperature": 0, "num_predict": 512},
                },
            )
            resp.raise_for_status()
            text = resp.json().get("response", "")
            parsed = _extract_json(text)
            return JudgeCall(
                judge_name=self.name,
                verdict=_normalize_verdict(parsed.get("verdict", "")),
                reasoning=parsed.get("reasoning", ""),
                ambiguity_reason=parsed.get("ambiguity_reason"),
                latency_ms=int((time.monotonic() - t0) * 1000),
                raw_response=text,
            )
        except Exception as e:  # noqa: BLE001
            return JudgeCall(
                judge_name=self.name, verdict="ambiguous",
                reasoning=f"JUDGE ERROR: {e}", ambiguity_reason=str(e),
                latency_ms=int((time.monotonic() - t0) * 1000),
                raw_response="", error=str(e),
            )

    async def aclose(self) -> None:
        await self._client.aclose()


# --- Helpers -----------------------------------------------------------------

def _normalize_verdict(raw: str) -> str:
    v = (raw or "").strip().lower()
    return v if v in VERDICTS else "ambiguous"


def _extract_json(text: str) -> dict[str, Any]:
    """Tolerate prose surrounding JSON blocks."""
    s = text.strip()
    if s.startswith("{") and s.endswith("}"):
        return json.loads(s)
    start = s.find("{")
    end = s.rfind("}")
    if start != -1 and end != -1 and end > start:
        return json.loads(s[start : end + 1])
    return {"verdict": "ambiguous", "reasoning": f"unparseable: {text[:200]}"}


def build_prompt(
    question: str, gold_answers: list[str], category: str, answer: str
) -> str:
    gold_repr = (
        "(empty — abstention expected)"
        if not gold_answers
        else ", ".join(f'"{g}"' for g in gold_answers)
    )
    return JUDGE_PROMPT_TEMPLATE.format(
        question=question,
        gold_answers=gold_repr,
        category=category,
        answer=answer,
    )


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    out = []
    with path.open("r", encoding="utf-8") as fh:
        for ln in fh:
            ln = ln.strip()
            if not ln or ln.startswith("//"):
                continue
            out.append(json.loads(ln))
    return out


# --- Kappa calculation -------------------------------------------------------

def cohen_kappa(y1: list[str], y2: list[str]) -> float:
    """Cohen's κ between two raters on same items."""
    assert len(y1) == len(y2), "ratings length mismatch"
    if not y1:
        return float("nan")
    labels = sorted({*y1, *y2})
    n = len(y1)
    # Observed agreement
    po = sum(1 for a, b in zip(y1, y2) if a == b) / n
    # Expected agreement under independence
    pe = 0.0
    for label in labels:
        p1 = y1.count(label) / n
        p2 = y2.count(label) / n
        pe += p1 * p2
    if pe == 1.0:
        return 1.0
    return (po - pe) / (1.0 - pe)


def fleiss_kappa(ratings: list[list[str]]) -> float:
    """Fleiss' κ across N items rated by K judges.

    ratings[i] = [judge_0_verdict, judge_1_verdict, ...] for item i.
    """
    if not ratings:
        return float("nan")
    N = len(ratings)
    K = len(ratings[0])
    if K < 2:
        return float("nan")
    categories = sorted({r for item in ratings for r in item})
    C = len(categories)
    cat_idx = {c: i for i, c in enumerate(categories)}

    # n[i][c] = number of judges that rated item i as category c
    n = [[0] * C for _ in range(N)]
    for i, item in enumerate(ratings):
        for r in item:
            n[i][cat_idx[r]] += 1

    # p_j = fraction of all ratings assigned to category j
    p = [sum(n[i][j] for i in range(N)) / (N * K) for j in range(C)]
    # P_i = agreement among pairs of judges for item i
    P = [
        (sum(n[i][j] * n[i][j] for j in range(C)) - K) / (K * (K - 1)) if K > 1 else 1.0
        for i in range(N)
    ]
    P_bar = sum(P) / N
    P_e = sum(pj * pj for pj in p)
    if P_e == 1.0:
        return 1.0
    return (P_bar - P_e) / (1.0 - P_e)


# --- Triangle orchestrator ---------------------------------------------------

JUDGE_FACTORY = {
    "gpt-4o": lambda: OpenAIJudge("gpt-4o"),
    "gpt-4o-mini": lambda: OpenAIJudge("gpt-4o-mini"),
    "claude-opus-4-6": lambda: AnthropicJudge("claude-opus-4-6"),
    "claude-sonnet-4-6": lambda: AnthropicJudge("claude-sonnet-4-6"),
    "qwen3-30b-local": lambda: LocalQwenJudge("qwen3:30b-instruct-q4_K_M"),
    "qwen3-32b-local": lambda: LocalQwenJudge("qwen3:32b"),
}


def build_judges(names: list[str]) -> list[Judge]:
    judges: list[Judge] = []
    for name in names:
        factory = JUDGE_FACTORY.get(name)
        if factory is None:
            raise SystemExit(
                f"Unknown judge: {name}. Known: {', '.join(JUDGE_FACTORY)}"
            )
        judges.append(factory())
    families = {j.family for j in judges}
    if len(families) < len(judges):
        print(
            f"WARN: judges share a family: {[j.family for j in judges]}. "
            "Cross-family triangulation lost — consider a different mix.",
            file=sys.stderr,
        )
    return judges


async def judge_item(
    judges: list[Judge], item: dict[str, Any], gold_by_id: dict[str, dict[str, Any]]
) -> dict[str, Any]:
    gold = gold_by_id.get(item["id"])
    if gold is None:
        return {
            "id": item["id"],
            "error": "no gold entry for id",
            "skipped": True,
        }
    prompt = build_prompt(
        question=gold["q"],
        gold_answers=gold.get("gold", []),
        category=gold.get("category", "unknown"),
        answer=item["answer"],
    )
    calls = await asyncio.gather(*(j.judge(prompt) for j in judges))
    verdicts = [c.verdict for c in calls]
    # Consensus = majority. Tie → ambiguous.
    from collections import Counter
    votes = Counter(verdicts).most_common()
    consensus = votes[0][0] if len(votes) == 1 or votes[0][1] > votes[1][1] else "ambiguous"

    return {
        "id": item["id"],
        "category": gold.get("category"),
        "tier": gold.get("tier"),
        "question": gold["q"],
        "gold": gold.get("gold", []),
        "answer": item["answer"],
        "backend": item.get("backend"),
        "answers_by_judge": [dataclasses.asdict(c) for c in calls],
        "consensus": consensus,
        "unanimous": len(set(verdicts)) == 1,
    }


async def run(
    answers: list[dict[str, Any]],
    gold: list[dict[str, Any]],
    judges: list[Judge],
    concurrency: int,
) -> dict[str, Any]:
    gold_by_id = {g["id"]: g for g in gold}
    sem = asyncio.Semaphore(concurrency)

    async def bounded(item: dict[str, Any]) -> dict[str, Any]:
        async with sem:
            return await judge_item(judges, item, gold_by_id)

    per_item = await asyncio.gather(*(bounded(a) for a in answers))
    per_item = [r for r in per_item if not r.get("skipped")]

    # Aggregate
    n = len(per_item)
    n_correct_consensus = sum(1 for r in per_item if r["consensus"] == "correct")
    n_unanimous = sum(1 for r in per_item if r["unanimous"])

    # Per-judge accuracy vs consensus
    per_judge = {}
    for idx, j in enumerate(judges):
        correct = sum(
            1 for r in per_item if r["answers_by_judge"][idx]["verdict"] == "correct"
        )
        errors = sum(
            1 for r in per_item if r["answers_by_judge"][idx].get("error")
        )
        per_judge[j.name] = {
            "correct_rate": correct / n if n else 0.0,
            "error_rate": errors / n if n else 0.0,
            "mean_latency_ms": (
                sum(r["answers_by_judge"][idx]["latency_ms"] for r in per_item) / n
                if n else 0.0
            ),
        }

    # Pairwise Cohen's κ
    pairwise_kappa = {}
    for i, j_i in enumerate(judges):
        for k in range(i + 1, len(judges)):
            j_k = judges[k]
            y_i = [r["answers_by_judge"][i]["verdict"] for r in per_item]
            y_k = [r["answers_by_judge"][k]["verdict"] for r in per_item]
            pairwise_kappa[f"{j_i.name}__{j_k.name}"] = round(cohen_kappa(y_i, y_k), 4)

    # Fleiss' κ
    all_ratings = [[r["answers_by_judge"][i]["verdict"] for i in range(len(judges))] for r in per_item]
    fleiss = fleiss_kappa(all_ratings)

    # Per-category rates
    by_cat: dict[str, dict[str, int]] = {}
    for r in per_item:
        cat = r.get("category") or "unknown"
        slot = by_cat.setdefault(cat, {"n": 0, "correct": 0})
        slot["n"] += 1
        if r["consensus"] == "correct":
            slot["correct"] += 1
    per_category = {
        cat: {
            "n": v["n"],
            "consensus_correct_rate": round(v["correct"] / v["n"], 4) if v["n"] else 0.0,
        }
        for cat, v in sorted(by_cat.items())
    }

    return {
        "schema_version": SCHEMA_VERSION,
        "run_metadata": {
            "prompt_hash": JUDGE_PROMPT_HASH,
            "judges": [
                {"name": j.name, "family": j.family} for j in judges
            ],
            "n_items": n,
            "concurrency": concurrency,
            "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        },
        "aggregate": {
            "consensus_correct_rate": round(n_correct_consensus / n, 4) if n else 0.0,
            "unanimous_rate": round(n_unanimous / n, 4) if n else 0.0,
            "per_judge": per_judge,
            "pairwise_cohen_kappa": pairwise_kappa,
            "fleiss_kappa": round(fleiss, 4),
        },
        "per_category": per_category,
        "per_item": per_item,
    }


# --- CLI ---------------------------------------------------------------------

def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description="Triangle judge for Cortex recall eval")
    p.add_argument("--answers", type=Path, required=True,
                   help="JSONL of {id, answer, backend} per model-answer row")
    p.add_argument("--gold", type=Path, required=True,
                   help="JSONL of gold items (CAS-100 format)")
    p.add_argument("--judges", default="gpt-4o,claude-opus-4-6,qwen3-30b-local",
                   help="Comma-separated judge names (default: canonical triangle)")
    p.add_argument("--output", type=Path, required=True,
                   help="JSON output path for aggregate + per-item report")
    p.add_argument("--concurrency", type=int, default=4,
                   help="Max concurrent items in flight across judges (default: 4)")
    p.add_argument("--answerer-family", default=None,
                   help="Optional: answerer LLM family; errors if any judge shares it")
    return p.parse_args()


def main() -> None:
    args = parse_args()

    answers = load_jsonl(args.answers)
    gold = load_jsonl(args.gold)

    judges = build_judges([j.strip() for j in args.judges.split(",") if j.strip()])
    if args.answerer_family:
        shared = [j for j in judges if j.family == args.answerer_family]
        if shared:
            raise SystemExit(
                f"Answerer family {args.answerer_family!r} overlaps judges: "
                f"{[j.name for j in shared]}. "
                "Cross-family guarantee broken — refusing to run."
            )

    print(
        f"Running triangle judge: {len(answers)} answers × {len(judges)} judges "
        f"({', '.join(j.name for j in judges)})",
        file=sys.stderr,
    )
    report = asyncio.run(run(answers, gold, judges, args.concurrency))

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False))

    agg = report["aggregate"]
    print(
        f"\n=== Triangle judge report ===\n"
        f"N items:               {report['run_metadata']['n_items']}\n"
        f"Consensus correct:     {agg['consensus_correct_rate'] * 100:.2f}%\n"
        f"Unanimous:             {agg['unanimous_rate'] * 100:.2f}%\n"
        f"Fleiss κ:              {agg['fleiss_kappa']}\n"
        f"Pairwise Cohen κ:      {json.dumps(agg['pairwise_cohen_kappa'], indent=2)}\n"
        f"Per-judge correct rate:{json.dumps({k: v['correct_rate'] for k, v in agg['per_judge'].items()}, indent=2)}\n"
        f"Output: {args.output}"
    )

    # Exit non-zero if any judge's error rate > 20%
    err_rates = {k: v["error_rate"] for k, v in agg["per_judge"].items()}
    worst = max(err_rates.values()) if err_rates else 0.0
    if worst > 0.20:
        print(f"\nFAIL: judge error rate > 20% ({err_rates})", file=sys.stderr)
        sys.exit(2)


if __name__ == "__main__":
    main()
