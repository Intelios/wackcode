import { useContext, useEffect, useState } from "react";
import type { NormalizedBlock } from "../../types";
import type { CanvasBoard } from "../../canvas-state";
import { canvasProgress } from "../../canvas-progress";
import { formatRunDuration } from "../../chat-utils";
import { CanvasScopeContext } from "./CanvasCard";
import { QuillMark } from "../QuillMark";
import { Icon } from "../Icons";

/** Only received content advances this card. A quiet stream never masquerades as progress.
 * Draft markup stays inert until the tool accepts it; stopped/refused calls keep their tool row. */
export function CanvasProgress({ call, compact = false, board: targetBoard }: { call: NormalizedBlock; compact?: boolean; board?: CanvasBoard }) {
  const { boards, onOpen } = useContext(CanvasScopeContext);
  const progress = canvasProgress(call);
  const board = targetBoard ?? (progress.canvasId ? boards.get(progress.canvasId) : undefined);
  const title = progress.title || board?.title || "New canvas";
  const current = progress.artboards.at(-1);
  const name = current?.id ? board?.artboards.find((item) => item.id === current.id)?.name ?? current.name : current?.name;
  const [quietSeconds, setQuietSeconds] = useState(0);
  useEffect(() => {
    setQuietSeconds(0);
    const receivedAt = Date.now();
    const timer = window.setInterval(() => setQuietSeconds(Math.floor((Date.now() - receivedAt) / 1000)), 1000);
    return () => window.clearInterval(timer);
  }, [call.toolCallId, progress.signature]);
  const quiet = quietSeconds >= 15;
  const label = name ? `Writing ${name}` : progress.removed ? "Preparing canvas changes" : "Preparing artboards";
  return (
    <div className={`canvas-progress${compact ? " compact" : ""}${quiet ? " quiet" : ""}`}>
      <div className="canvas-progress-head">
        <QuillMark live={!quiet} />
        <div className="canvas-progress-heading" role="status">
          <span>Drawing canvas</span><strong title={title}>{title}</strong>
        </div>
        {board && onOpen && !compact && <button type="button" className="icon-button" aria-label={`Open canvas: ${title}`} title="Open canvas"
          onClick={() => onOpen(board.id, progress.artboards.flatMap((item) => item.id ? [item.id] : []))}><Icon name="external" /></button>}
      </div>
      <div className="canvas-progress-body">
        <span className="canvas-progress-step" role="status">{label}</span>
        <span className="canvas-progress-count" aria-live="off">{progress.characters.toLocaleString()} characters received{progress.artboards.length > 1 ? ` · ${progress.artboards.length} artboards` : ""}</span>
        <span className="canvas-progress-note" role="status">{quiet ? <>No new canvas content<span aria-hidden="true"> for {formatRunDuration(quietSeconds * 1000)}</span></> : "Artboards appear when this update is ready."}</span>
      </div>
    </div>
  );
}
