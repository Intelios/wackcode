import { useEffect, useState } from "react";
import type { GitChangeFile, GitChanges } from "../types";
import { Icon } from "./Icons";

interface Props {
  changes?: GitChanges;
  loading: boolean;
  width: number;
  onWidthChange: (width: number) => void;
  onClose: () => void;
  onRefresh: () => void;
}

function Diff({ file }: { file: GitChangeFile }) {
  return (
    <pre className="diff-view" aria-label={`Diff for ${file.path}`}>
      {file.diff.split("\n").map((line, index) => {
        const kind = line.startsWith("+") && !line.startsWith("+++") ? "addition" : line.startsWith("-") && !line.startsWith("---") ? "deletion" : line.startsWith("@@") ? "hunk" : line.startsWith("diff ") || line.startsWith("# ") ? "heading" : "";
        return <span className={kind} key={index}>{line || " "}{"\n"}</span>;
      })}
    </pre>
  );
}

export function ChangesPanel({ changes, loading, width, onWidthChange, onClose, onRefresh }: Props) {
  const [selectedPath, setSelectedPath] = useState<string>();
  const files = changes?.files ?? [];
  const selected = files.find((file) => file.path === selectedPath) ?? files[0];

  useEffect(() => {
    if (selectedPath && !files.some((file) => file.path === selectedPath)) setSelectedPath(undefined);
  }, [files, selectedPath]);

  function startResize(event: React.PointerEvent) {
    const startX = event.clientX;
    const startWidth = width;
    const move = (moveEvent: PointerEvent) => onWidthChange(Math.max(290, Math.min(720, startWidth + startX - moveEvent.clientX)));
    const end = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
  }

  return (
    <aside className="changes-panel" style={{ width }}>
      <div className="panel-resizer" onPointerDown={startResize} />
      <header className="changes-header">
        <div><h3>Changes <span>{files.length}</span></h3></div>
        <div><button className="icon-button" onClick={onRefresh} aria-label="Refresh changes"><Icon name="refresh" className={loading ? "spinning" : ""} /></button><button className="icon-button" onClick={onClose} aria-label="Close changes panel">×</button></div>
      </header>
      {!changes?.isGit ? (
        <div className="panel-empty"><Icon name="git" /><strong>No Git repository</strong><span>Chat and editing still work. Changes can’t be summarized here.</span></div>
      ) : files.length === 0 ? (
        <div className="panel-empty"><span className="clean-check">✓</span><strong>Working tree clean</strong><span>No staged, unstaged, or untracked files.</span></div>
      ) : (
        <>
          <div className="changed-files">
            {files.map((file) => <button key={file.path} className={selected?.path === file.path ? "active" : ""} onClick={() => setSelectedPath(file.path)} title={file.path}><span className={`status-letter ${file.status}`}>{file.status[0]?.toUpperCase()}</span><span>{file.path}</span>{file.staged && <em>S</em>}</button>)}
          </div>
          <div className="diff-file-header"><span title={selected?.path}>{selected?.path}</span>{selected?.binary && <em>binary</em>}{selected?.truncated && <em>truncated</em>}</div>
          {selected && <Diff file={selected} />}
        </>
      )}
    </aside>
  );
}
