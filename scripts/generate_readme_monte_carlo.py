from __future__ import annotations

import argparse
import json
import sqlite3
from dataclasses import dataclass
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
import numpy as np


ROOT = Path(__file__).resolve().parent.parent
DEFAULT_DB = Path.home() / ".cortex" / "cortex.db"
DEFAULT_OUTPUT = ROOT / "assets" / "monte-carlo-readme.png"


BG = "#050B14"
PANEL = "#091321"
GRID = "#16324A"
TEXT = "#F4FAFF"
MUTED = "#88A0B7"
CYAN = "#28D4FF"
CYAN_SOFT = "#0D84A5"
GREEN = "#18E27B"
GREEN_SOFT = "#0C6A3A"
WHITE = "#FFFFFF"


plt.rcParams.update(
    {
        "figure.facecolor": BG,
        "axes.facecolor": PANEL,
        "axes.edgecolor": PANEL,
        "savefig.facecolor": BG,
        "text.color": TEXT,
        "axes.labelcolor": MUTED,
        "xtick.color": MUTED,
        "ytick.color": MUTED,
        "font.family": "DejaVu Sans",
        "font.size": 12,
    }
)


@dataclass
class DailyPoint:
    day: str
    saved_tokens: float


def load_daily_savings(db_path: Path) -> list[DailyPoint]:
    conn = sqlite3.connect(db_path)
    rows = conn.execute(
        """
        SELECT date(created_at) AS day, data
        FROM events
        WHERE type = 'boot_savings'
        ORDER BY created_at
        """
    ).fetchall()
    conn.close()

    per_day: dict[str, float] = {}
    for day, payload in rows:
        if not payload:
            continue
        try:
            data = json.loads(payload)
        except json.JSONDecodeError:
            continue

        baseline = float(data.get("baseline", 0) or 0)
        served = float(data.get("served", 0) or 0)
        saved = max(0.0, baseline - served)
        per_day[day] = per_day.get(day, 0.0) + saved

    return [DailyPoint(day=day, saved_tokens=per_day[day]) for day in sorted(per_day)]


def simulate_cumulative(points: list[DailyPoint], n_days: int = 30, n_sims: int = 2000, seed: int = 7):
    if len(points) < 5:
        raise ValueError("Need at least 5 days of savings history for the README Monte Carlo chart.")

    daily = np.array([p.saved_tokens for p in points], dtype=float)
    cumulative = np.cumsum(daily)
    rng = np.random.default_rng(seed)

    # Bootstrap from observed daily savings so the README chart stays grounded in one real Cortex install.
    projections = np.zeros((n_sims, n_days), dtype=float)
    for i in range(n_sims):
        current = cumulative[-1]
        draws = rng.choice(daily, size=n_days, replace=True)
        for j, draw in enumerate(draws):
            current += max(0.0, float(draw))
            projections[i, j] = current

    sample_idx = np.linspace(0, n_sims - 1, 8, dtype=int)
    sample_paths = projections[sample_idx]

    return {
        "daily": daily,
        "cumulative": cumulative,
        "p10": np.percentile(projections, 10, axis=0),
        "p25": np.percentile(projections, 25, axis=0),
        "p50": np.percentile(projections, 50, axis=0),
        "p75": np.percentile(projections, 75, axis=0),
        "p90": np.percentile(projections, 90, axis=0),
        "sample_paths": sample_paths,
        "n_days": n_days,
        "n_sims": n_sims,
    }


def fmt_tokens(value: float, _: object = None) -> str:
    if value >= 1_000_000:
        return f"{value / 1_000_000:.1f}M"
    if value >= 1_000:
        return f"{value / 1_000:.0f}K"
    return f"{value:.0f}"


def human_tokens(value: float) -> str:
    if value >= 1_000_000:
        return f"{value / 1_000_000:.1f}M tokens"
    if value >= 1_000:
        return f"{value / 1_000:.0f}K tokens"
    return f"{value:.0f} tokens"


def pill(ax, text: str, x: float, y: float, fc: str, ec: str | None = None, alpha: float = 0.95) -> None:
    ax.text(
        x,
        y,
        text,
        ha="left",
        va="center",
        fontsize=11,
        fontweight="bold",
        color=WHITE,
        bbox={
            "boxstyle": "round,pad=0.34,rounding_size=0.35",
            "facecolor": fc,
            "edgecolor": ec or fc,
            "linewidth": 1,
            "alpha": alpha,
        },
        zorder=10,
    )


