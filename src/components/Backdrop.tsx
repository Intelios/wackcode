import { useEffect, useState, type CSSProperties } from "react";
import { cropVariables, loadAspect, sanitizeCrop, type ImageCrop } from "../backdrop-crop";

interface BackdropProps {
  /** An `asset:` URL for the stored background image; nothing renders without one. */
  imageUrl?: string;
  /** The new-chat hero shows the image vividly; chats and Settings show it dimmed and blurred. */
  scene: "hero" | "chat";
  /** 0–90: how much of the background colour covers the image behind a chat. */
  dim: number;
  /** Blur behind a chat, in px. */
  blur: number;
  /** Which part of the image shows, and how magnified. */
  crop: ImageCrop;
  /** Automatic protection is committed with its matching foreground, never mid-crossfade. */
  extraDim?: number;
  readabilityReady?: boolean;
  analysedAspect?: number;
}

/** The image behind the whole window (Settings › Appearance › Backdrop › Image). */
export function Backdrop({ imageUrl, scene, dim, blur, crop, extraDim = 0, readabilityReady = true, analysedAspect }: BackdropProps) {
  // Fade in once decoded rather than popping in half-loaded. The shape is needed to place it.
  const [loaded, setLoaded] = useState<{ url: string; aspect: number }>();
  useEffect(() => {
    if (!imageUrl) return;
    return loadAspect(imageUrl, (aspect) => setLoaded({ url: imageUrl, aspect }));
  }, [imageUrl]);

  if (!imageUrl) return null;
  const ready = readabilityReady && (analysedAspect !== undefined || loaded?.url === imageUrl);
  const style = {
    ...cropVariables(sanitizeCrop(crop), analysedAspect ?? (loaded?.url === imageUrl ? loaded.aspect : 1.6)),
    "--backdrop-image": `url("${imageUrl}")`,
    "--backdrop-dim": dim / 100,
    "--backdrop-blur": `${blur}px`,
    "--backdrop-extra-dim": extraDim
  } as CSSProperties;
  return (
    <div className={`backdrop ${ready ? "ready" : ""}`} data-scene={scene} data-auto-readability={analysedAspect !== undefined || !readabilityReady ? "true" : undefined} style={style} aria-hidden="true">
      <div className="backdrop-layer backdrop-hero" />
      <div className="backdrop-layer backdrop-chat" />
      <div className="backdrop-contrast" />
    </div>
  );
}
