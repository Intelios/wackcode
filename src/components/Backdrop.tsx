import { useEffect, useState, type CSSProperties } from "react";

interface BackdropProps {
  /** An `asset:` URL for the stored background image; nothing renders without one. */
  imageUrl?: string;
  /** The new-chat hero shows the image vividly; chats and Settings show it dimmed and blurred. */
  scene: "hero" | "chat";
  /** 0–90: how much of the background colour covers the image behind a chat. */
  dim: number;
  /** Blur behind a chat, in px. */
  blur: number;
}

/** The image behind the whole window (Settings › Appearance › Backdrop › Image). */
export function Backdrop({ imageUrl, scene, dim, blur }: BackdropProps) {
  // Fade in once decoded rather than popping in half-loaded.
  const [loaded, setLoaded] = useState<string>();
  useEffect(() => {
    if (!imageUrl) return;
    const image = new Image();
    image.onload = () => setLoaded(imageUrl);
    image.src = imageUrl;
    return () => { image.onload = null; };
  }, [imageUrl]);

  if (!imageUrl) return null;
  const style = {
    "--backdrop-image": `url("${imageUrl}")`,
    "--backdrop-dim": dim / 100,
    "--backdrop-blur": `${blur}px`
  } as CSSProperties;
  return (
    <div className={`backdrop ${loaded === imageUrl ? "ready" : ""}`} data-scene={scene} style={style} aria-hidden="true">
      <div className="backdrop-layer backdrop-hero" />
      <div className="backdrop-layer backdrop-chat" />
    </div>
  );
}
