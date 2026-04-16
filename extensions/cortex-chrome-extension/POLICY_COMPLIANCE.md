# Chrome Web Store Policy Alignment Notes

This extension implementation is designed to align with current Chrome extension guidance and reduce review risk.

## Design Decisions

1. MV3 service worker architecture only.
2. No remote code execution or dynamic script loading.
3. Minimal default host access (`localhost` and `127.0.0.1`).
4. Optional host permissions are requested only for the configured Cortex origin.
5. No ad-tech behaviors, no affiliate redirects, no external analytics beacons.
6. User-triggered memory operations (popup/context-menu), not silent scraping.

## Reviewer-Facing Behaviors

- Single purpose: Cortex memory store/recall workflow helper.
- Transparent data flow:
  - Selected/manual text -> Cortex REST endpoint configured by user.
  - No third-party relay endpoints.
- Host access:
  - Default loopback only.
  - Additional origins granted by explicit permission prompt.

## Canonical References

- Manifest V3 overview and migration: <https://developer.chrome.com/docs/extensions/develop/migrate/what-is-mv3>
- Extension platform security guidance: <https://developer.chrome.com/docs/extensions/develop/security-privacy/stay-secure>
- Permissions and optional permissions model: <https://developer.chrome.com/docs/extensions/reference/api/permissions>
- Chrome Web Store Program Policies: <https://developer.chrome.com/docs/webstore/program-policies/>
