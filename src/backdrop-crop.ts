/**
 * The crop of the background image (Settings › Appearance › Backdrop › Position). The picture is
 * cover-fitted to the window, magnified by `zoom`, and `x` / `y` say which part of the overflow
 * shows: 0 is flush with the left / top edge, 1000 flush with the right / bottom, exactly CSS
 * `background-position` in thousandths. Because it is a fraction of the overflow, any value covers
 * any window shape, so a crop set in one window size still fills another. `styles.css`
 * (`.backdrop`, `.crop-frame`) draws the same maths in `cq` units; keep the two in step.
 */
export interface ImageCrop {
  /** 100–400 (%), over a cover fit. */
  zoom: number;
  /** 0–1000. */
  x: number;
  y: number;
}

export const MIN_ZOOM = 100;
export const MAX_ZOOM = 400;
export const CENTRED: ImageCrop = { zoom: MIN_ZOOM, x: 500, y: 500 };

export interface Frame {
  width: number;
  height: number;
}

const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

/** The size the whole picture is drawn at inside `frame`, given its width / height `aspect`. */
export function drawnSize(frame: Frame, aspect: number, zoom: number): Frame {
  const factor = zoom / 100;
  return {
    width: Math.max(frame.width, frame.height * aspect) * factor,
    height: Math.max(frame.height, frame.width / aspect) * factor
  };
}

export function isCentred(crop: ImageCrop): boolean {
  return crop.zoom === CENTRED.zoom && crop.x === CENTRED.x && crop.y === CENTRED.y;
}

/** Drags the picture by `dx` / `dy` screen pixels. An axis with no overflow stays put. */
export function pan(crop: ImageCrop, dx: number, dy: number, frame: Frame, aspect: number): ImageCrop {
  const size = drawnSize(frame, aspect, crop.zoom);
  const axis = (position: number, delta: number, overflow: number) =>
    overflow < 0.5 ? position : Math.round(clamp(position - (delta / overflow) * 1000, 0, 1000));
  return { ...crop, x: axis(crop.x, dx, size.width - frame.width), y: axis(crop.y, dy, size.height - frame.height) };
}

/** Changes the zoom while keeping whatever is at the middle of the frame at the middle. */
export function zoomTo(crop: ImageCrop, zoom: number, frame: Frame, aspect: number): ImageCrop {
  const next = clamp(Math.round(zoom), MIN_ZOOM, MAX_ZOOM);
  const before = drawnSize(frame, aspect, crop.zoom);
  const after = drawnSize(frame, aspect, next);
  const axis = (position: number, was: number, now: number, visible: number) => {
    const overflow = now - visible;
    if (overflow < 0.5) return 500;
    // The picture's fraction sitting at the middle of the frame, carried across the zoom.
    const middle = ((position / 1000) * (was - visible) + visible / 2) / was;
    return Math.round(clamp(((middle * now - visible / 2) / overflow) * 1000, 0, 1000));
  };
  return { zoom: next, x: axis(crop.x, before.width, after.width, frame.width), y: axis(crop.y, before.height, after.height, frame.height) };
}

/** Clamps whatever came from disk into range. */
export function sanitizeCrop(crop: ImageCrop): ImageCrop {
  return { zoom: clamp(crop.zoom, MIN_ZOOM, MAX_ZOOM), x: clamp(crop.x, 0, 1000), y: clamp(crop.y, 0, 1000) };
}

/** The custom properties `.backdrop` and `.crop-frame` read. */
export function cropVariables(crop: ImageCrop, aspect: number): Record<string, string | number> {
  return { "--backdrop-aspect": aspect, "--backdrop-zoom": crop.zoom / 100, "--backdrop-x": crop.x / 1000, "--backdrop-y": crop.y / 1000 };
}

/** Width over height of the image at `url`, once it has loaded. */
export function loadAspect(url: string, onLoad: (aspect: number) => void): () => void {
  const image = new Image();
  image.onload = () => onLoad(image.naturalWidth > 0 && image.naturalHeight > 0 ? image.naturalWidth / image.naturalHeight : 1.6);
  image.src = url;
  return () => { image.onload = null; };
}
