export function pickDefined(...values) {
  for (const value of values) {
    if (value !== undefined && value !== null && value !== "") {
      return value;
    }
  }
  return null;
}

export function normalizePermissionGrant(entry, index) {
  const client = String(pickDefined(entry?.client, entry?.client_id, entry?.clientId, "unknown") || "unknown");
  const permission = String(pickDefined(entry?.permission, "read") || "read").toLowerCase();
  const scope = String(pickDefined(entry?.scope, "*") || "*");
  const grantedBy = String(pickDefined(entry?.grantedBy, entry?.granted_by, "") || "");
  const grantedAt = String(pickDefined(entry?.grantedAt, entry?.granted_at, "") || "");
  return {
    key: `${client}-${permission}-${scope}-${index}`,
    client,
    permission,
    scope,
    grantedBy,
    grantedAt,
  };
}

export function normalizePermissionPayload(payload) {
  const grants = Array.isArray(payload?.grants) ? payload.grants : [];
  return grants.map((entry, index) => normalizePermissionGrant(entry, index));
}
