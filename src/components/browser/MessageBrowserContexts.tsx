import type { BrowserContextRecord, NormalizedBlock } from "../../types";
import { BrowserContextChip } from "./BrowserContextChip";

export function MessageBrowserContexts({ contexts, images, loadImage, onRemove }: {
  contexts: BrowserContextRecord[]; images: NormalizedBlock[];
  loadImage?: (index: number) => Promise<string | undefined>;
  onRemove?: (context: BrowserContextRecord) => void;
}) {
  if (!contexts.length) return null;
  return <div className="message-browser-contexts">{contexts.map((context) => <BrowserContextChip key={context.id} context={context}
    preview={images[context.imageIndex]?.thumbnail} load={loadImage ? () => loadImage(context.imageIndex) : undefined}
    onRemove={onRemove ? () => onRemove(context) : undefined} />)}</div>;
}
