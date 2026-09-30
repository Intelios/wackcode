import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { motion, useReducedMotion } from "motion/react";
import { Icon } from "../Icons";

interface Props {
  /** Shown at once (a small preview), and kept if the original can't be loaded. */
  preview: string;
  /** Resolves to the full-size image's URL, or undefined when only the preview exists. */
  load?: () => Promise<string | undefined>;
  alt: string;
  onClose: () => void;
}

/**
 * A screenshot at full size over the app. It grows out of the preview while the original loads,
 * then sharpens into it. Escape or a click outside closes it.
 */
export function ImageLightbox({ preview, load, alt, onClose }: Props) {
  const reduce = useReducedMotion();
  const [full, setFull] = useState<string>();

  useEffect(() => {
    let live = true;
    void load?.().then((url) => { if (live && url) setFull(url); }).catch(() => undefined);
    return () => { live = false; };
  }, [load]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return createPortal(
    <motion.div
      className="modal-backdrop image-lightbox"
      role="presentation"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: reduce ? 0 : 0.18 }}
      onMouseDown={(event) => event.button === 0 && event.target === event.currentTarget && onClose()}
    >
      <motion.figure
        className="image-lightbox-figure"
        role="dialog"
        aria-modal="true"
        aria-label={alt}
        initial={reduce ? false : { scale: 0.92, opacity: 0, y: 8 }}
        animate={{ scale: 1, opacity: 1, y: 0 }}
        transition={{ type: "spring", stiffness: 420, damping: 32 }}
      >
        <img className={full ? "loaded" : "preview"} src={full ?? preview} alt={alt} />
        <button type="button" className="image-lightbox-close" aria-label="Close" onClick={onClose} autoFocus>
          <Icon name="close" />
        </button>
      </motion.figure>
    </motion.div>,
    document.body
  );
}
