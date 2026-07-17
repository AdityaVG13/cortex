import React from "react";
import { AppIcon } from "../../ui-icons.jsx";
import {
  CURRENCY_OPTIONS,
  SAVINGS_OPERATION_LABELS,
  timeAgo,
} from "../../constants.js";
import {
  SAVINGS_USD_PER_MILLION,
  ANALYTICS_METRIC_LEGEND,
} from "../constants.js";
import { normalizeCurrencyCode } from "../utils/format.js";
import { agentColor } from "../utils/agent-color.js";
import { AnimatedNumber } from "../components/AnimatedNumber.jsx";
import { Sparkline } from "../components/Sparkline.jsx";
import { MonteCarloProjectionChart } from "../components/MonteCarloProjectionChart.jsx";
import { EmptyItem } from "../components/common.jsx";
import { clampNumber } from "../components/sparkline-utils.js";
function AnalyticsPanel(p) {
  const {
    panel,
    savings,
    hasVisitedAnalytics,
    analyticsReady,
    setCurrency,
    analyticsMode,
    setAnalyticsMode,
    effectiveReducedMotion,
    analyticsPanelRef,
    analyticsTabRefs,
    safeCurrency,
    formatCurrency,
    savingsEstimateLegend,
    refreshSavings,
    reportSurfaceError,
    call,
    monteCarloProjection,
    bootSavingsMomentum,
    latestRecallHitRate,
    recallWindowAverage,
    recallWindowSpread,
    handleAnalyticsTabKey,
    operationRows,
    operationMaxSaved,
    topSavingsByAgent,
  } = p;
  return React.createElement(
    React.Fragment,
    null,
    panel === "analytics" || hasVisitedAnalytics
      ? React.createElement(
          "section",
          {
            ref: analyticsPanelRef,
            className: `panel analytics-panel ${panel === "analytics" ? "active" : "panel-hidden"}`,
            "aria-hidden": panel === "analytics" ? void 0 : !0,
          },
          React.createElement(
            "div",
            { className: "analytics-panel-header" },
            React.createElement(
              "div",
              { className: "analytics-header-copy" },
              React.createElement(
                "span",
                { className: "analytics-kicker" },
                "Cortex / Analytics",
              ),
              React.createElement("h1", null, "Compounding Memory Economics"),
              React.createElement(
                "p",
                null,
                "Track how Cortex turns raw recall pressure into a smaller boot prompt, compounding token savings over time instead of replaying the whole brain on every boot.",
              ),
            ),
            React.createElement(
              "div",
              { className: "analytics-toolbar" },
              React.createElement(
                "span",
                { className: "panel-subtitle" },
                "Token savings and brain health",
              ),
              React.createElement(
                "label",
                { className: "analytics-inline-control" },
                React.createElement("span", null, "Currency"),
                React.createElement(
                  "select",
                  {
                    value: safeCurrency,
                    onChange: (event) =>
                      setCurrency(normalizeCurrencyCode(event.target.value)),
                  },
                  CURRENCY_OPTIONS.map((code) =>
                    React.createElement(
                      "option",
                      { key: code, value: code },
                      code,
                    ),
                  ),
                ),
              ),
              React.createElement(
                "div",
                {
                  className: "analytics-view-toggle",
                  role: "tablist",
                  "aria-label": "Analytics view mode",
                },
                React.createElement(
                  "button",
                  {
                    id: "analytics-tab-aggregate",
                    ref: (element) => {
                      analyticsTabRefs.current.aggregate = element;
                    },
                    type: "button",
                    role: "tab",
                    "aria-selected": analyticsMode === "aggregate",
                    tabIndex: analyticsMode === "aggregate" ? 0 : -1,
                    className: `btn-sm ${analyticsMode === "aggregate" ? "btn-primary" : ""}`,
                    onClick: () => setAnalyticsMode("aggregate"),
                    onKeyDown: handleAnalyticsTabKey,
                  },
                  "Aggregate",
                ),
                React.createElement(
                  "button",
                  {
                    id: "analytics-tab-operations",
                    ref: (element) => {
                      analyticsTabRefs.current.operations = element;
                    },
                    type: "button",
                    role: "tab",
                    "aria-selected": analyticsMode === "operations",
                    tabIndex: analyticsMode === "operations" ? 0 : -1,
                    className: `btn-sm ${analyticsMode === "operations" ? "btn-primary" : ""}`,
                    onClick: () => setAnalyticsMode("operations"),
                    onKeyDown: handleAnalyticsTabKey,
                  },
                  "By Operation",
                ),
              ),
              React.createElement(
                "button",
                {
                  type: "button",
                  className: "btn-sm",
                  onClick: () => refreshSavings().catch(reportSurfaceError),
                },
                "Refresh",
              ),
            ),
          ),
          analyticsReady
            ? savings
              ? React.createElement(
                  React.Fragment,
                  null,
                  React.createElement(
                    "div",
                    {
                      className: "analytics-metric-legend",
                      role: "group",
                      "aria-label": "Analytics metric legend",
                    },
                    ANALYTICS_METRIC_LEGEND.map((entry) =>
                      React.createElement(
                        "div",
                        {
                          key: entry.label,
                          className: "analytics-metric-legend-item",
                        },
                        React.createElement(
                          "span",
                          { className: "analytics-metric-legend-label" },
                          entry.label,
                        ),
                        React.createElement(
                          "span",
                          { className: "analytics-metric-legend-value" },
                          entry.meaning,
                        ),
                      ),
                    ),
                  ),
                  React.createElement(
                    "div",
                    { className: "analytics-assumption-note" },
                    savingsEstimateLegend,
                  ),
                  React.createElement(
                    "div",
                    { className: "analytics-metrics-grid" },
                    React.createElement(
                      "div",
                      {
                        className: "metric metric-featured",
                        "data-accent": "cyan",
                      },
                      React.createElement(
                        "span",
                        { className: "metric-kicker" },
                        "Compounding return",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-value" },
                        React.createElement(AnimatedNumber, {
                          value: savings.summary?.totalSaved || 0,
                          duration: MOTION_MS.numberSlow,
                          reducedMotion: effectiveReducedMotion,
                        }),
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-label" },
                        "Boot Tokens Saved (30d total)",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-footnote" },
                        bootSavingsMomentum === null
                          ? "Rolling 30-day total tokens saved across boot compilations. Momentum appears after at least 8 daily samples."
                          : `Rolling 30-day total tokens saved across boot compilations, momentum ${bootSavingsMomentum >= 0 ? "+" : ""}${bootSavingsMomentum}% vs prior 4-day window.`,
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-icon" },
                        React.createElement(AppIcon, { name: "savings" }),
                      ),
                    ),
                    React.createElement(
                      "div",
                      { className: "metric", "data-accent": "green" },
                      React.createElement(
                        "span",
                        { className: "metric-kicker" },
                        "Efficiency",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-value" },
                        React.createElement(AnimatedNumber, {
                          value: savings.summary?.avgPercent || 0,
                          reducedMotion: effectiveReducedMotion,
                        }),
                        "%",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-label" },
                        "30d Avg Compression",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-footnote" },
                        "Average tokens saved per boot: ",
                        formatCompactNumber(
                          Number(savings.summary?.avgSavedPerBoot || 0),
                        ),
                        " over the same 30-day window",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-icon" },
                        React.createElement(AppIcon, { name: "efficiency" }),
                      ),
                    ),
                    React.createElement(
                      "div",
                      { className: "metric", "data-accent": "blue" },
                      React.createElement(
                        "span",
                        { className: "metric-kicker" },
                        "Throughput",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-value" },
                        React.createElement(AnimatedNumber, {
                          value: throughputBoots7d,
                          reducedMotion: effectiveReducedMotion,
                        }),
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-label" },
                        "Boot Compilations (last 7 calendar days)",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-footnote" },
                        throughputSummary.daysRepresented === 0
                          ? "No boot compilations recorded in the last 7 calendar days."
                          : throughputSummary.isPartialHistory
                            ? `Since first recorded boot (${throughputSummary.daysRepresented} day${throughputSummary.daysRepresented === 1 ? "" : "s"}, ~${throughputAvgPerDay7d}/day).`
                            : `Last 7 calendar days (~${throughputAvgPerDay7d}/day).`,
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-icon" },
                        React.createElement(AppIcon, { name: "refresh" }),
                      ),
                    ),
                    React.createElement(
                      "div",
                      { className: "metric", "data-accent": "purple" },
                      React.createElement(
                        "span",
                        { className: "metric-kicker" },
                        "Compiled context",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-value" },
                        React.createElement(AnimatedNumber, {
                          value: savings.summary?.totalServed || 0,
                          duration: MOTION_MS.numberSlow,
                          reducedMotion: effectiveReducedMotion,
                        }),
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-label" },
                        "Boot Prompt Tokens Served (30d total)",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-footnote" },
                        "30-day cumulative prompt tokens served at boot; average ",
                        formatCompactNumber(
                          Number(savings.summary?.avgServedPerBoot || 0),
                        ),
                        " per boot",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-icon" },
                        React.createElement(AppIcon, { name: "outbound" }),
                      ),
                    ),
                    React.createElement(
                      "div",
                      { className: "metric", "data-accent": "green" },
                      React.createElement(
                        "span",
                        { className: "metric-kicker" },
                        "Economic value",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-value" },
                        formatCurrency(
                          ((savings.summary?.totalSaved || 0) *
                            SAVINGS_USD_PER_MILLION) /
                            1e6,
                        ),
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-label" },
                        "Est. ",
                        safeCurrency,
                        " Saved",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-footnote" },
                        "Derived from ",
                        formatCompactNumber(
                          Number(savings.summary?.totalSaved || 0),
                        ),
                        " total tokens saved across boot compilations in the last 30 days",
                      ),
                      React.createElement(
                        "span",
                        { className: "metric-icon" },
                        "$",
                      ),
                    ),
                  ),
                  analyticsMode === "aggregate"
                    ? React.createElement(
                        "div",
                        {
                          id: "analytics-tabpanel-aggregate",
                          className: "analytics-mode-panel",
                          role: "tabpanel",
                          "aria-labelledby": "analytics-tab-aggregate",
                          tabIndex: 0,
                        },
                        React.createElement(
                          "div",
                          {
                            className:
                              "analytics-explainer analytics-explainer-rich",
                          },
                          React.createElement(
                            "div",
                            { className: "analytics-explainer-title" },
                            "How to read this",
                          ),
                          React.createElement(
                            "p",
                            null,
                            "Cortex compiles a budgeted boot prompt instead of replaying raw memory. ",
                            React.createElement("code", null, "baseline"),
                            " is estimated raw context load, ",
                            React.createElement("code", null, "served"),
                            " is the compiled prompt, and ",
                            React.createElement("code", null, "saved"),
                            " is the difference. Aggregate mode shows the compounding system view. By Operation isolates where those savings come from.",
                          ),
                          React.createElement(
                            "div",
                            { className: "analytics-stat-strip" },
                            React.createElement(
                              "div",
                              { className: "analytics-stat-chip" },
                              React.createElement(
                                "span",
                                { className: "analytics-stat-chip-label" },
                                "Avg raw per boot",
                              ),
                              React.createElement(
                                "strong",
                                null,
                                formatCompactNumber(
                                  Number(
                                    savings.summary?.avgBaselinePerBoot || 0,
                                  ),
                                ),
                                "t",
                              ),
                            ),
                            React.createElement(
                              "div",
                              { className: "analytics-stat-chip" },
                              React.createElement(
                                "span",
                                { className: "analytics-stat-chip-label" },
                                "Avg served per boot",
                              ),
                              React.createElement(
                                "strong",
                                null,
                                formatCompactNumber(
                                  Number(
                                    savings.summary?.avgServedPerBoot || 0,
                                  ),
                                ),
                                "t",
                              ),
                            ),
                            React.createElement(
                              "div",
                              { className: "analytics-stat-chip" },
                              React.createElement(
                                "span",
                                { className: "analytics-stat-chip-label" },
                                "Median 30d gain",
                              ),
                              React.createElement(
                                "strong",
                                null,
                                monteCarloProjection
                                  ? `${formatSignedCompactNumber(Number(monteCarloProjection.summary?.p50Gain || 0))}t`
                                  : "Pending",
                              ),
                            ),
                          ),
                        ),
                        React.createElement(
                          "div",
                          { className: "analytics-stage-grid" },
                          React.createElement(
                            "div",
                            {
                              className:
                                "card analytics-hero-card analytics-card-span-2",
                            },
                            React.createElement(
                              "div",
                              { className: "analytics-card-header-tight" },
                              React.createElement(
                                "div",
                                null,
                                React.createElement(
                                  "span",
                                  { className: "analytics-card-kicker" },
                                  "Projection",
                                ),
                                React.createElement(
                                  "h2",
                                  null,
                                  "Monte Carlo Savings Horizon",
                                ),
                              ),
                              React.createElement(
                                "span",
                                { className: "badge" },
                                monteCarloProjection
                                  ? `${monteCarloProjection.simulationCount} sims / 30 days`
                                  : "Waiting for more history",
                              ),
                            ),
                            React.createElement(
                              "p",
                              { className: "chart-summary" },
                              "A deterministic Monte Carlo projection built from recent daily savings. It estimates the likely additional savings band over the next 30 days so the trajectory reads as future lift, not replayed lifetime totals.",
                            ),
                            React.createElement(MonteCarloProjectionChart, {
                              projection: monteCarloProjection,
                            }),
                            monteCarloProjection
                              ? React.createElement(
                                  "div",
                                  {
                                    className:
                                      "analytics-stat-strip analytics-stat-strip-tight",
                                  },
                                  React.createElement(
                                    "div",
                                    { className: "analytics-stat-chip" },
                                    React.createElement(
                                      "span",
                                      {
                                        className: "analytics-stat-chip-label",
                                      },
                                      "p10",
                                    ),
                                    React.createElement(
                                      "strong",
                                      null,
                                      formatSignedCompactNumber(
                                        Number(
                                          monteCarloProjection.summary
                                            ?.p10Gain || 0,
                                        ),
                                      ),
                                      "t",
                                    ),
                                  ),
                                  React.createElement(
                                    "div",
                                    { className: "analytics-stat-chip" },
                                    React.createElement(
                                      "span",
                                      {
                                        className: "analytics-stat-chip-label",
                                      },
                                      "p50",
                                    ),
                                    React.createElement(
                                      "strong",
                                      null,
                                      formatSignedCompactNumber(
                                        Number(
                                          monteCarloProjection.summary
                                            ?.p50Gain || 0,
                                        ),
                                      ),
                                      "t",
                                    ),
                                  ),
                                  React.createElement(
                                    "div",
                                    { className: "analytics-stat-chip" },
                                    React.createElement(
                                      "span",
                                      {
                                        className: "analytics-stat-chip-label",
                                      },
                                      "p90",
                                    ),
                                    React.createElement(
                                      "strong",
                                      null,
                                      formatSignedCompactNumber(
                                        Number(
                                          monteCarloProjection.summary
                                            ?.p90Gain || 0,
                                        ),
                                      ),
                                      "t",
                                    ),
                                  ),
                                  React.createElement(
                                    "div",
                                    { className: "analytics-stat-chip" },
                                    React.createElement(
                                      "span",
                                      {
                                        className: "analytics-stat-chip-label",
                                      },
                                      "Current run-rate",
                                    ),
                                    React.createElement(
                                      "strong",
                                      null,
                                      formatCompactNumber(
                                        Number(
                                          monteCarloProjection.summary
                                            ?.avgDaily || 0,
                                        ),
                                      ),
                                      "t/day",
                                    ),
                                  ),
                                )
                              : null,
                          ),
                          React.createElement(
                            "div",
                            {
                              className:
                                "card analytics-chart-card analytics-health-card",
                            },
                            React.createElement(
                              "div",
                              { className: "analytics-card-header-tight" },
                              React.createElement(
                                "div",
                                null,
                                React.createElement(
                                  "span",
                                  { className: "analytics-card-kicker" },
                                  "Live health",
                                ),
                                React.createElement(
                                  "h2",
                                  null,
                                  "Recall Quality",
                                ),
                              ),
                              React.createElement(
                                "span",
                                { className: "badge" },
                                latestRecallHitRate || 0,
                                "%",
                              ),
                            ),
                            React.createElement(
                              "p",
                              { className: "chart-summary" },
                              "Recall quality is tracked as a health box because the current signal is usually flat. What matters here is whether it is stable, drifting, or falling behind token savings.",
                            ),
                            React.createElement(
                              "div",
                              {
                                className:
                                  "analytics-stat-strip analytics-stat-strip-tight",
                              },
                              React.createElement(
                                "div",
                                { className: "analytics-stat-chip" },
                                React.createElement(
                                  "span",
                                  { className: "analytics-stat-chip-label" },
                                  "Headline",
                                ),
                                React.createElement(
                                  "strong",
                                  null,
                                  latestRecallHitRate || 0,
                                  "%",
                                ),
                              ),
                              React.createElement(
                                "div",
                                { className: "analytics-stat-chip" },
                                React.createElement(
                                  "span",
                                  { className: "analytics-stat-chip-label" },
                                  "7-day avg",
                                ),
                                React.createElement(
                                  "strong",
                                  null,
                                  recallWindowAverage || 0,
                                  "%",
                                ),
                              ),
                              React.createElement(
                                "div",
                                { className: "analytics-stat-chip" },
                                React.createElement(
                                  "span",
                                  { className: "analytics-stat-chip-label" },
                                  "Spread",
                                ),
                                React.createElement(
                                  "strong",
                                  null,
                                  recallWindowSpread || 0,
                                  " pts",
                                ),
                              ),
                              React.createElement(
                                "div",
                                { className: "analytics-stat-chip" },
                                React.createElement(
                                  "span",
                                  { className: "analytics-stat-chip-label" },
                                  "Assessment",
                                ),
                                React.createElement(
                                  "strong",
                                  null,
                                  recallWindowSpread <= 2
                                    ? "Stable"
                                    : latestRecallHitRate >= 90
                                      ? "Strong"
                                      : "Watch",
                                ),
                              ),
                            ),
                            recallHeadlineUsesFallback
                              ? React.createElement(
                                  "p",
                                  { className: "analytics-inline-note" },
                                  "Headline is pinned to the last full sample day until live recall reaches ",
                                  RECALL_HEADLINE_MIN_QUERIES,
                                  " queries. Today's live sample is ",
                                  Math.round(
                                    Number(latestRecallPoint?.hitRatePct || 0),
                                  ),
                                  "% on ",
                                  latestRecallSampleSize,
                                  " queries.",
                                )
                              : null,
                            React.createElement(
                              "div",
                              {
                                className:
                                  "chart-legend analytics-quality-strip",
                              },
                              recentRecallWindow.length
                                ? recentRecallWindow.map((point) =>
                                    React.createElement(
                                      "span",
                                      {
                                        key: point.date,
                                        className: "chart-day",
                                      },
                                      React.createElement(
                                        "span",
                                        { className: "chart-day-label" },
                                        (point.date || "").slice(5),
                                      ),
                                      React.createElement(
                                        "span",
                                        { className: "chart-day-value" },
                                        Math.round(
                                          Number(point.hitRatePct || 0),
                                        ),
                                        "%",
                                      ),
                                    ),
                                  )
                                : React.createElement(
                                    "span",
                                    { className: "sparkline-empty" },
                                    "Recall metrics will appear after recent boots.",
                                  ),
                            ),
                          ),
                        ),
                        React.createElement(
                          "div",
                          {
                            className: "overview-grid analytics-secondary-grid",
                          },
                          React.createElement(
                            "div",
                            { className: "card analytics-chart-card" },
                            React.createElement(
                              "div",
                              { className: "analytics-card-header-tight" },
                              React.createElement(
                                "div",
                                null,
                                React.createElement(
                                  "span",
                                  { className: "analytics-card-kicker" },
                                  "Short-term movement",
                                ),
                                React.createElement(
                                  "h2",
                                  null,
                                  "Daily Boot Token Savings",
                                ),
                              ),
                              React.createElement(
                                "span",
                                { className: "badge" },
                                dailySeries.length,
                                " days",
                              ),
                            ),
                            React.createElement(Sparkline, {
                              data: (savings.daily || []).map((d) => d.saved),
                              width: 520,
                              height: 120,
                              className: "sparkline-tall",
                            }),
                            React.createElement(
                              "div",
                              { className: "chart-legend" },
                              (savings.daily || [])
                                .slice(-7)
                                .map((d) =>
                                  React.createElement(
                                    "span",
                                    { key: d.date, className: "chart-day" },
                                    React.createElement(
                                      "span",
                                      { className: "chart-day-label" },
                                      d.date.slice(5),
                                    ),
                                    React.createElement(
                                      "span",
                                      { className: "chart-day-value" },
                                      formatCompactNumber(Number(d.saved || 0)),
                                    ),
                                  ),
                                ),
                            ),
                          ),
                          React.createElement(
                            "div",
                            { className: "card analytics-chart-card" },
                            React.createElement(
                              "div",
                              { className: "analytics-card-header-tight" },
                              React.createElement(
                                "div",
                                null,
                                React.createElement(
                                  "span",
                                  { className: "analytics-card-kicker" },
                                  "System load",
                                ),
                                React.createElement(
                                  "h2",
                                  null,
                                  "Daily Boot Compilations",
                                ),
                              ),
                              React.createElement(
                                "span",
                                { className: "badge" },
                                formatCompactNumber(throughputBoots30d),
                                " last 30d",
                              ),
                            ),
                            React.createElement(Sparkline, {
                              data: (savings.daily || []).map((d) => d.boots),
                              width: 520,
                              height: 120,
                              color: "var(--agent-claude)",
                              className: "sparkline-tall",
                            }),
                            React.createElement(
                              "div",
                              { className: "chart-legend" },
                              (savings.daily || [])
                                .slice(-7)
                                .map((d) =>
                                  React.createElement(
                                    "span",
                                    { key: d.date, className: "chart-day" },
                                    React.createElement(
                                      "span",
                                      { className: "chart-day-label" },
                                      d.date.slice(5),
                                    ),
                                    React.createElement(
                                      "span",
                                      { className: "chart-day-value" },
                                      d.boots,
                                    ),
                                  ),
                                ),
                            ),
                          ),
                          React.createElement(
                            "div",
                            { className: "card analytics-chart-card" },
                            React.createElement(
                              "div",
                              { className: "analytics-card-header-tight" },
                              React.createElement(
                                "div",
                                null,
                                React.createElement(
                                  "span",
                                  { className: "analytics-card-kicker" },
                                  "Long-term impact",
                                ),
                                React.createElement(
                                  "h2",
                                  null,
                                  "Cumulative Savings (30d)",
                                ),
                              ),
                              React.createElement(
                                "span",
                                { className: "badge" },
                                formatCompactNumber(cumulativeLatestTotal),
                                "t",
                              ),
                            ),
                            React.createElement(Sparkline, {
                              data: cumulativeSeries.map((point) =>
                                Number(point.savedTotal || 0),
                              ),
                              width: 520,
                              height: 120,
                              color: "var(--green)",
                              className: "sparkline-tall",
                            }),
                            React.createElement(
                              "div",
                              { className: "chart-legend" },
                              cumulativeSeries
                                .slice(-7)
                                .map((point) =>
                                  React.createElement(
                                    "span",
                                    {
                                      key: point.date || point.timestamp,
                                      className: "chart-day",
                                    },
                                    React.createElement(
                                      "span",
                                      { className: "chart-day-label" },
                                      (point.date || "").slice(5),
                                    ),
                                    React.createElement(
                                      "span",
                                      { className: "chart-day-value" },
                                      formatCompactNumber(
                                        Number(point.savedTotal || 0),
                                      ),
                                    ),
                                  ),
                                ),
                            ),
                          ),
                        ),
                        activityHeatmap.length > 0 &&
                          React.createElement(
                            "div",
                            { className: "card analytics-heatmap-card" },
                            React.createElement(
                              "div",
                              { className: "analytics-card-header-tight" },
                              React.createElement(
                                "div",
                                null,
                                React.createElement(
                                  "span",
                                  { className: "analytics-card-kicker" },
                                  "Behavioral map",
                                ),
                                React.createElement(
                                  "h2",
                                  null,
                                  "Agent Activity Heatmap",
                                ),
                              ),
                              React.createElement(
                                "div",
                                {
                                  className: "heatmap-legend-scale",
                                  "aria-hidden": "true",
                                },
                                React.createElement("span", null, "Low"),
                                React.createElement("span", {
                                  className: "heatmap-legend-bar",
                                }),
                                React.createElement("span", null, "High"),
                              ),
                            ),
                            React.createElement(
                              "div",
                              { className: "activity-heatmap" },
                              [
                                "Sun",
                                "Mon",
                                "Tue",
                                "Wed",
                                "Thu",
                                "Fri",
                                "Sat",
                              ].map((day) =>
                                React.createElement(
                                  "div",
                                  {
                                    key: day,
                                    className: "activity-heatmap-row",
                                  },
                                  React.createElement(
                                    "span",
                                    { className: "activity-heatmap-day" },
                                    day,
                                  ),
                                  React.createElement(
                                    "div",
                                    { className: "activity-heatmap-cells" },
                                    Array.from({ length: 24 }).map(
                                      (_, hour) => {
                                        const count =
                                            activityHeatmapLookup.get(
                                              `${day}:${hour}`,
                                            ) || 0,
                                          alpha =
                                            count > 0
                                              ? clampNumber(
                                                  count / activityHeatmapMax,
                                                  0.12,
                                                  1,
                                                )
                                              : 0.04;
                                        return React.createElement("span", {
                                          key: `${day}-${hour}`,
                                          className: "activity-heatmap-cell",
                                          title: `${day} ${hour.toString().padStart(2, "0")}:00 - ${count} events`,
                                          style: {
                                            background: `linear-gradient(180deg, rgba(67, 234, 255, ${alpha}), rgba(58, 109, 255, ${alpha * 0.72}))`,
                                          },
                                        });
                                      },
                                    ),
                                  ),
                                ),
                              ),
                            ),
                          ),
                        React.createElement(
                          "div",
                          { className: "analytics-lists-grid" },
                          React.createElement(
                            "div",
                            { className: "card analytics-list-card" },
                            React.createElement(
                              "div",
                              { className: "analytics-card-header-tight" },
                              React.createElement(
                                "div",
                                null,
                                React.createElement(
                                  "span",
                                  { className: "analytics-card-kicker" },
                                  "Who is creating lift (30d)",
                                ),
                                React.createElement(
                                  "h2",
                                  null,
                                  "Boot Savings by Agent (30d)",
                                ),
                              ),
                              React.createElement(
                                "span",
                                { className: "badge" },
                                savings.byAgent?.length || 0,
                              ),
                            ),
                            React.createElement(
                              "ul",
                              { className: "item-list analytics-list" },
                              topSavingsByAgent.length
                                ? topSavingsByAgent.map((row, i) =>
                                    React.createElement(
                                      "li",
                                      { key: `${row.agent}-${i}` },
                                      React.createElement(
                                        "div",
                                        { className: "item-meta" },
                                        React.createElement(
                                          "span",
                                          {
                                            className: "item-name",
                                            style: {
                                              color: agentColor(row.agent),
                                            },
                                          },
                                          row.agent,
                                        ),
                                        React.createElement(
                                          "span",
                                          { className: "memory-method" },
                                          Number(row.percent || 0),
                                          "% saved",
                                        ),
                                        React.createElement(
                                          "span",
                                          { className: "muted-inline" },
                                          Number(row.boots || 0),
                                          " boots",
                                        ),
                                      ),
                                      React.createElement(
                                        "div",
                                        { className: "item-detail" },
                                        `${Number(row.saved || 0).toLocaleString()}t saved - ${Number(row.served || 0).toLocaleString()}t served`,
                                      ),
                                    ),
                                  )
                                : React.createElement(EmptyItem, {
                                    text: "No per-agent savings data yet",
                                  }),
                            ),
                          ),
                          React.createElement(
                            "div",
                            { className: "card analytics-list-card" },
                            React.createElement(
                              "div",
                              { className: "analytics-card-header-tight" },
                              React.createElement(
                                "div",
                                null,
                                React.createElement(
                                  "span",
                                  { className: "analytics-card-kicker" },
                                  "Latest savings events",
                                ),
                                React.createElement(
                                  "h2",
                                  null,
                                  "Recent Boot Savings",
                                ),
                              ),
                              React.createElement(
                                "span",
                                { className: "badge" },
                                savings.recent?.length || 0,
                              ),
                            ),
                            React.createElement(
                              "ul",
                              { className: "item-list analytics-list" },
                              savings.recent?.length
                                ? savings.recent
                                    .slice(-10)
                                    .reverse()
                                    .map((s, i) =>
                                      React.createElement(
                                        "li",
                                        { key: `${s.timestamp}-${i}` },
                                        React.createElement(
                                          "div",
                                          { className: "item-meta" },
                                          React.createElement(
                                            "span",
                                            {
                                              className: "item-name",
                                              style: {
                                                color: agentColor(s.agent),
                                              },
                                            },
                                            s.agent,
                                          ),
                                          React.createElement(
                                            "span",
                                            { className: "memory-method" },
                                            s.percent,
                                            "% saved",
                                          ),
                                          React.createElement(
                                            "span",
                                            { className: "muted-inline" },
                                            timeAgo(s.timestamp),
                                          ),
                                        ),
                                        React.createElement(
                                          "div",
                                          { className: "item-detail" },
                                          `boot prompt ${Number(s.served || 0).toLocaleString()}t from est. raw ${Number(s.baseline || 0).toLocaleString()}t (${Number(s.saved || 0).toLocaleString()}t saved)`,
                                          Number(s.admitted || 0) > 0 ||
                                            Number(s.rejected || 0) > 0
                                            ? ` - capsules ${Number(s.admitted || 0)} in / ${Number(s.rejected || 0)} out`
                                            : "",
                                        ),
                                      ),
                                    )
                                : React.createElement(EmptyItem, {
                                    text: "No recent boot savings events yet",
                                  }),
                            ),
                          ),
                        ),
                      )
                    : React.createElement(
                        "div",
                        {
                          id: "analytics-tabpanel-operations",
                          className: "analytics-mode-panel",
                          role: "tabpanel",
                          "aria-labelledby": "analytics-tab-operations",
                          tabIndex: 0,
                        },
                        React.createElement(
                          "div",
                          {
                            className:
                              "analytics-explainer analytics-explainer-rich",
                          },
                          React.createElement(
                            "div",
                            { className: "analytics-explainer-title" },
                            "Operation view",
                          ),
                          React.createElement(
                            "p",
                            null,
                            "Operation view breaks savings into recall, store, boot compression, and tool-call categories using local events. Use it to see where the system is earning margin, not just how much it saved overall.",
                          ),
                        ),
                        React.createElement(
                          "div",
                          { className: "card analytics-operations-card" },
                          React.createElement(
                            "div",
                            { className: "analytics-card-header-tight" },
                            React.createElement(
                              "div",
                              null,
                              React.createElement(
                                "span",
                                { className: "analytics-card-kicker" },
                                "Attribution",
                              ),
                              React.createElement(
                                "h2",
                                null,
                                "Savings by Operation (30d)",
                              ),
                            ),
                            React.createElement(
                              "span",
                              { className: "badge" },
                              operationRows.length,
                              " categories",
                            ),
                          ),
                          React.createElement(
                            "div",
                            { className: "operation-bars" },
                            operationRows.length
                              ? operationRows.map((row) => {
                                  const saved = Number(row.saved || 0),
                                    served = Number(row.served || 0),
                                    baseline = Number(row.baseline || 0),
                                    width = Math.max(
                                      4,
                                      Math.round(
                                        (saved / operationMaxSaved) * 100,
                                      ),
                                    ),
                                    label =
                                      SAVINGS_OPERATION_LABELS[row.operation] ||
                                      row.operation;
                                  return React.createElement(
                                    "div",
                                    {
                                      className: "operation-bar-row",
                                      key: row.operation,
                                    },
                                    React.createElement(
                                      "div",
                                      { className: "operation-bar-header" },
                                      React.createElement(
                                        "span",
                                        { className: "item-name" },
                                        label,
                                      ),
                                      React.createElement(
                                        "span",
                                        { className: "muted-inline" },
                                        saved.toLocaleString(),
                                        " tokens - ",
                                        formatCurrency(
                                          (saved * SAVINGS_USD_PER_MILLION) /
                                            1e6,
                                        ),
                                      ),
                                    ),
                                    React.createElement(
                                      "div",
                                      {
                                        className: "operation-bar-track",
                                        title: `Raw ${baseline.toLocaleString()} - Compressed ${served.toLocaleString()}`,
                                      },
                                      React.createElement("span", {
                                        className: "operation-bar-fill",
                                        style: { width: `${width}%` },
                                      }),
                                    ),
                                    React.createElement(
                                      "div",
                                      { className: "item-detail" },
                                      `${Number(row.events || 0)} events - raw ${baseline.toLocaleString()} - compressed ${served.toLocaleString()}`,
                                    ),
                                  );
                                })
                              : React.createElement(EmptyItem, {
                                  text: "No operation breakdown data yet",
                                }),
                          ),
                        ),
                      ),
                )
              : React.createElement(
                  "div",
                  { className: "card full" },
                  React.createElement(EmptyItem, {
                    text: "Loading savings data...",
                  }),
                )
            : React.createElement(
                "div",
                { className: "card full analytics-loading-card" },
                React.createElement(EmptyItem, {
                  text: "Preparing analytics surface...",
                }),
              ),
        )
      : null,
  );
}
export { AnalyticsPanel };
