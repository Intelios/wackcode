import { useEffect, useState } from "react";
import type { SubscriptionLoginEvent } from "../types";

type PromptEvent = Extract<SubscriptionLoginEvent, { type: "prompt" }>;

interface Props {
  login: {
    loginId: string;
    providerId: string;
    prompt?: PromptEvent;
    authUrl?: string;
    deviceCode?: { userCode: string; verificationUri: string };
    message?: string;
    error?: string;
  };
  onOpenUrl: (url: string) => Promise<void>;
  onCopyCode: (code: string) => Promise<void>;
  onRespond: (promptId: string, value: string) => Promise<void>;
  onCancel: () => void;
}

export function SubscriptionLoginDialog({ login, onOpenUrl, onCopyCode, onRespond, onCancel }: Props) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [localError, setLocalError] = useState<string>();

  useEffect(() => {
    setValue(login.prompt?.prompt.type === "select" ? login.prompt.prompt.options?.[0]?.id ?? "" : "");
    setLocalError(undefined);
  }, [login.prompt?.promptId]);

  async function submit() {
    if (!login.prompt || (login.prompt.prompt.type !== "text" && !value.trim())) return;
    setBusy(true);
    setLocalError(undefined);
    try { await onRespond(login.prompt.promptId, value.trim()); }
    catch (reason) { setLocalError(String(reason)); }
    finally { setBusy(false); }
  }

  return (
    <div className="modal-backdrop" role="presentation">
      <div className="confirm-dialog subscription-login-dialog" role="dialog" aria-modal="true" aria-label="Subscription sign-in">
        <h2>Subscription sign-in</h2>
        <p>Follow Pi’s sign-in steps for {login.providerId}.</p>
        {login.message && <p role="status">{login.message}</p>}
        {login.authUrl && <button type="button" className="secondary-button" onClick={() => void onOpenUrl(login.authUrl!).catch((reason) => setLocalError(String(reason)))}>Open sign-in page</button>}
        {login.deviceCode && <div className="subscription-device-code">
          <span>Enter this code on the sign-in page</span>
          <strong>{login.deviceCode.userCode}</strong>
          <button type="button" className="secondary-button" onClick={() => void onCopyCode(login.deviceCode!.userCode).catch((reason) => setLocalError(String(reason)))}>Copy code</button>
        </div>}
        {login.prompt && <div className="subscription-prompt">
          <label htmlFor="subscription-prompt-value">{login.prompt.prompt.message}</label>
          {login.prompt.prompt.type === "select" ? (
            <select id="subscription-prompt-value" value={value} onChange={(event) => setValue(event.target.value)}>
              {login.prompt.prompt.options?.map((option) => <option key={option.id} value={option.id}>{option.label}</option>)}
            </select>
          ) : (
            <input id="subscription-prompt-value" type={login.prompt.prompt.type === "secret" ? "password" : "text"} value={value}
              onChange={(event) => setValue(event.target.value)} placeholder={login.prompt.prompt.placeholder}
              autoComplete="off" spellCheck={false} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); void submit(); } }} />
          )}
          <button type="button" className="primary-button" disabled={busy || (login.prompt.prompt.type !== "text" && !value.trim())} onClick={() => void submit()}>Continue</button>
        </div>}
        {(login.error || localError) && <div className="error-banner" role="alert">{localError || login.error}</div>}
        <div className="confirm-actions"><button type="button" className="secondary-button" onClick={onCancel}>{login.error ? "Close" : "Cancel sign-in"}</button></div>
      </div>
    </div>
  );
}
