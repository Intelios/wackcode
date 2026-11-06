/**
 * Local backdrop analysis. Read source pixels at native resolution in small tiles, retaining
 * channel extrema rather than downsampling away bright details. The bounded atlas is reused
 * for every crop/resize; no screen capture, fetch or continuous frame loop is involved.
 * Images above the pixel budget use the conservative unknown-image fallback instead.
 */
import { drawnSize, type Frame, type ImageCrop } from "./backdrop-crop";
import { parseHex } from "./theme";
import type { ReadabilitySample } from "./image-readability";

export const MAX_ANALYSIS_PIXELS = 16_777_216;
/** Very thin panoramas must not turn a small source into thousands of scheduled tile reads. */
export const MAX_ANALYSIS_TILES = 1024;
const CELL = 32;
const TILE = 256;
const CHANNELS = 8; // min RGB, max RGB, min/max alpha (0–255).

export interface ImageBounds {
  width: number;
  height: number;
  columns: number;
  rows: number;
  cells: Uint8ClampedArray;
}

export interface BackdropRegion extends Frame {
  left: number;
  top: number;
}

/** A decoded image is read in bounded tiles, yielding between them so the UI stays usable. */
export async function analyseImage(image: HTMLImageElement, cancelled: () => boolean): Promise<ImageBounds | undefined> {
  const width = image.naturalWidth, height = image.naturalHeight;
  if (!(width > 0 && height > 0) || width * height > MAX_ANALYSIS_PIXELS
    || Math.ceil(width / TILE) * Math.ceil(height / TILE) > MAX_ANALYSIS_TILES) return undefined;
  const columns = Math.ceil(width / CELL), rows = Math.ceil(height / CELL);
  const cells = new Uint8ClampedArray(columns * rows * CHANNELS);
  for (let cell = 0; cell < columns * rows; cell++) {
    const at = cell * CHANNELS;
    cells.fill(255, at, at + 3);
    cells[at + 6] = 255;
  }
  const canvas = document.createElement("canvas");
  canvas.width = TILE;
  canvas.height = TILE;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) return undefined;
  try {
    for (let top = 0; top < height; top += TILE) {
      for (let left = 0; left < width; left += TILE) {
        if (cancelled()) return undefined;
        const w = Math.min(TILE, width - left), h = Math.min(TILE, height - top);
        context.clearRect(0, 0, TILE, TILE);
        context.drawImage(image, left, top, w, h, 0, 0, w, h);
        const pixels = context.getImageData(0, 0, w, h).data;
        for (let y = 0; y < h; y++) {
          for (let x = 0; x < w; x++) {
            const at = (Math.floor((top + y) / CELL) * columns + Math.floor((left + x) / CELL)) * CHANNELS;
            const pixel = (y * w + x) * 4;
            for (let channel = 0; channel < 3; channel++) {
              cells[at + channel] = Math.min(cells[at + channel], pixels[pixel + channel]);
              cells[at + 3 + channel] = Math.max(cells[at + 3 + channel], pixels[pixel + channel]);
            }
            cells[at + 6] = Math.min(cells[at + 6], pixels[pixel + 3]);
            cells[at + 7] = Math.max(cells[at + 7], pixels[pixel + 3]);
          }
        }
        await new Promise<void>((resolve) => setTimeout(resolve, 0));
      }
    }
    return cancelled() ? undefined : { width, height, columns, rows, cells };
  } finally {
    canvas.width = 0;
    canvas.height = 0;
  }
}

/**
 * Match .backdrop-chat::before: cover fit, crop, then a centred 1.08 enlargement. Include a
 * four-sigma blur halo and adjacent source cells. Combining channel bounds is deliberately
 * conservative for blur/interpolation between differently coloured pixels; it never relies
 * on Canvas2D.filter support or averages away a bright patch.
 */
export function backdropSamples(bounds: ImageBounds, frame: Frame, region: BackdropRegion, crop: ImageCrop, blur: number, background: string): ReadabilitySample[] {
  const bg = parseHex(background);
  if (!bg || frame.width <= 0 || frame.height <= 0) return [{ min: [0, 0, 0], max: [255, 255, 255] }];
  const size = drawnSize(frame, bounds.width / bounds.height, crop.zoom);
  const scale = 1.08;
  const startX = frame.width * (1 - scale) / 2 + (frame.width - size.width) * crop.x / 1000 * scale;
  const startY = frame.height * (1 - scale) / 2 + (frame.height - size.height) * crop.y / 1000 * scale;
  const pixelScale = size.width * scale / bounds.width;
  const halo = Math.max(0, blur) * scale * 4 + pixelScale;
  const column = (x: number) => Math.floor((x - startX) / pixelScale / CELL);
  const row = (y: number) => Math.floor((y - startY) / pixelScale / CELL);
  const x0 = Math.max(0, Math.min(bounds.columns - 1, column(region.left - halo)));
  const x1 = Math.max(0, Math.min(bounds.columns - 1, column(region.left + region.width + halo)));
  const y0 = Math.max(0, Math.min(bounds.rows - 1, row(region.top - halo)));
  const y1 = Math.max(0, Math.min(bounds.rows - 1, row(region.top + region.height + halo)));
  const sample: ReadabilitySample = { min: [255, 255, 255], max: [0, 0, 0] };
  for (let y = y0; y <= y1; y++) {
    for (let x = x0; x <= x1; x++) {
      const at = (y * bounds.columns + x) * CHANNELS;
      const lowAlpha = bounds.cells[at + 6] / 255, highAlpha = bounds.cells[at + 7] / 255;
      for (let channel = 0; channel < 3; channel++) {
        const low = bounds.cells[at + channel], high = bounds.cells[at + 3 + channel];
        const values = [lowAlpha * low + (1 - lowAlpha) * bg[channel], highAlpha * low + (1 - highAlpha) * bg[channel],
          lowAlpha * high + (1 - lowAlpha) * bg[channel], highAlpha * high + (1 - highAlpha) * bg[channel]];
        sample.min[channel] = Math.min(sample.min[channel], ...values);
        sample.max[channel] = Math.max(sample.max[channel], ...values);
      }
    }
  }
  // CSS blurs the finite window-sized pseudo-element before enlarging it. Its outside is
  // transparent, even when the cover-fitted source image extends further than that box.
  // Near that edge include the underlying solid colour in our blur bounds as well.
  const edgeX = frame.width * (scale - 1) / 2, edgeY = frame.height * (scale - 1) / 2;
  if (blur > 0 && (region.left - halo < -edgeX || region.top - halo < -edgeY
    || region.left + region.width + halo > frame.width + edgeX
    || region.top + region.height + halo > frame.height + edgeY)) {
    for (let channel = 0; channel < 3; channel++) {
      sample.min[channel] = Math.min(sample.min[channel], bg[channel]);
      sample.max[channel] = Math.max(sample.max[channel], bg[channel]);
    }
  }
  return [sample];
}
