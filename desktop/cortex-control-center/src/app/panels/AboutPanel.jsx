import React from "react";
import { useDashboard } from "../DashboardContext.jsx";
import { CONTROL_CENTER_VERSION } from "../constants.js";
function AboutPanel() { const { panel, stats } = useDashboard();
  return ( <React.Fragment>
      {panel === "about" ? ( <section className="panel active">
          <div className="panel-header">
            <div>
              <h1>About</h1>
              <p className="panel-subtitle">
                Shipping surface, runtime contract, and contributor credits for Cortex Control Center.
              </p>
            </div>
          </div>
          <div className="card full">
            <div className="about-content">
              <div className="about-brand">
                <img
                  src={`${import.meta.env.BASE_URL}icons/icon.png`}
                  alt="Cortex"
                  className="about-logo"
                  onError={(event) => { ((event.currentTarget.style.display = "none"), (event.currentTarget.nextSibling.style.display = "flex"));
                  }} />
                <div className="about-logo about-logo-fallback">CC</div>
                <div className="about-heading">
                  <h2 className="about-title">Cortex Control Center</h2>
                  <p className="about-version">
                    {"Built by the Cortex maintainer team -- Version "}
                    {CONTROL_CENTER_VERSION}
                  </p>
                </div>
              </div>
              <p className="about-description">
                A desktop command surface for Cortex built around one app-managed daemon instance: auth-aware startup,
                owned lifecycle control, live telemetry, and a brain view that can double as a showpiece.
              </p>
              <div className="about-stats-grid">
                {[ ["Daemon", "Rust + Axum"], ["Desktop shell", "Tauri + React"], ["Embeddings", "ONNX (all-MiniLM-L6-v2)"],
                  ["Storage", "SQLite (WAL)"], ["Transport", "HTTP + MCP stdio"], ["Port", "7437"], ].map(([label, value]) => (
                  <div key={label} className="about-stat-card">
                    <span className="about-stat-label">{label}</span>
                    <div className="about-stat-value">{value}</div>
                  </div>
                ))}
              </div>
              <div className="about-section">
                <h3 className="about-section-title">App Lifecycle</h3>
                <table className="about-lifecycle-table">
                  <thead>
                    <tr>
                      <th>Action</th>
                      <th>What happens</th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr>
                      <td>Start</td>
                      <td>Launches the app-managed Cortex daemon and waits for a healthy API before reloading data.</td>
                    </tr>
                    <tr>
                      <td>Stop</td>
                      <td>
                        Sends a graceful shutdown request to the app-managed daemon, then clears owned process handles.
                      </td>
                    </tr>
                    <tr>
                      <td>Restart</td>
                      <td>
                        Runs Stop then Start with timeout handling so the UI can recover from stale daemon state without
                        creating a second instance.
                      </td>
                    </tr>
                    <tr>
                      <td>Close Window</td>
                      <td>
                        Minimizes to tray by default so the app-managed daemon can keep serving local clients in the
                        background.
                      </td>
                    </tr>
                    <tr>
                      <td>Exit</td>
                      <td>Fully quits the app and requests daemon shutdown when this app instance owns it.</td>
                    </tr>
                  </tbody>
                </table>
              </div>
              <div className="about-section">
                <h3 className="about-section-title">Contributors</h3>
                <div className="about-contributors">
                  {[ { handle: "Cortex-Team", role: "Creator & maintainer" }, { handle: "Claude Code",
                      role: "Core architecture & retrieval pipeline", }, { handle: "Factory Droid",
                      role: "Desktop app, reconnection & telemetry", }, { handle: "Codex", role: "Desktop rewrite, auth hardening, analytics and brain UX", },
                  ].map(({ handle, role }) => ( <div key={handle} className="about-contributor">
                      <span
                        className="agent-indicator"
                        style={{ background: "var(--cyan)", boxShadow: "0 0 8px var(--cyan)", }} />
                      <span className="about-contributor-handle">@{handle}</span>
                      <span className="about-contributor-role">{role}</span>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          </div>
        </section>
      ) : null}
    </React.Fragment> );
}
export { AboutPanel };
