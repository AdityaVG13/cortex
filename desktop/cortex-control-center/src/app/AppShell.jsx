import { useEffect } from "react";
import { installUpdate } from "../updater.js";
import { AppIcon } from "../ui-icons.jsx";
import { PANEL_SEQUENCE } from "./constants.js";
import { PanelStage } from "./panels/panel-stage.jsx";

export function AppShell(d) {
  const {
    effectiveSidebarCollapsed,
    panel,
    changePanel,
    pill,
    utilityPill,
    sidebarUtilityStats,
    activePanelLabel,
    daemonState,
    daemonRecoveryHint,
    handleRestartDaemon,
    restartingDaemon,
    invokeRef,
    handleStartDaemon,
    handleStopDaemon,
    canStartDaemon,
    canStopDaemon,
    restartError,
    availableUpdate,
    updateInstalling,
    setUpdateInstalling,
    setFeedbackMessage,
    feedbackMessage,
    setSidebarCollapsed,
    topbarRef,
    stats,
    normalizedSessions,
    openConnectionDialog,
    hostLabel,
    daemonStatusBadge,
    showEditorSetupWizard,
    isSettingUpEditors,
    closeEditorSetupWizard,
    editorSetupDialogRef,
    editorDetectionSummary,
    selectedEditorIds,
    toggleEditorSelection,
    manualMcpSnippet,
    applyEditorSetup,
    showConnectionDialog,
    dismissConnectionDialog,
    connectionDialogRef,
    connectionDialogTriggerRef,
    isTauriRuntime,
    connectionEndpoint,
    closeConnectionDialog,
    setCortexBase,
    tokenRef,
    persistBrowserAuthToken,
    readAuthToken,
    refreshAllRef,
    DEFAULT_CORTEX_BASE,
    trapFocusInContainer,
    restoreFocusToTrigger,
  } = d;

  useEffect(() => {
    if (!showConnectionDialog || !connectionDialogRef.current) return undefined;
    return trapFocusInContainer(connectionDialogRef.current);
  }, [showConnectionDialog]);

  useEffect(() => {
    if (!showEditorSetupWizard || !editorSetupDialogRef.current) return undefined;
    return trapFocusInContainer(editorSetupDialogRef.current);
  }, [showEditorSetupWizard]);

  return (
    <div className={`app ${effectiveSidebarCollapsed ? "sidebar-collapsed" : ""}`}>
      <a className="skip-link" href="#main-content">Skip to main content</a>
      <aside className={`sidebar ${effectiveSidebarCollapsed ? "collapsed" : ""}`} aria-labelledby="sidebar-title">
        <div className="sidebar-header">
          <div className="logo">
            <span id="sidebar-title">Cortex</span>
          </div>
          <div className={pill.className}>{pill.label}</div>
        </div>

        <nav className="sidebar-nav" aria-label="Primary panels">
          {PANEL_SEQUENCE.map((item, idx) => (
            <button
              key={item.key}
              type="button"
              className={`nav-item ${panel === item.key ? "active" : ""}`}
              onClick={() => changePanel(item.key)}
              data-key={idx + 1}
              aria-current={panel === item.key ? "page" : undefined}
            >
              <span style={{ opacity: 0.5, fontSize: "12px" }}><AppIcon name={item.icon} /></span>
              {item.label}
            </button>
          ))}
        </nav>

        <div className="sidebar-utility">
          <div className="sidebar-utility-header">
            <span className="sidebar-utility-kicker">Mission status</span>
            <span className={`sidebar-utility-pill ${utilityPill.className}`}>
              {utilityPill.label}
            </span>
          </div>
          <div className="sidebar-utility-grid">
            {sidebarUtilityStats.map((item) => (
              <div key={item.label} className={`sidebar-utility-card tone-${item.tone}`}>
                <span className="sidebar-utility-label">{item.label}</span>
                <strong className="sidebar-utility-value">{item.value}</strong>
              </div>
            ))}
          </div>
          <div className="sidebar-utility-note">
            <span className="sidebar-utility-note-label">Focus</span>
            <strong>{activePanelLabel}</strong>
            <p>{daemonState.message}</p>
            {daemonRecoveryHint ? <p className="sidebar-utility-alert">{daemonRecoveryHint}</p> : null}
          </div>
        </div>

        <div className="sidebar-footer">
          <div className="daemon-restart-row">
            <button
              type="button"
              className="btn-ctrl btn-restart"
              onClick={handleRestartDaemon}
              disabled={restartingDaemon || !invokeRef.current}
            >
              {restartingDaemon ? "Restarting..." : "Restart"}
            </button>
          </div>
          <div className="daemon-controls-grid">
            <button type="button" className="btn-ctrl btn-primary" onClick={handleStartDaemon} disabled={!canStartDaemon}>Start</button>
            <button type="button" className="btn-ctrl" onClick={handleStopDaemon} disabled={!canStopDaemon}>Stop</button>
            <button type="button" className="btn-ctrl btn-danger" onClick={async () => {
              if (invokeRef.current) {
                try { await d.call("quit_app"); } catch { /* app is exiting */ }
              }
            }}>Exit</button>
          </div>
          {restartError ? (
            <button type="button" className="btn-sm btn-danger btn-restart-retry" onClick={handleRestartDaemon}>
              Retry Restart
            </button>
          ) : null}
          {availableUpdate && (
            <div className="update-banner">
              <span>v{availableUpdate.version} available</span>
              <button
                type="button"
                className="btn-sm btn-primary"
                disabled={updateInstalling}
                onClick={async () => {
                  setUpdateInstalling(true);
                  setFeedbackMessage("Downloading update...");
                  try {
                    await installUpdate(availableUpdate);
                  } catch (err) {
                    setFeedbackMessage(`Update failed: ${String(err)}`);
                    setUpdateInstalling(false);
                  }
                }}
              >
                {updateInstalling ? "Installing..." : "Update"}
              </button>
            </div>
          )}
          <p className="sidebar-status" aria-hidden="true">{feedbackMessage}</p>
          <button
            type="button"
            className="btn-sidebar-collapse"
            aria-label={effectiveSidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
            title={effectiveSidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
            onClick={() => setSidebarCollapsed((c) => !c)}
          >
            <AppIcon name={effectiveSidebarCollapsed ? "chevron-right" : "chevron-left"} size={16} />
          </button>
        </div>
      </aside>

      <main id="main-content" className="content" tabIndex={-1}>
        <p className="sr-only" role="status" aria-live="polite" aria-atomic="true">
          {feedbackMessage}
        </p>
        <div
          ref={topbarRef}
          className={`topbar ${panel === "overview" ? "topbar-hidden" : ""}`}
          aria-hidden={panel === "overview" ? true : undefined}
        >
          <div className="topbar-left">
            <span className="topbar-path">CORTEX</span>
            <span className="topbar-sep">/</span>
            <span className="topbar-current">{activePanelLabel.toUpperCase()}</span>
          </div>
          <div className="topbar-right">
            <span className="topbar-stat"><span className="topbar-label">MEM</span> {stats.memories}</span>
            <span className="topbar-stat"><span className="topbar-label">DEC</span> {stats.decisions}</span>
            <span className="topbar-stat"><span className="topbar-label">EVT</span> {stats.events}</span>
            <span className="topbar-stat"><span className="topbar-label">AGENTS</span> {normalizedSessions.length}</span>
            <button
              type="button"
              className="topbar-stat topbar-connection"
              onClick={openConnectionDialog}
              tabIndex={panel === "overview" ? -1 : undefined}
              title="Click to change connection"
              aria-label={`Connection host ${hostLabel}. Open connection settings.`}
            >
              <span className="topbar-label">HOST</span>
              {hostLabel}
            </button>
            <span className={`topbar-status ${daemonStatusBadge.className}`} title={daemonStatusBadge.title}>
              {daemonStatusBadge.label}
            </span>
          </div>
        </div>

        {showEditorSetupWizard && (
          <div className="connection-overlay" role="presentation" onClick={() => !isSettingUpEditors && closeEditorSetupWizard()}>
            <div
              ref={editorSetupDialogRef}
              className="connection-dialog editor-setup-dialog"
              role="dialog"
              aria-modal="true"
              aria-labelledby="editor-setup-title"
              aria-describedby="editor-setup-description"
              aria-busy={isSettingUpEditors ? true : undefined}
              tabIndex={-1}
              onClick={(e) => e.stopPropagation()}
            >
              <div className="editor-setup-dialog-header">
                <div>
                  <span className="editor-setup-kicker">Shared MCP Registration</span>
                  <h2 id="editor-setup-title">Setup MCP</h2>
                </div>
                <span className="badge">
                  {editorDetectionSummary.detected}/{editorDetectionSummary.results.length}
                </span>
              </div>
              <p className="connection-subtitle" id="editor-setup-description">
                Choose which supported clients should receive the shared Cortex attach-only MCP entry. Every client points at the same
                app-owned daemon command.
              </p>
              <div className="editor-setup-choice-list">
                {editorDetectionSummary.results.map((entry) => {
                  const tone = !entry.detected ? "idle" : entry.registered ? "ok" : "warn";
                  const stateLabel = !entry.detected ? "Not detected" : entry.registered ? "Configured" : "Detected";
                  const selected = selectedEditorIds.includes(entry.id);
                  return (
                    <label key={entry.id} className={`editor-setup-choice ${tone} ${!entry.detected ? "disabled" : ""}`}>
                      <input
                        type="checkbox"
                        checked={selected}
                        disabled={!entry.detected || isSettingUpEditors}
                        onChange={() => toggleEditorSelection(entry.id)}
                      />
                      <div className="editor-setup-choice-body">
                        <div className="editor-setup-item-head">
                          <span className="editor-setup-name">{entry.name}</span>
                          <span className="editor-setup-state">{stateLabel}</span>
                        </div>
                        {entry.configPath ? <code>{entry.configPath}</code> : null}
                        <p>{entry.message || "No detail provided."}</p>
                      </div>
                    </label>
                  );
                })}
              </div>
              <div className="editor-setup-manual">
                <span className="editor-setup-kicker">Manual Fallback</span>
                <p>If a client is missing from the supported list, register this MCP server manually or paste it into that AI&apos;s setup flow:</p>
                <pre>{manualMcpSnippet}</pre>
                <p>Replace <code>codex</code> with that AI&apos;s agent ID (for example: <code>claude</code>, <code>cursor</code>, <code>gemini</code>).</p>
              </div>
              <div className="connection-actions">
                <button type="button" className="btn-sm" onClick={closeEditorSetupWizard} disabled={isSettingUpEditors}>
                  Cancel
                </button>
                <button
                  type="button"
                  className="btn-sm btn-primary"
                  onClick={applyEditorSetup}
                  disabled={isSettingUpEditors || !selectedEditorIds.length}
                >
                  {isSettingUpEditors ? "Applying..." : `Apply to ${selectedEditorIds.length} Client${selectedEditorIds.length === 1 ? "" : "s"}`}
                </button>
              </div>
            </div>
          </div>
        )}

        {showConnectionDialog && (
          <div className="connection-overlay" role="presentation" onClick={dismissConnectionDialog}>
            <div
              ref={connectionDialogRef}
              className="connection-dialog"
              role="dialog"
              aria-modal="true"
              aria-labelledby="connection-dialog-title"
              aria-describedby="connection-dialog-description"
              tabIndex={-1}
              onClick={(e) => e.stopPropagation()}
            >
              <div className="connection-dialog-header">
                <h2 id="connection-dialog-title">Connection Settings</h2>
                <button
                  type="button"
                  className="connection-dialog-close"
                  aria-label="Close connection settings"
                  onClick={dismissConnectionDialog}
                >
                  ×
                </button>
              </div>
              <p className="connection-subtitle" id="connection-dialog-description">
                {isTauriRuntime
                  ? "Desktop app mode uses the local app-managed Cortex daemon only."
                  : "Connect to a local or remote Cortex daemon"}
              </p>
              <form onSubmit={(e) => {
                e.preventDefault();
                if (isTauriRuntime) {
                  setCortexBase(DEFAULT_CORTEX_BASE);
                  tokenRef.current = "";
                  persistBrowserAuthToken("");
                  closeConnectionDialog();
                  queueMicrotask(() => refreshAllRef.current());
                  return;
                }
                const fd = new FormData(e.target);
                const host = fd.get("host")?.toString().trim() || "127.0.0.1";
                const port = fd.get("port")?.toString().trim() || "7437";
                const token = fd.get("token")?.toString().trim();
                setCortexBase(`http://${host}:${port}`);
                tokenRef.current = token || "";
                persistBrowserAuthToken(token || "");
                closeConnectionDialog();
                queueMicrotask(() => refreshAllRef.current());
              }}>
                <label className="connection-field">
                  <span>Host</span>
                  <input
                    name="host"
                    defaultValue={connectionEndpoint.host}
                    placeholder="127.0.0.1"
                    disabled={isTauriRuntime}
                  />
                </label>
                <label className="connection-field">
                  <span>Port</span>
                  <input
                    name="port"
                    defaultValue={connectionEndpoint.port}
                    placeholder="7437"
                    disabled={isTauriRuntime}
                  />
                </label>
                <label className="connection-field">
                  <span>Auth Token</span>
                  <input
                    name="token"
                    type="password"
                    placeholder={isTauriRuntime ? "Managed by desktop app token flow" : "Leave blank for local (auto-read)"}
                    disabled={isTauriRuntime}
                  />
                </label>
                <div className="connection-actions">
                  <button type="button" className="btn-sm" onClick={() => {
                    setCortexBase(DEFAULT_CORTEX_BASE);
                    tokenRef.current = "";
                    persistBrowserAuthToken("");
                    closeConnectionDialog();
                    readAuthToken({ suppressFeedback: true });
                    queueMicrotask(() => refreshAllRef.current());
                  }}>Reset to Local</button>
                  <button type="submit" className="btn-sm btn-primary">Connect</button>
                </div>
              </form>
            </div>
          </div>
        )}

        <PanelStage {...d} />
      </main>
    </div>
  );
}
