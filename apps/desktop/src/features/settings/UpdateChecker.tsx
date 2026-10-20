import { useEffect, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { nativeDesktop } from "../../lib/surface";

/**
 * Startup update check (auto-update, P1 follow-up). A failed check is
 * SILENT by design — an offline machine or an unreachable endpoint must
 * look exactly like "no news"; only a verified update announcement may
 * interrupt. Download progress is honest: bytes, not percent guesses.
 */
export function UpdateChecker() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [phase, setPhase] = useState<"idle" | "downloading" | "installing" | "failed">("idle");
  const [progress, setProgress] = useState("");
  const [error, setError] = useState("");
  const acting = useRef(false);

  useEffect(() => {
    if (!nativeDesktop) return;
    let disposed = false;
    // A few seconds after boot: never race startup, never block the panel.
    const timer = setTimeout(() => {
      check()
        .then((found) => {
          if (!disposed && found) setUpdate(found);
        })
        .catch(() => {});
    }, 4000);
    return () => {
      disposed = true;
      clearTimeout(timer);
    };
  }, []);

  async function install() {
    if (acting.current || !update) return;
    acting.current = true;
    setError("");
    setPhase("downloading");
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started" && event.data.contentLength)
          setProgress(`0 / ${Math.round(event.data.contentLength / 1048576)} MB`);
        if (event.event === "Progress")
          setProgress(
            `${Math.round(event.data.chunkLength / 1024)} KB…（持续下载中）`,
          );
        if (event.event === "Finished") setProgress("下载完成");
      });
      setPhase("installing");
      await relaunch();
    } catch (e) {
      setPhase("failed");
      setError(String(e));
    } finally {
      acting.current = false;
    }
  }

  if (!update) return null;
  return (
    <section className="model-settings" aria-label="版本更新">
      <h2>发现新版本</h2>
      <p>
        当前可更新到 <strong>v{update.version}</strong>。更新包经签名校验后安装，
        安装完成会自动重启栖伴；正在进行的对话请先结束。
      </p>
      {phase === "downloading" && (
        <p role="status">
          正在下载更新… {progress}
        </p>
      )}
      {phase === "installing" && <p role="status">正在安装，即将重启…</p>}
      {error && (
        <p role="alert">
          更新失败：{error}。可稍后在设置里重试，或到 GitHub Releases 手动下载。
        </p>
      )}
      <div className="account-actions">
        <button
          className="primary"
          disabled={phase === "downloading" || phase === "installing"}
          onClick={() => void install()}
        >
          {phase === "failed" ? "重试更新" : "下载并安装"}
        </button>
        <button type="button" onClick={() => setUpdate(null)}>
          稍后再说
        </button>
      </div>
      <p className="workspace-hint">
        检查与下载需要能访问 GitHub；网络不通时下次启动会再试。
      </p>
    </section>
  );
}
