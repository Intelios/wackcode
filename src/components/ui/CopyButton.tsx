import { createContext, useContext, useEffect, useState } from "react";
import { Icon } from "../Icons";

/** App supplies the native clipboard writer to every transcript, including side panels. */
export const CopyText = createContext<((text: string) => Promise<void>) | undefined>(undefined);

/** Copies source text, never the highlighted or shortened DOM representation. */
export function CopyButton({ text, label }: { text: string; label: string }) {
  const copy = useContext(CopyText);
  const [status, setStatus] = useState<"idle" | "copying" | "copied" | "failed">("idle");
  useEffect(() => {
    if (status !== "copied" && status !== "failed") return;
    const timer = setTimeout(() => setStatus("idle"), 2000);
    return () => clearTimeout(timer);
  }, [status]);

  async function copyText() {
    if (!copy) return;
    setStatus("copying");
    try {
      await copy(text);
      setStatus("copied");
    } catch {
      setStatus("failed");
    }
  }

  return (
    <span className="copy-control">
      <button type="button" className="secondary-button compact copy-button" aria-label={label}
        disabled={!copy || status === "copying"} onClick={() => void copyText()}>
        <Icon name={status === "copied" ? "check" : "copy"} />
        {status === "copied" ? "Copied" : status === "failed" ? "Try again" : "Copy"}
      </button>
      <span className="copy-feedback" role="status">
        {status === "copied" ? "Copied to clipboard." : status === "failed" ? "Couldn’t copy. Try again." : ""}
      </span>
    </span>
  );
}
