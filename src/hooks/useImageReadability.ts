/**
 * One stable foreground/veil for the main conversation. Only image/settings/viewport changes
 * analyse pixels; scrolling, token streaming and transcript height do not. App supplies the
 * native motion check so this cosmetic hook never owns a Tauri bridge or event subscription.
 */
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";
import { analyseImage, backdropSamples, type BackdropRegion, type ImageBounds } from "../image-analysis";
import { FALLBACK_READABILITY_SAMPLES, resolveImageReadability } from "../image-readability";
import { resolveTheme } from "../theme";
import type { Frame, ImageCrop } from "../backdrop-crop";
import type { BackgroundImageMotion } from "../types";

interface Geometry {
  frame: Frame;
  region: BackdropRegion;
}

/** Offset geometry ignores Framer's entrance transforms, which do not notify ResizeObserver. */
function layoutPosition(element: HTMLElement) {
  let left = 0, top = 0;
  for (let current: HTMLElement | null = element; current; current = current.offsetParent as HTMLElement | null) {
    left += current.offsetLeft;
    top += current.offsetTop;
  }
  return { left, top };
}

interface ImageReadabilityInput {
  root: RefObject<HTMLDivElement | null>;
  enabled: boolean;
  imageUrl?: string;
  imageName?: string;
  inspectMotion: (name: string) => Promise<BackgroundImageMotion>;
  crop: ImageCrop;
  dim: number;
  blur: number;
  accent?: string | null;
  background?: string | null;
  /** Identifies the selected conversation/area, never a scroll or streaming revision. */
  conversationKey: string;
  area: "code" | "chat";
  visual: boolean;
  warning: boolean;
}

interface AnalysedImage {
  url: string;
  bounds?: ImageBounds;
  aspect?: number;
}

export function useImageReadability(input: ImageReadabilityInput) {
  const { root, enabled, imageUrl, imageName, inspectMotion, conversationKey, area, dim, blur, accent, background, visual, warning } = input;
  const { zoom, x, y } = input.crop;
  const [source, setSource] = useState<AnalysedImage>();
  const cached = useRef<AnalysedImage>(undefined);
  const [geometry, setGeometry] = useState<Geometry>();
  const previous = useRef<{ key: string; polarity: "light" | "dark" }>(undefined);

  useEffect(() => {
    if (!enabled || !imageUrl || !imageName) return;
    if (cached.current?.url === imageUrl) {
      setSource(cached.current);
      return;
    }
    let disposed = false;
    const image = new Image();
    // Asset responses already allow the app origin; fetch would need broader connect-src.
    image.crossOrigin = "anonymous";
    const decoded = new Promise<boolean>((resolve) => {
      image.onload = () => resolve(true);
      image.onerror = () => resolve(false);
    });
    image.src = imageUrl;
    void Promise.all([decoded, inspectMotion(imageName).catch(() => "unknown" as const)])
      .then(async ([loaded, motion]) => {
        const bounds = loaded && motion === "static" && !disposed ? await analyseImage(image, () => disposed) : undefined;
        if (disposed) return;
        const result = { url: imageUrl, bounds, aspect: loaded && image.naturalWidth > 0 && image.naturalHeight > 0 ? image.naturalWidth / image.naturalHeight : undefined };
        cached.current = result;
        setSource(result);
      }).catch(() => {
        if (!disposed) {
          const result = { url: imageUrl };
          cached.current = result;
          setSource(result);
        }
      });
    return () => {
      disposed = true;
      image.onload = null;
      image.onerror = null;
      image.src = "";
    };
  }, [enabled, imageUrl, imageName, inspectMotion]);

  useLayoutEffect(() => {
    if (!enabled || !root.current) return;
    const shell = root.current;
    const workspace = shell.querySelector<HTMLElement>(".workspace");
    const selector = area === "chat" ? ".chat-area .chat-scroll" : ".chat-view .conversation-scroll";
    const viewport = workspace?.querySelector<HTMLElement>(selector);
    let frame = 0;
    const measure = () => {
      const outer = shell.getBoundingClientRect();
      const measured = viewport ?? workspace ?? shell;
      const inner = measured.getBoundingClientRect();
      if (outer.width <= 0 || outer.height <= 0) return;
      const shellPosition = layoutPosition(shell), viewportPosition = layoutPosition(measured);
      // Include both the stable layout and its current entrance excursion. Text never leaves
      // the analysed region as its transform settles, without a per-animation frame loop.
      const layoutLeft = measured.offsetWidth > 0 ? viewportPosition.left - shellPosition.left : inner.left - outer.left;
      const layoutTop = measured.offsetHeight > 0 ? viewportPosition.top - shellPosition.top : inner.top - outer.top;
      const left = Math.min(layoutLeft, inner.left - outer.left), top = Math.min(layoutTop, inner.top - outer.top);
      const right = Math.max(layoutLeft + (measured.offsetWidth || inner.width), inner.right - outer.left);
      const bottom = Math.max(layoutTop + (measured.offsetHeight || inner.height), inner.bottom - outer.top);
      const next: Geometry = {
        frame: { width: Math.round(outer.width), height: Math.round(outer.height) },
        region: { left: Math.floor(left), top: Math.floor(top), width: Math.ceil(right - left), height: Math.ceil(bottom - top) }
      };
      setGeometry((current) => JSON.stringify(current) === JSON.stringify(next) ? current : next);
    };
    const schedule = () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(measure); };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(schedule);
    observer?.observe(shell);
    if (workspace) observer?.observe(workspace);
    if (viewport) observer?.observe(viewport);
    window.addEventListener("resize", schedule);
    return () => { observer?.disconnect(); cancelAnimationFrame(frame); window.removeEventListener("resize", schedule); };
  }, [enabled, root, conversationKey, area]);

  const theme = useMemo(() => resolveTheme({ accent, background }), [accent, background]);
  const current = source?.url === imageUrl ? source : undefined;
  const samples = useMemo(() => current?.bounds && geometry
    ? backdropSamples(current.bounds, geometry.frame, visual ? { left: 0, top: 0, ...geometry.frame } : geometry.region, { zoom, x, y }, blur, theme.background)
    : FALLBACK_READABILITY_SAMPLES,
  [current, geometry, zoom, x, y, blur, theme.background, visual]);
  const result = useMemo(() => {
    if (!enabled) return undefined;
    const fixedText: { colour: string; ratio: number }[] = [];
    if (visual) {
      for (const name of ["--text", "--text-soft", "--wc-accent-text"]) fixedText.push({ colour: theme.variables[name], ratio: 4.5 });
      fixedText.push({ colour: theme.variables["--text-dim"], ratio: 3.8 });
    }
    // Stopped labels are semantic warning ink in both areas, fixed in :root.
    if (warning) fixedText.push({ colour: getComputedStyle(document.documentElement).getPropertyValue("--warning").trim() || "#e6ba69", ratio: 4.5 });
    const key = `${imageUrl}:${conversationKey}`;
    return resolveImageReadability({ samples, background: theme.background, accent: theme.accent, dim,
      forceLight: !current?.bounds || visual, fixedText,
      previous: previous.current?.key === key ? previous.current.polarity : undefined });
  }, [enabled, samples, theme, dim, visual, warning, current, imageUrl, conversationKey]);

  useLayoutEffect(() => {
    if (result && current) previous.current = { key: `${imageUrl}:${conversationKey}`, polarity: result.polarity };
  }, [result, current, imageUrl, conversationKey]);

  return { result, ready: !enabled || Boolean(current), aspect: current?.aspect };
}
