import React from "react";
import { AppIcon } from "../../ui-icons.jsx";
import { CURRENCY_OPTIONS } from "../../constants.js";
import { BUDGET_ENDPOINT_DEFINITIONS } from "../../settings/settings-state.js";
import {
  normalizeCurrencyCode,
  formatDaemonEndpoint,
} from "../utils/format.js";
import { PANEL_SEQUENCE } from "../constants.js";
function SettingsPanel(p) {
  const {
    panel,
    savings,
    cortexBase,
    controlSettings,
    budgetDraft,
    budgetConfigBusy,
    budgetConfigMessage,
    ipcAvailable,
    setCurrency,
    isTauriRuntime,
    safeCurrency,
    budgetSummary,
    budgetDraftError,
    budgetDraftEndpoints,
    runRefreshAll,
    openConnectionDialog,
    updateControlSetting,
    reloadBudgetConfigDraft,
    saveBudgetConfigDraft,
    updateBudgetDraftRoot,
    updateBudgetEndpointDraft,
    pill,
    daemonStatusBadge,
  } = p;
  return React.createElement(
    React.Fragment,
    null,
    React.createElement(
      "section",
      {
        className: `panel settings-panel ${panel === "settings" ? "active" : "panel-hidden"}`,
        "aria-hidden": panel === "settings" ? void 0 : !0,
      },
      React.createElement(
        "div",
        { className: "panel-header" },
        React.createElement(
          "div",
          null,
          React.createElement(
            "span",
            { className: "panel-kicker" },
            "Control Center",
          ),
          React.createElement("h1", null, "Settings"),
          React.createElement(
            "p",
            { className: "panel-subtitle" },
            "Accessibility, motion, connection, keyboard, and local budget state.",
          ),
        ),
        React.createElement(
          "button",
          { type: "button", className: "btn-sm", onClick: runRefreshAll },
          "Refresh",
        ),
      ),
      React.createElement(
        "div",
        { className: "settings-grid" },
        React.createElement(
          "section",
          {
            className: "settings-section",
            "aria-labelledby": "settings-accessibility",
          },
          React.createElement(
            "div",
            { className: "settings-section-head" },
            React.createElement(AppIcon, { name: "settings", size: 18 }),
            React.createElement(
              "h2",
              { id: "settings-accessibility" },
              "Accessibility",
            ),
          ),
          React.createElement(
            "label",
            { className: "settings-row" },
            React.createElement(
              "span",
              null,
              React.createElement("strong", null, "High contrast"),
              React.createElement(
                "small",
                null,
                "Increase text and border contrast.",
              ),
            ),
            React.createElement("input", {
              type: "checkbox",
              checked: controlSettings.highContrast,
              onChange: (event) =>
                updateControlSetting("highContrast", event.target.checked),
            }),
          ),
          React.createElement(
            "label",
            { className: "settings-row" },
            React.createElement(
              "span",
              null,
              React.createElement("strong", null, "Keyboard hints"),
              React.createElement("small", null, "Show shortcut labels."),
            ),
            React.createElement("input", {
              type: "checkbox",
              checked: controlSettings.keyboardHints,
              onChange: (event) =>
                updateControlSetting("keyboardHints", event.target.checked),
            }),
          ),
        ),
        React.createElement(
          "section",
          {
            className: "settings-section",
            "aria-labelledby": "settings-motion",
          },
          React.createElement(
            "div",
            { className: "settings-section-head" },
            React.createElement(AppIcon, { name: "analytics", size: 18 }),
            React.createElement(
              "h2",
              { id: "settings-motion" },
              "Appearance & Motion",
            ),
          ),
          React.createElement(
            "label",
            { className: "settings-row" },
            React.createElement(
              "span",
              null,
              React.createElement("strong", null, "Motion"),
              React.createElement("small", null, "Runtime transition profile."),
            ),
            React.createElement(
              "select",
              {
                value: controlSettings.reducedMotion,
                onChange: (event) =>
                  updateControlSetting("reducedMotion", event.target.value),
              },
              React.createElement("option", { value: "system" }, "System"),
              React.createElement("option", { value: "reduce" }, "Reduced"),
              React.createElement("option", { value: "full" }, "Full"),
            ),
          ),
          React.createElement(
            "label",
            { className: "settings-row" },
            React.createElement(
              "span",
              null,
              React.createElement("strong", null, "Currency"),
              React.createElement("small", null, "Token-savings estimates."),
            ),
            React.createElement(
              "select",
              {
                value: safeCurrency,
                onChange: (event) =>
                  setCurrency(normalizeCurrencyCode(event.target.value)),
              },
              CURRENCY_OPTIONS.map((code) =>
                React.createElement("option", { key: code, value: code }, code),
              ),
            ),
          ),
        ),
        React.createElement(
          "section",
          {
            className: "settings-section settings-section-wide",
            "aria-labelledby": "settings-connection",
          },
          React.createElement(
            "div",
            { className: "settings-section-head" },
            React.createElement(AppIcon, { name: "outbound", size: 18 }),
            React.createElement(
              "h2",
              { id: "settings-connection" },
              "Connection",
            ),
          ),
          React.createElement(
            "div",
            { className: "settings-status-grid" },
            React.createElement(
              "div",
              null,
              React.createElement(
                "span",
                { className: "settings-label" },
                "Host",
              ),
              React.createElement(
                "strong",
                null,
                formatDaemonEndpoint(cortexBase),
              ),
            ),
            React.createElement(
              "div",
              null,
              React.createElement(
                "span",
                { className: "settings-label" },
                "Mode",
              ),
              React.createElement(
                "strong",
                null,
                isTauriRuntime ? "Desktop managed" : "Browser attach",
              ),
            ),
            React.createElement(
              "div",
              null,
              React.createElement(
                "span",
                { className: "settings-label" },
                "Daemon",
              ),
              React.createElement("strong", null, daemonStatusBadge.label),
            ),
          ),
          React.createElement(
            "button",
            {
              type: "button",
              className: "btn-sm",
              onClick: openConnectionDialog,
            },
            "Connection Settings",
          ),
        ),
        React.createElement(
          "section",
          {
            className: "settings-section settings-section-wide",
            "aria-labelledby": "settings-budgets",
          },
          React.createElement(
            "div",
            { className: "settings-section-head" },
            React.createElement(AppIcon, { name: "token", size: 18 }),
            React.createElement("h2", { id: "settings-budgets" }, "Budgets"),
            React.createElement(
              "span",
              {
                className: `settings-budget-pill ${budgetSummary.statusLabel.toLowerCase()}`,
              },
              budgetSummary.statusLabel,
            ),
          ),
          React.createElement(
            "div",
            { className: "settings-status-grid" },
            React.createElement(
              "div",
              null,
              React.createElement(
                "span",
                { className: "settings-label" },
                "Config",
              ),
              React.createElement(
                "strong",
                null,
                budgetSummary.configLoaded ? "Loaded" : "Not loaded",
              ),
            ),
            React.createElement(
              "div",
              null,
              React.createElement(
                "span",
                { className: "settings-label" },
                "Enforcement",
              ),
              React.createElement(
                "strong",
                null,
                budgetSummary.enabled ? "Enabled" : "Off",
              ),
            ),
            React.createElement(
              "div",
              null,
              React.createElement(
                "span",
                { className: "settings-label" },
                "Source",
              ),
              React.createElement(
                "strong",
                null,
                budgetSummary.source || "Default unlimited",
              ),
            ),
            React.createElement(
              "div",
              null,
              React.createElement(
                "span",
                { className: "settings-label" },
                "Recent Denials",
              ),
              React.createElement(
                "strong",
                null,
                budgetSummary.recentDenialsTotal,
              ),
            ),
          ),
          budgetSummary.error
            ? React.createElement(
                "p",
                { className: "settings-error", role: "alert" },
                budgetSummary.error,
              )
            : null,
          React.createElement(
            "div",
            { className: "settings-budget-table-wrap" },
            React.createElement(
              "table",
              { className: "settings-budget-table" },
              React.createElement(
                "caption",
                { className: "sr-only" },
                "Configured budget endpoints",
              ),
              React.createElement(
                "thead",
                null,
                React.createElement(
                  "tr",
                  null,
                  React.createElement("th", { scope: "col" }, "Endpoint"),
                  React.createElement("th", { scope: "col" }, "Limit"),
                  React.createElement("th", { scope: "col" }, "Window"),
                  React.createElement("th", { scope: "col" }, "Recent Denials"),
                ),
              ),
              React.createElement(
                "tbody",
                null,
                (budgetSummary.endpointRows.length
                  ? budgetSummary.endpointRows
                  : [{ endpoint: "none", limit: null, windowSeconds: null }]
                ).map((row) => {
                  const denial = budgetSummary.denialRows.find(
                    (entry) => entry.endpoint === row.endpoint,
                  )?.count;
                  return React.createElement(
                    "tr",
                    { key: row.endpoint },
                    React.createElement(
                      "th",
                      { scope: "row", "data-label": "Endpoint" },
                      row.endpoint,
                    ),
                    React.createElement(
                      "td",
                      { "data-label": "Limit" },
                      row.limit ?? "--",
                    ),
                    React.createElement(
                      "td",
                      { "data-label": "Window" },
                      row.windowSeconds ? `${row.windowSeconds}s` : "--",
                    ),
                    React.createElement(
                      "td",
                      { "data-label": "Recent Denials" },
                      denial ?? (budgetSummary.denialRows.length ? 0 : "--"),
                    ),
                  );
                }),
              ),
            ),
          ),
          React.createElement(
            "form",
            {
              className: "settings-budget-editor",
              onSubmit: saveBudgetConfigDraft,
            },
            React.createElement(
              "div",
              { className: "settings-budget-editor-head" },
              React.createElement(
                "label",
                { className: "settings-row settings-budget-defaults" },
                React.createElement(
                  "span",
                  null,
                  React.createElement("strong", null, "Enforce budgets"),
                  React.createElement(
                    "small",
                    null,
                    "Writes the local operator budget config.",
                  ),
                ),
                React.createElement("input", {
                  type: "checkbox",
                  checked: budgetDraft.enabled,
                  disabled: !ipcAvailable || budgetConfigBusy,
                  onChange: (event) =>
                    updateBudgetDraftRoot({ enabled: event.target.checked }),
                }),
              ),
              React.createElement(
                "div",
                { className: "settings-budget-actions" },
                React.createElement(
                  "button",
                  {
                    type: "button",
                    className: "btn-sm",
                    disabled: !ipcAvailable || budgetConfigBusy,
                    onClick: () => reloadBudgetConfigDraft(),
                  },
                  "Reload",
                ),
                React.createElement(
                  "button",
                  {
                    type: "submit",
                    className: "btn-sm btn-primary",
                    disabled:
                      !ipcAvailable || budgetConfigBusy || !!budgetDraftError,
                  },
                  budgetConfigBusy ? "Saving..." : "Save",
                ),
              ),
            ),
            React.createElement(
              "div",
              {
                className: "settings-budget-edit-grid",
                role: "group",
                "aria-label": "Budget endpoint editor",
              },
              BUDGET_ENDPOINT_DEFINITIONS.map((definition) => {
                const draft = budgetDraftEndpoints[definition.key],
                  endpointEnabled = !!draft?.enabled;
                return React.createElement(
                  "fieldset",
                  {
                    key: definition.key,
                    className: "settings-budget-edit-row",
                    disabled: !ipcAvailable || budgetConfigBusy,
                  },
                  React.createElement("legend", null, definition.label),
                  React.createElement(
                    "label",
                    { className: "settings-budget-enable" },
                    React.createElement("input", {
                      type: "checkbox",
                      checked: endpointEnabled,
                      onChange: (event) =>
                        updateBudgetEndpointDraft(definition.key, {
                          enabled: event.target.checked,
                        }),
                    }),
                    React.createElement("span", null, "Limited"),
                  ),
                  React.createElement(
                    "label",
                    { className: "settings-budget-input" },
                    React.createElement("span", null, "Calls"),
                    React.createElement("input", {
                      type: "number",
                      min: "1",
                      step: "1",
                      inputMode: "numeric",
                      value: draft?.limit ?? "",
                      disabled: !endpointEnabled,
                      onChange: (event) =>
                        updateBudgetEndpointDraft(definition.key, {
                          limit: event.target.value,
                        }),
                    }),
                  ),
                  React.createElement(
                    "label",
                    { className: "settings-budget-input" },
                    React.createElement("span", null, "Window"),
                    React.createElement("input", {
                      type: "number",
                      min: "1",
                      step: "1",
                      inputMode: "numeric",
                      value: draft?.windowSeconds ?? "",
                      disabled: !endpointEnabled,
                      onChange: (event) =>
                        updateBudgetEndpointDraft(definition.key, {
                          windowSeconds: event.target.value,
                        }),
                    }),
                  ),
                );
              }),
            ),
            ipcAvailable
              ? null
              : React.createElement(
                  "p",
                  { className: "settings-budget-note" },
                  "Budget edits require the desktop app.",
                ),
            budgetDraftError
              ? React.createElement(
                  "p",
                  { className: "settings-error", role: "alert" },
                  budgetDraftError,
                )
              : null,
            budgetConfigMessage
              ? React.createElement(
                  "p",
                  { className: "settings-budget-note", role: "status" },
                  budgetConfigMessage,
                )
              : null,
          ),
        ),
        React.createElement(
          "section",
          {
            className: "settings-section",
            "aria-labelledby": "settings-keyboard",
          },
          React.createElement(
            "div",
            { className: "settings-section-head" },
            React.createElement(AppIcon, { name: "work", size: 18 }),
            React.createElement(
              "h2",
              { id: "settings-keyboard" },
              "Keyboard & Navigation",
            ),
          ),
          React.createElement(
            "label",
            { className: "settings-row" },
            React.createElement(
              "span",
              null,
              React.createElement("strong", null, "Compact navigation"),
              React.createElement("small", null, "Denser sidebar controls."),
            ),
            React.createElement("input", {
              type: "checkbox",
              checked: controlSettings.compactNavigation,
              onChange: (event) =>
                updateControlSetting("compactNavigation", event.target.checked),
            }),
          ),
          controlSettings.keyboardHints
            ? React.createElement(
                "div",
                {
                  className: "settings-shortcut-grid",
                  role: "group",
                  "aria-label": "Keyboard shortcut summary",
                },
                PANEL_SEQUENCE.map((item, idx) =>
                  React.createElement(
                    "span",
                    { key: item.key },
                    React.createElement("kbd", null, idx + 1),
                    item.label,
                  ),
                ),
              )
            : null,
        ),
      ),
    ),
  );
}
export { SettingsPanel };