def draw_chart(points: list[DailyPoint], sim: dict[str, np.ndarray], output_path: Path) -> None:
    cumulative = sim["cumulative"]
    n_hist = len(cumulative)
    n_days = int(sim["n_days"])

    fig, ax = plt.subplots(figsize=(16, 9))
    fig.patch.set_facecolor(BG)
    ax.set_facecolor(PANEL)

    # Historical segment: last 10 days only to keep the chart calm.
    hist_window = min(8, n_hist)
    hist_start = n_hist - hist_window
    x_hist = np.arange(hist_window)
    y_hist = cumulative[hist_start:]
    hist_labels = [p.day[5:] for p in points[hist_start:]]

    x_proj = np.arange(hist_window - 1, hist_window + n_days)
    p10 = np.concatenate([[y_hist[-1]], sim["p10"]])
    p25 = np.concatenate([[y_hist[-1]], sim["p25"]])
    p50 = np.concatenate([[y_hist[-1]], sim["p50"]])
    p75 = np.concatenate([[y_hist[-1]], sim["p75"]])
    p90 = np.concatenate([[y_hist[-1]], sim["p90"]])

    # Base panel polish.
    for spine in ax.spines.values():
        spine.set_visible(False)
    ax.grid(axis="y", color=GRID, linewidth=0.8, alpha=0.35)
    ax.grid(axis="x", visible=False)

    # Historical line + soft fill.
    ax.fill_between(x_hist, y_hist, color=CYAN_SOFT, alpha=0.16, zorder=2)
    ax.plot(x_hist, y_hist, color=CYAN, linewidth=3.0, zorder=5)

    # Forecast fan.
    ax.fill_between(x_proj, p10, p90, color=GREEN_SOFT, alpha=0.18, zorder=1)
    ax.fill_between(x_proj, p25, p75, color=GREEN, alpha=0.22, zorder=2)

    # A few faint sample paths so it still feels like Monte Carlo.
    for path in sim["sample_paths"]:
        sample = np.concatenate([[y_hist[-1]], path])
        ax.plot(x_proj, sample, color=CYAN, linewidth=0.9, alpha=0.08, zorder=3)

    ax.plot(x_proj, p50, color=GREEN, linewidth=2.6, zorder=6)

    # Today divider.
    ax.axvline(hist_window - 1, color=MUTED, linewidth=1.0, linestyle=":", alpha=0.5, zorder=4)
    ax.text(hist_window - 1, p90[-1] * 1.035, "today", ha="center", va="bottom", fontsize=10, color=MUTED)

    # Right-edge labels.
    label_x = x_proj[-1] - 1.6
    pill(ax, f"p90  {human_tokens(p90[-1])}", label_x, p90[-1] * 1.01, "#115732")
    pill(ax, f"median  {human_tokens(p50[-1])}", label_x, p50[-1], "#12A85A")
    pill(ax, f"p10  {human_tokens(p10[-1])}", label_x, max(p10[-1], y_hist[-1] * 1.06), "#0F6077")

    # Titles.
    fig.text(0.055, 0.94, "30-day savings horizon", fontsize=28, fontweight="bold", color=TEXT)
    fig.text(
        0.055,
        0.902,
        "Monte Carlo fan chart from a live Cortex usage history. Real history on the left, projected uncertainty on the right.",
        fontsize=13,
        color=MUTED,
    )
    fig.text(
        0.055,
        0.855,
        "30-day horizon",
        fontsize=11,
        fontweight="bold",
        color=TEXT,
        bbox={
            "boxstyle": "round,pad=0.35,rounding_size=0.35",
            "facecolor": "#10314A",
            "edgecolor": "#10314A",
            "linewidth": 1,
            "alpha": 0.92,
        },
    )
    fig.text(
        0.145,
        0.855,
        "Based on one maintainer-run Cortex dataset",
        fontsize=11,
        fontweight="bold",
        color=TEXT,
        bbox={
            "boxstyle": "round,pad=0.35,rounding_size=0.35",
            "facecolor": "#12301F",
            "edgecolor": "#12301F",
            "linewidth": 1,
            "alpha": 0.92,
        },
    )

    # Axis formatting.
    ax.set_xlim(-0.2, x_proj[-1] + 1.8)
    ax.set_ylim(0, p90[-1] * 1.18)
    ax.yaxis.set_major_formatter(ticker.FuncFormatter(fmt_tokens))
    ax.set_ylabel("Cumulative tokens saved", color=MUTED, labelpad=10)
    ax.tick_params(axis="y", labelsize=11)
    ax.tick_params(axis="x", labelsize=10)

    hist_ticks = list(range(0, hist_window, 2))
    if (hist_window - 1) not in hist_ticks:
        if hist_ticks and (hist_window - 1) - hist_ticks[-1] <= 1:
            hist_ticks[-1] = hist_window - 1
        else:
            hist_ticks.append(hist_window - 1)
    hist_tick_labels = [hist_labels[i] for i in hist_ticks]
    proj_ticks = [hist_window + 4, hist_window + 9, hist_window + 14, hist_window + 19, hist_window + 24, hist_window + 29]
    proj_labels = ["+5d", "+10d", "+15d", "+20d", "+25d", "+30d"]
    ax.set_xticks(hist_ticks + proj_ticks)
    ax.set_xticklabels(hist_tick_labels + proj_labels)

    # Small note.
    fig.text(
        0.055,
        0.055,
        "Projection uses 2,000 bootstrap simulations sampled from observed daily savings in one Cortex install.",
        fontsize=11,
        color=MUTED,
    )

    plt.tight_layout(rect=[0.02, 0.08, 0.98, 0.89])
    fig.savefig(output_path, dpi=180)
    plt.close(fig)


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate the README Monte Carlo proof surface.")
    parser.add_argument("--db", type=Path, default=DEFAULT_DB, help="SQLite DB with Cortex events")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT, help="Output PNG path")
    parser.add_argument("--inspect", action="store_true", help="Print savings summary and exit")
    args = parser.parse_args()

    points = load_daily_savings(args.db)
    if args.inspect:
        print(f"days={len(points)}")
        print(f"first={points[0] if points else None}")
        print(f"last={points[-1] if points else None}")
        print(f"total_saved={sum(p.saved_tokens for p in points):.0f}")
        return

    sim = simulate_cumulative(points)
    draw_chart(points, sim, args.output)
    print(f"wrote {args.output}")


if __name__ == "__main__":
    main()
