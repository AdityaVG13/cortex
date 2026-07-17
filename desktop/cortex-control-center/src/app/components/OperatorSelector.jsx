import React from "react";
import { useId } from "react";
function OperatorSelector({
  value,
  knownAgents,
  onChange,
  label = "Operator",
  placeholder = "codex",
}) {
  const datalistId = useId();
  return React.createElement(
    "label",
    { className: "feed-control" },
    React.createElement("span", null, label),
    React.createElement("input", {
      type: "text",
      list: datalistId,
      placeholder,
      value,
      onChange: (event) => onChange(event.target.value),
    }),
    React.createElement(
      "datalist",
      { id: datalistId },
      knownAgents.map((agent) =>
        React.createElement("option", { key: agent, value: agent }),
      ),
    ),
  );
}
export { OperatorSelector };
