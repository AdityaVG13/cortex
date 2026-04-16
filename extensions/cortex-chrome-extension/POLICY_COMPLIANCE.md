# Chrome Web Store Policy Alignment Notes

This extension implementation is designed to align with current Chrome extension guidance and reduce review risk.

## Design Decisions

1. Manifest V3 service worker architecture only.
2. No remotely hosted executable code, dynamic eval, or remote script loading.
3. Loopback-only host access (`http://127.0.0.1/*`, `http://localhost/*`) for local-first operation.
4. No wildcard host permissions (`https://*/*`, `<all_urls>`, `*://*/*`) in the Web Store build.
5. User-triggered operations only (popup actions and explicit context-menu selection).
6. API key persistence is opt-in; default is session-only storage.
7. Context-menu collection is scoped to selected text; page metadata is opt-in.

## Reviewer-Facing Behaviors

- Single purpose: store and recall Cortex memory.
- Transparent data flow:
  - User-selected/manual text -> user-configured local Cortex endpoint.
  - No third-party relay and no external analytics beacons.
- Clear local-first boundaries:
  - Web Store build accepts loopback endpoints only.
  - Remote integration is intentionally excluded from this package.

## Canonical References

- Declare permissions (least privilege): <https://developer.chrome.com/docs/extensions/develop/concepts/declare-permissions>
- User privacy and permission minimization: <https://developer.chrome.com/docs/extensions/develop/security-privacy/user-privacy>
- Improve extension security / remotely hosted code restrictions: <https://developer.chrome.com/docs/extensions/develop/migrate/improve-security>
- Chrome Web Store Program Policies: <https://developer.chrome.com/docs/webstore/program-policies/policies>
- Privacy policy requirement: <https://developer.chrome.com/docs/webstore/program-policies/privacy/>
- User Data FAQ (secure handling, minimum permissions): <https://developer.chrome.com/docs/webstore/program-policies/user-data-faq>
- Disclosure requirements: <https://developer.chrome.com/docs/webstore/program-policies/disclosure-requirements/>
- Web Store review process: <https://developer.chrome.com/docs/webstore/review-process/>
