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
}

/** The image behind the whole window (Settings › Appearance › Backdrop › Image). */
export function Backdrop({ imageUrl, scene, dim, blur, crop }: BackdropProps) {
  // Fade in once decoded rather than popping in half-loaded. The shape is needed to place it.
  const [loaded, setLoaded] = useState<{ url: string; aspect: number }>();
  useEffect(() => {
    if (!imageUrl) return;
    return loadAspect(imageUrl, (aspect) => setLoaded({ url: imageUrl, aspect }));
  }, [imageUrl]);

  if (!imageUrl) return null;
  const ready = loaded?.url === imageUrl;
  const style = {
    ...cropVariables(sanitizeCrop(crop), loaded?.aspect ?? 1.6),
    "--backdrop-image": `url("${imageUrl}")`,
    "--backdrop-dim": dim / 100,
    "--backdrop-blur": `${blur}px`
  } as CSSProperties;
  return (
    <div className={`backdrop ${ready ? "ready" : ""}`} data-scene={scene} style={style} aria-hidden="true">
      <div className="backdrop-layer backdrop-hero" />
      <div className="backdrop-layer backdrop-chat" />
    </div>
  );
}
