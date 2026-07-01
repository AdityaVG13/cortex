import { useId } from "react";

export function OperatorSelector({ value, knownAgents, onChange, label = "Operator", placeholder = "codex" }) {
  const datalistId = useId();
  return (
    <label className="feed-control">
      <span>{label}</span>
      <input
        type="text"
        list={datalistId}
        placeholder={placeholder}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
      <datalist id={datalistId}>
        {knownAgents.map((agent) => (
          <option key={agent} value={agent} />
        ))}
      </datalist>
    </label>
  );
}
