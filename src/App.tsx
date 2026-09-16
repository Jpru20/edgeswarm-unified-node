import { FormEvent, useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import "./App.css";
import swarmLogo from "./assets/swarm-logo.png";

type Screen = "login" | "mfa" | "dashboard";

type AuthBeginResult = {
  email: string;
  mfaRequired: boolean;
};

type AuthVerifyResult = {
  email: string;
};

type ProviderLedgerSummary = {
  totalEarnedUsd: number;
  syncedAt?: string | null;
};

type ModelDownloadProgress = {
  status: string;
  filename: string;
  downloadedBytes: number;
  totalBytes: number;
  percent: number;
  bytesPerSecond: number;
  etaSeconds?: number | null;
};

type CertificationProgress = {
  state: string;
  maximumConcurrency: number;
  currentConcurrency?: number | null;
  completedWorkloads: number;
  totalWorkloads: number;
  testedConcurrencyLevels: number[];
  certifiedConcurrency: number;
  rejectedConcurrency?: number | null;
  lastError?: string | null;
};

type NodeServiceStatus = {
  running: boolean;
  desiredRunning: boolean;
  stopping: boolean;
  lastError?: string | null;
  logs: string[];
  modelDownload?: ModelDownloadProgress | null;
  certification?: CertificationProgress | null;
  capacityTestRequested?: boolean;
};

type ModelState = {
  selectedModel: string;
  modelId: string;
  capability: string;
  tier: number;
  runtime: string;
  acceleration: string;
  status: string;
  capacityStatus: string;
  certifiedConcurrency?: number | null;
  downloadProgressPct?: number | null;
};

type NodeState = {
  hardwareIdentity: {
    hardwareId: string;
    source: string;
  };
  platform: {
    os: string;
  };
  hardware: {
    logicalCpuCount: number;
    physicalCpuCount?: number | null;
    cpuBrand: string;
    cpuVendor: string;
    totalMemoryBytes: number;
    availableMemoryBytes: number;
  };
  acceleration: {
    backend: string;
    deviceName?: string | null;
  };
  models: ModelState[];
};

function App() {
  const [appVersion, setAppVersion] = useState("");

  useEffect(() => {
    getVersion()
      .then(setAppVersion)
      .catch(() => setAppVersion("unknown"));
  }, []);

  const [screen, setScreen] = useState<Screen>("login");
  const [email, setEmail] = useState("");
  const [providerEmail, setProviderEmail] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [mfaCode, setMfaCode] = useState("");
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);
  const [nodeState, setNodeState] = useState<NodeState | null>(null);
  const [serviceStatus, setServiceStatus] =
    useState<NodeServiceStatus>({
      running: false,
      desiredRunning: false,
      stopping: false,
      lastError: null,
      logs: [],
      modelDownload: null,
    });
  const [nodeActionBusy, setNodeActionBusy] = useState(false);

  // PROVIDER_LEDGER_UI_V1
  const [totalEarnedUsd, setTotalEarnedUsd] = useState(0);
  const [ledgerBusy, setLedgerBusy] = useState(false);
  const ledgerSyncInFlightRef = useRef(false);
  const lastLedgerTriggerRef = useRef("");

  // CERTIFIED_CAPACITY_EVENT_REFRESH_V3
  // Full NodeState detection is expensive. Refresh it only when
  // provider/certification state actually changes.
  const nodeStateRefreshKeyRef = useRef("");

  const consoleRef = useRef<HTMLElement | null>(null);
  const shouldFollowConsoleRef = useRef(true);

  async function loadNodeState() {
    const state = await invoke<NodeState>("get_node_state");
    setNodeState(state);
  }

  const syncLedger = useCallback(async () => {
    if (ledgerSyncInFlightRef.current) {
      return;
    }

    ledgerSyncInFlightRef.current = true;
    setLedgerBusy(true);

    try {
      const ledger =
        await invoke<ProviderLedgerSummary>(
          "provider_ledger_sync",
        );

      if (
        Number.isFinite(ledger.totalEarnedUsd) &&
        ledger.totalEarnedUsd >= 0
      ) {
        setTotalEarnedUsd(ledger.totalEarnedUsd);
      }
    } catch {
      // Keep the last verified balance if a refresh temporarily fails.
    } finally {
      ledgerSyncInFlightRef.current = false;
      setLedgerBusy(false);
    }
  }, []);

  async function toggleNode() {
    setNodeActionBusy(true);

    try {
      const command = serviceStatus.desiredRunning
        ? "stop_node"
        : "start_node";

      const current =
        await invoke<NodeServiceStatus>(command);

      setServiceStatus(current);
      setStatus("");
    } catch (error) {
      setStatus(String(error));
    } finally {
      setNodeActionBusy(false);
    }
  }

  useEffect(() => {
    void invoke("set_window_layout", { screen });
  }, [screen]);

  useEffect(() => {
    if (screen !== "dashboard") {
      return;
    }

    let disposed = false;

    const refresh = async () => {
      try {
        const current =
          await invoke<NodeServiceStatus>(
            "node_service_status",
          );

        if (!disposed) {
          setServiceStatus(current);
        }
      } catch {
        // The dashboard remains usable if a local status refresh fails.
      }
    };

    void refresh();

    const timer = window.setInterval(
      () => void refresh(),
      1000,
    );

    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [screen]);

  useEffect(() => {
    if (screen !== "dashboard") {
      return;
    }

    void syncLedger();

    const timer = window.setInterval(
      () => void syncLedger(),
      30000,
    );

    return () => window.clearInterval(timer);
  }, [screen, syncLedger]);

  useEffect(() => {
    if (screen !== "dashboard") {
      return;
    }

    const resultLine = [...serviceStatus.logs]
      .reverse()
      .find((line) => {
        const value = line.toLowerCase();

        return (
          value.includes("result") &&
          (
            value.includes("submit") ||
            value.includes("accepted") ||
            value.includes("completed")
          )
        );
      });

    if (
      !resultLine ||
      resultLine === lastLedgerTriggerRef.current
    ) {
      return;
    }

    lastLedgerTriggerRef.current = resultLine;
    void syncLedger();
  }, [screen, serviceStatus.logs, syncLedger]);

  useEffect(() => {
    const consoleElement = consoleRef.current;

    if (consoleElement && shouldFollowConsoleRef.current) {
      consoleElement.scrollTop = consoleElement.scrollHeight;
    }
  }, [serviceStatus.logs, serviceStatus.lastError]);


  // CERTIFIED_CAPACITY_EVENT_REFRESH_V3
  //
  // Never poll get_node_state: it performs full machine/model
  // detection. Refresh only on meaningful provider state changes.
  useEffect(() => {
    if (screen !== "dashboard") {
      nodeStateRefreshKeyRef.current = "";
      return;
    }

    const certificationState =
      serviceStatus.certification?.state
        ?.trim()
        .toLowerCase() ?? "";

    const certifiedConcurrency =
      serviceStatus.certification?.certifiedConcurrency ?? 0;

    const providerReady =
      serviceStatus.running;

    const certificationComplete =
      certificationState === "complete";

    if (!providerReady && !certificationComplete) {
      return;
    }

    const refreshKey =
      `${providerReady}:${certificationState}:${certifiedConcurrency}`;

    if (
      nodeStateRefreshKeyRef.current === refreshKey
    ) {
      return;
    }

    nodeStateRefreshKeyRef.current = refreshKey;

    void loadNodeState().catch(() => {
      // Preserve the last valid NodeState.
    });
  }, [
    screen,
    serviceStatus.running,
    serviceStatus.certification?.state,
    serviceStatus.certification?.certifiedConcurrency,
  ]);

  async function handleLogin(event: FormEvent) {
    event.preventDefault();

    if (!email.trim() || !password) {
      setStatus("Enter your provider email and password.");
      return;
    }

    setBusy(true);
    setStatus("Authenticating...");

    try {
      const result = await invoke<AuthBeginResult>("auth_begin", {
        email: email.trim().toLowerCase(),
        password,
      });

      setProviderEmail(result.email);
      setPassword("");
      setStatus("");

      if (result.mfaRequired) {
        setScreen("mfa");
      }
    } catch (error) {
      setStatus(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function handleMfa(event: FormEvent) {
    event.preventDefault();

    if (!/^\d{6}$/.test(mfaCode.trim())) {
      setStatus("Enter the 6-digit authenticator code.");
      return;
    }

    setBusy(true);
    setStatus("Verifying...");

    try {
      const result = await invoke<AuthVerifyResult>("auth_verify", {
        code: mfaCode.trim(),
      });

      setProviderEmail(result.email);
      setMfaCode("");
      await loadNodeState();
      setStatus("");
      setScreen("dashboard");
    } catch (error) {
      setStatus(String(error));
    } finally {
      setBusy(false);
    }
  }

  if (screen === "login") {
    return (
      <main className="app auth-screen">
        <section className="auth-card">
          <h1>Swarm Authentication</h1>

          <form onSubmit={handleLogin}>
            <input
              type="text"
              inputMode="email"
              autoComplete="email"
              autoCapitalize="none"
              spellCheck={false}
              placeholder="Provider Email"
              value={email}
              onChange={(event) => setEmail(event.currentTarget.value)}
            />

            <div className="password-row">
              <input
                type={showPassword ? "text" : "password"}
                autoComplete="current-password"
                placeholder="Password"
                value={password}
                onChange={(event) => setPassword(event.currentTarget.value)}
              />

              <button
                className="show-button"
                type="button"
                onClick={() => setShowPassword((value) => !value)}
              >
                {showPassword ? "Hide" : "Show"}
              </button>
            </div>

            <button className="primary-button" type="submit" disabled={busy}>
              {busy ? "Authenticating..." : "Sign In"}
            </button>
          </form>

          <div className={`status-message ${busy ? "working" : ""}`}>{status}</div>
        </section>
      </main>
    );
  }

  if (screen === "mfa") {
    return (
      <main className="app auth-screen">
        <section className="auth-card">
          <h1>Two-Factor Authentication</h1>

          <p className="muted">
            Enter the 6-digit code from your authenticator app.
          </p>

          <form onSubmit={handleMfa}>
            <input
              className="mfa-input"
              inputMode="numeric"
              maxLength={6}
              placeholder="000000"
              value={mfaCode}
              onChange={(event) =>
                setMfaCode(event.currentTarget.value.replace(/\D/g, ""))
              }
            />

            <button className="verify-button" type="submit" disabled={busy}>
              {busy ? "Verifying..." : "Verify"}
            </button>
          </form>

          <div className={`status-message ${busy ? "working" : ""}`}>{status}</div>
        </section>
      </main>
    );
  }

  const models = nodeState?.models ?? [];

  const readyModel =
    models.find(
      (model) =>
        model.status.toLowerCase() === "ready" &&
        model.capacityStatus.toLowerCase() === "certified",
    ) ?? null;

  const visibleModel = readyModel ?? models[0] ?? null;



  const ramGb = nodeState?.hardware
    ? Math.round(nodeState.hardware.totalMemoryBytes / (1024 ** 3))
    : null;

  const cpuName = nodeState?.hardware?.cpuBrand || "Unknown CPU";
  const gpuName = nodeState?.acceleration?.deviceName || "None";

  const capabilityText = readyModel
    ? `Ready - ${readyModel.capability} - ${readyModel.runtime} / ${readyModel.acceleration}`
    : visibleModel
      ? `${visibleModel.status} - ${visibleModel.capability}`
      : "Configuring device capabilities...";

  const earningsText =
    totalEarnedUsd === 0
      ? "$0.00 USD"
      : `$${totalEarnedUsd.toFixed(8)} USD`;

  return (
    <main className="dashboard-modern">
      <div className="swarm-shell">
        <header className="swarm-topbar">
          <div className="swarm-brand">
            <img
              className="swarm-brand-logo"
              src={swarmLogo}
              alt="Swarm"
            />
            <div>
              <div className="swarm-brand-name">Swarm</div>
              <div className="swarm-brand-subtitle">
                Distributed AI Provider
              </div>
            </div>
          </div>

          <div className="swarm-topbar-meta">
            <span className="swarm-version">
              v{appVersion || "..."}
            </span>
            <span
              className={`swarm-status-pill ${
                serviceStatus.running ? "online" : "offline"
              }`}
            >
              <span className="swarm-status-dot" />
              {serviceStatus.stopping
                ? "Stopping"
                : serviceStatus.running
                  ? "Online"
                  : "Offline"}
            </span>
          </div>
        </header>

        <section className="swarm-hero">
          <div className="swarm-hero-copy">
            <div className="swarm-eyebrow">
              PROVIDER NODE
            </div>

            <h1>
              {serviceStatus.running
                ? "Your node is online"
                : "Your node is paused"}
            </h1>

            <p>
              {serviceStatus.running
                ? `${gpuName} \u00B7 ${
                    visibleModel?.acceleration ??
                    nodeState?.acceleration.backend ??
                    "Detecting runtime"
                  }`
                : "Start your node to make this device available to Swarm."}
            </p>
          </div>

          <button
            className={`swarm-primary-button ${
              serviceStatus.desiredRunning ? "stop" : ""
            }`}
            type="button"
            onClick={toggleNode}
            disabled={nodeActionBusy || serviceStatus.stopping}
          >
            {serviceStatus.stopping
              ? "Stopping..."
              : serviceStatus.desiredRunning
                ? "Stop node"
                : "Start node"}
          </button>
        </section>

        {serviceStatus.certification?.state === "running" && (
          <section className="swarm-card swarm-certification-card">
            <div className="swarm-card-heading">
              <div>
                <div className="swarm-eyebrow">
                  DEVICE OPTIMIZATION
                </div>
                <h2>Finding your best capacity</h2>
              </div>
              <div className="swarm-live-badge">
                Testing
              </div>
            </div>

            <p className="swarm-card-description">
              Swarm is testing how many AI tasks this device can
              run reliably at the same time.
            </p>

            <div className="swarm-cert-list">
              {Array.from(
                {
                  length:
                    serviceStatus.certification.maximumConcurrency,
                },
                (_, index) => index + 1,
              ).map((level) => {
                const cert = serviceStatus.certification!;
                const rejected =
                  cert.rejectedConcurrency === level;
                const passed =
                  cert.testedConcurrencyLevels.includes(level) &&
                  level <= cert.certifiedConcurrency;
                const testing =
                  cert.currentConcurrency === level;

                return (
                  <div className="swarm-cert-row" key={level}>
                    <div>
                      <strong>
                        {level} worker{level === 1 ? "" : "s"}
                      </strong>
                      <span>
                        {level === 1
                          ? "Single AI task"
                          : `${level} simultaneous AI tasks`}
                      </span>
                    </div>

                    <span
                      className={`swarm-cert-state ${
                        rejected
                          ? "rejected"
                          : passed
                            ? "passed"
                            : testing
                              ? "testing"
                              : "waiting"
                      }`}
                    >
                      {rejected
                        ? "Rejected"
                        : passed
                          ? "Pass"
                          : testing
                            ? `${cert.completedWorkloads}/${cert.totalWorkloads}`
                            : "Waiting"}
                    </span>
                  </div>
                );
              })}
            </div>
          </section>
        )}

        {serviceStatus.modelDownload &&
          ["downloading", "verifying"].includes(
            serviceStatus.modelDownload.status,
          ) && (
            <section className="swarm-card">
              <div className="swarm-card-heading">
                <div>
                  <div className="swarm-eyebrow">
                    MODEL SETUP
                  </div>
                  <h2>
                    {serviceStatus.modelDownload.status ===
                    "verifying"
                      ? "Verifying model"
                      : "Preparing your AI model"}
                  </h2>
                </div>

                <strong>
                  {serviceStatus.modelDownload.percent.toFixed(0)}%
                </strong>
              </div>

              <div className="swarm-progress-track">
                <div
                  className="swarm-progress-fill"
                  style={{
                    width: `${Math.min(
                      100,
                      Math.max(
                        0,
                        serviceStatus.modelDownload.percent,
                      ),
                    )}%`,
                  }}
                />
              </div>
            </section>
          )}

        <div className="swarm-metric-grid">
          <section className="swarm-card swarm-earnings-card">
            <div className="swarm-eyebrow">EARNINGS</div>
            <div className="swarm-metric-value">
              {earningsText}
            </div>
            <div className="swarm-muted">
              Total earned
            </div>

            <button
              className="swarm-text-button"
              type="button"
              onClick={() => void syncLedger()}
              disabled={ledgerBusy}
            >
              {ledgerBusy ? "Syncing..." : "Refresh earnings"}
            </button>
          </section>

          <section className="swarm-card">
            <div className="swarm-eyebrow">
              CERTIFIED CAPACITY
            </div>
            <div className="swarm-metric-value">
              {visibleModel?.certifiedConcurrency ?? "?"}
            </div>
            <div className="swarm-muted">
              concurrent task
              {visibleModel?.certifiedConcurrency === 1
                ? ""
                : "s"}
            </div>
          </section>
        </div>

        <section className="swarm-card">
          <div className="swarm-card-heading">
            <div>
              <div className="swarm-eyebrow">
                DEVICE PERFORMANCE
              </div>
              <h2>
                {visibleModel?.selectedModel ??
                  "Preparing device"}
              </h2>
            </div>

            <span
              className={`swarm-model-badge ${
                readyModel ? "ready" : ""
              }`}
            >
              {capabilityText}
            </span>
          </div>

          <div className="swarm-detail-list">
            <div className="swarm-detail-row">
              <span>AI model</span>
              <strong>
                {visibleModel?.selectedModel ?? "Detecting"}
              </strong>
            </div>

            <div className="swarm-detail-row">
              <span>Acceleration</span>
              <strong>
                {visibleModel?.acceleration ??
                  nodeState?.acceleration.backend ??
                  "Detecting"}
              </strong>
            </div>

            <div className="swarm-detail-row">
              <span>GPU</span>
              <strong>{gpuName}</strong>
            </div>

            <div className="swarm-detail-row">
              <span>Memory</span>
              <strong>
                {ramGb ?? "?"} GB
              </strong>
            </div>

            <div className="swarm-detail-row">
              <span>Processor</span>
              <strong>{cpuName}</strong>
            </div>
          </div>
        </section>

        <section className="swarm-card swarm-activity-card">
          <div className="swarm-card-heading">
            <div>
              <div className="swarm-eyebrow">
                ACTIVITY
              </div>
              <h2>
                {serviceStatus.running
                  ? "Node status"
                  : "Node paused"}
              </h2>
            </div>

            <span
              className={`swarm-activity-indicator ${
                serviceStatus.running ? "active" : ""
              }`}
            />
          </div>

          <p className="swarm-activity-message">
            {serviceStatus.lastError ||
              serviceStatus.logs
                .slice()
                .reverse()
                .find((line) =>
                  line.startsWith("POLL_BLOCK_REASON="),
                )
                ?.replace("POLL_BLOCK_REASON=", "") ||
              (serviceStatus.running
                ? "Waiting for work"
                : "Start the node when you are ready.")}
          </p>
        </section>

        <details className="swarm-diagnostics">
          <summary>
            <span>Diagnostics</span>
            <span className="swarm-muted">
              Technical details
            </span>
          </summary>

          <section
            className="swarm-diagnostics-console"
            ref={consoleRef}
            onScroll={(event) => {
              const element = event.currentTarget;
              const distanceFromBottom =
                element.scrollHeight -
                element.scrollTop -
                element.clientHeight;

              shouldFollowConsoleRef.current =
                distanceFromBottom < 32;
            }}
          >
            <div>
              &gt; Swarm Provider Node v
              {appVersion || "..."}
            </div>

            {serviceStatus.logs.map((line, index) => (
              <div key={`${index}-${line}`}>
                &gt; {line}
              </div>
            ))}

            {serviceStatus.lastError && (
              <div>
                &gt; Error: {serviceStatus.lastError}
              </div>
            )}
          </section>
        </details>

        <footer className="swarm-footer">
          <span>{providerEmail}</span>
          <span>Swarm v{appVersion || "..."}</span>
        </footer>
      </div>
    </main>
  );
}

export default App;
