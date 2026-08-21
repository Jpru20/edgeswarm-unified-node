import { FormEvent, useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

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

type NodeServiceStatus = {
  running: boolean;
  stopping: boolean;
  lastError?: string | null;
  logs: string[];
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
      stopping: false,
      lastError: null,
      logs: [],
    });
  const [nodeActionBusy, setNodeActionBusy] = useState(false);

  // PROVIDER_LEDGER_UI_V1
  const [totalEarnedUsd, setTotalEarnedUsd] = useState(0);
  const [ledgerBusy, setLedgerBusy] = useState(false);
  const ledgerSyncInFlightRef = useRef(false);
  const lastLedgerTriggerRef = useRef("");
  const consoleRef = useRef<HTMLElement | null>(null);

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
      const command = serviceStatus.running
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
    if (consoleRef.current) {
      consoleRef.current.scrollTop = consoleRef.current.scrollHeight;
    }
  }, [serviceStatus.logs, serviceStatus.lastError]);

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
          <h1>EdgeSwarm Authentication</h1>

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

  const hardwareId =
    nodeState?.hardwareIdentity?.hardwareId ?? "unavailable";

  const hardwareShort =
    hardwareId.length >= 8 ? hardwareId.slice(0, 8) : hardwareId;

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
    <main className="app dashboard">
      <h1>Edge Swarm Provider Node</h1>

      <div className="attestation">Hardware Attestation: Valid [{hardwareShort}]</div>

      <div className="account">
        Logged in as: {providerEmail}
      </div>

      <div className="version">Node Version: v1.5.15</div>

      <section className="model-panel">
        <div className="panel-heading">Device Capability Profile</div>

        <div className="model-name">
          {visibleModel?.selectedModel ?? "Awaiting capability profile"}
        </div>

        <div className={readyModel ? "model-status ready" : "model-status"}>
          {capabilityText}
        </div>

        {/* MODEL_DOWNLOAD_PROGRESS_UI_V1 */}
        {visibleModel && (visibleModel.status ?? "").toLowerCase().includes("download") && (
          <>
            <div className="progress-track">
              <div className="progress-fill download-indeterminate" />
            </div>
            <div className="progress-label">Downloading model...</div>
          </>
        )}
      </section>

      <div className="earnings">{earningsText}</div>
      <div className="earnings-label">Total Earnings</div>

      <button
        className={`start-button ${
          serviceStatus.running ? "running" : ""
        }`}
        type="button"
        onClick={toggleNode}
        disabled={nodeActionBusy || serviceStatus.stopping}
      >
        {serviceStatus.stopping
          ? "STOPPING..."
          : serviceStatus.running
            ? "STOP NODE"
            : "START NODE"}
      </button>

      <section className="console" ref={consoleRef}>
        <div>&gt; System Initialized. Identity Verified via Supabase Auth.</div>
        <div>&gt; Running Edge Swarm Provider Node v1.5.15</div>
        <div>
          &gt; Hardware: {cpuName} | RAM: {ramGb ?? "Unknown"}GB | GPU: {gpuName}
        </div>

        {serviceStatus.logs.map((line, index) => (
          <div key={`${index}-${line}`}>
            &gt; {line}
          </div>
        ))}

        {serviceStatus.lastError && (
          <div>
            &gt; Node service error: {serviceStatus.lastError}
          </div>
        )}
      </section>

      <button
        className="ledger-button"
        type="button"
        onClick={() => void syncLedger()}
        disabled={ledgerBusy}
      >
        SYNC LEDGER DATA
      </button>
    </main>
  );
}

export default App;
