import { afterEach, describe, expect, it, vi } from "vitest";
import { CENTRED, type ImageCrop } from "./backdrop-crop";
import { analyseImage, backdropSamples, MAX_ANALYSIS_PIXELS, MAX_ANALYSIS_TILES, type ImageBounds } from "./image-analysis";

type Pixel = [number, number, number, number];

function decodedImage(width: number, height: number): HTMLImageElement {
  const image = new Image();
  Object.defineProperties(image, { naturalWidth: { value: width }, naturalHeight: { value: height } });
  return image;
}

/** Emulate a native-resolution tile read, retaining the draw geometry for budget assertions. */
function canvasPixels(pixelAt: (x: number, y: number) => Pixel) {
  let sourceLeft = 0;
  let sourceTop = 0;
  let canvas: HTMLCanvasElement | undefined;
  const context = {
    clearRect: vi.fn(),
    drawImage: vi.fn((_image: CanvasImageSource, left: number, top: number) => {
      sourceLeft = left;
      sourceTop = top;
    }),
    getImageData: vi.fn((_left: number, _top: number, width: number, height: number) => {
      const data = new Uint8ClampedArray(width * height * 4);
      for (let y = 0; y < height; y++) {
        for (let x = 0; x < width; x++) data.set(pixelAt(sourceLeft + x, sourceTop + y), (y * width + x) * 4);
      }
      return { data } as ImageData;
    })
  };
  const getContext = vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(function(this: HTMLCanvasElement) {
    canvas = this;
    return context as unknown as CanvasRenderingContext2D;
  });
  return { context, getContext, canvas: () => canvas };
}

function cellBounds(width = 320, height = 320, colour: (x: number, y: number) => Pixel = (x, y) => [x * 10, y * 10, 0, 255]): ImageBounds {
  const columns = Math.ceil(width / 32), rows = Math.ceil(height / 32);
  const cells = new Uint8ClampedArray(columns * rows * 8);
  for (let y = 0; y < rows; y++) {
    for (let x = 0; x < columns; x++) {
      const [r, g, b, a] = colour(x, y);
      cells.set([r, g, b, r, g, b, a, a], (y * columns + x) * 8);
    }
  }
  return { width, height, columns, rows, cells };
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("native image analysis", () => {
  it("keeps isolated bright pixels, including details at tile and image edges", async () => {
    vi.useFakeTimers();
    const { context, canvas } = canvasPixels((x, y) => (x === 255 && y === 31) || (x === 512 && y === 64) ? [255, 255, 255, 255] : [3, 5, 7, 255]);
    const pending = analyseImage(decodedImage(513, 65), () => false);
    await vi.runAllTimersAsync();
    const bounds = (await pending)!;
    expect(bounds).toMatchObject({ width: 513, height: 65, columns: 17, rows: 3 });
    // One white pixel amongst 1,023 dark pixels cannot be averaged out.
    expect([...bounds.cells.slice(7 * 8, 8 * 8)]).toEqual([3, 5, 7, 255, 255, 255, 255, 255]);
    const finalCell = (2 * 17 + 16) * 8;
    expect([...bounds.cells.slice(finalCell, finalCell + 8)]).toEqual([255, 255, 255, 255, 255, 255, 255, 255]);
    expect(context.drawImage).toHaveBeenCalledTimes(3);
    expect(canvas()).toMatchObject({ width: 0, height: 0 });
  });

  it("bounds each pixel read and never rescales a source tile", async () => {
    vi.useFakeTimers();
    const image = decodedImage(513, 257);
    const { context, getContext } = canvasPixels(() => [30, 40, 50, 255]);
    const pending = analyseImage(image, () => false);
    await vi.runAllTimersAsync();
    expect(await pending).toBeDefined();
    expect(getContext).toHaveBeenCalledWith("2d", { willReadFrequently: true });
    expect(context.drawImage).toHaveBeenCalledTimes(6);
    for (const args of context.drawImage.mock.calls as unknown as [HTMLImageElement, number, number, number, number, number, number, number, number][]) {
      const [source, , , width, height, left, top, targetWidth, targetHeight] = args;
      expect(source).toBe(image);
      expect(width).toBeGreaterThan(0);
      expect(height).toBeGreaterThan(0);
      expect(Math.max(width, height)).toBeLessThanOrEqual(256);
      expect([left, top, targetWidth, targetHeight]).toEqual([0, 0, width, height]);
    }
    expect(context.drawImage).toHaveBeenLastCalledWith(image, 512, 256, 1, 1, 0, 0, 1, 1);
  });

  it("uses the conservative fallback above the source pixel budget or before dimensions exist", async () => {
    const { getContext } = canvasPixels(() => [0, 0, 0, 255]);
    expect(MAX_ANALYSIS_PIXELS).toBe(4096 * 4096);
    expect(await analyseImage(decodedImage(4097, 4096), () => false)).toBeUndefined();
    expect(await analyseImage(decodedImage(0, 100), () => false)).toBeUndefined();
    expect(getContext).not.toHaveBeenCalled();
  });

  it("bounds scheduled work for extreme panoramas even when their pixel count is small", async () => {
    const { getContext, context } = canvasPixels(() => [0, 0, 0, 255]);
    const longSide = 256 * MAX_ANALYSIS_TILES;
    expect(longSide + 1).toBeLessThan(MAX_ANALYSIS_PIXELS);
    expect(await analyseImage(decodedImage(longSide + 1, 1), () => false)).toBeUndefined();
    expect(await analyseImage(decodedImage(1, longSide + 1), () => false)).toBeUndefined();
    expect(getContext).not.toHaveBeenCalled();
    // The limit itself is permitted; cancel before the first tile to avoid unnecessary work.
    expect(await analyseImage(decodedImage(longSide, 1), () => true)).toBeUndefined();
    expect(getContext).toHaveBeenCalledTimes(1);
    expect(context.drawImage).not.toHaveBeenCalled();
  });

  it("yields between tiles, allowing a retired image to cancel before another read", async () => {
    vi.useFakeTimers();
    let cancelled = false;
    const { context, canvas } = canvasPixels(() => [0, 0, 0, 255]);
    const pending = analyseImage(decodedImage(513, 257), () => cancelled);
    expect(context.getImageData).toHaveBeenCalledTimes(1);
    cancelled = true;
    await vi.runAllTimersAsync();
    expect(await pending).toBeUndefined();
    expect(context.getImageData).toHaveBeenCalledTimes(1);
    expect(canvas()).toMatchObject({ width: 0, height: 0 });
  });

  it("releases the canvas when pixel access fails", async () => {
    const { context, canvas } = canvasPixels(() => [0, 0, 0, 255]);
    context.getImageData.mockImplementation(() => { throw new DOMException("Unreadable image", "SecurityError"); });
    await expect(analyseImage(decodedImage(32, 32), () => false)).rejects.toMatchObject({ name: "SecurityError" });
    expect(canvas()).toMatchObject({ width: 0, height: 0 });
  });
});

describe("displayed backdrop samples", () => {
  it("composites transparency over the solid background and keeps conservative channel bounds", () => {
    const transparent = cellBounds(32, 32, () => [255, 0, 255, 0]);
    const region = { left: 0, top: 0, width: 32, height: 32 };
    expect(backdropSamples(transparent, region, region, CENTRED, 0, "#204060")).toEqual([{ min: [32, 64, 96], max: [32, 64, 96] }]);

    // RGB and alpha extrema need not belong to the same source pixel. Combining them must
    // enclose every possible composite instead of assuming that the light pixels are opaque.
    const varying = cellBounds(32, 32);
    varying.cells.set([0, 20, 40, 100, 180, 240, 64, 192]);
    const [sample] = backdropSamples(varying, region, region, CENTRED, 0, "#204060");
    const expectedMin = [32 * 63 / 255, 20 * 192 / 255 + 64 * 63 / 255, 40 * 192 / 255 + 96 * 63 / 255];
    const expectedMax = [100 * 192 / 255 + 32 * 63 / 255, 180 * 192 / 255 + 64 * 63 / 255, 240 * 192 / 255 + 96 * 63 / 255];
    for (let channel = 0; channel < 3; channel++) {
      expect(sample.min[channel]).toBeCloseTo(expectedMin[channel]);
      expect(sample.max[channel]).toBeCloseTo(expectedMax[channel]);
    }
  });

  it("includes the chat layer's centred 1.08 enlargement", () => {
    const bounds = cellBounds();
    // This screen point sees source cell (2, 2) after enlargement; an unscaled image would
    // sample cell (1, 1), causing the readability decision to use the wrong part of the photo.
    expect(backdropSamples(bounds, { width: 320, height: 320 }, { left: 60, top: 60, width: 1, height: 1 }, CENTRED, 0, "#000000"))
      .toEqual([{ min: [20, 20, 0], max: [20, 20, 0] }]);
  });

  it.each([
    { name: "cover-fitted left edge", width: 640, height: 320, crop: { zoom: 100, x: 0, y: 500 }, expected: [20, 20, 0] },
    { name: "cover-fitted right edge", width: 640, height: 320, crop: { zoom: 100, x: 1000, y: 500 }, expected: [120, 20, 0] }
  ])("samples the $name", ({ width, height, crop, expected }) => {
    expect(backdropSamples(cellBounds(width, height), { width: 320, height: 320 }, { left: 60, top: 60, width: 1, height: 1 }, crop, 0, "#000000"))
      .toEqual([{ min: expected, max: expected }]);
  });

  it.each([
    { name: "top-left", crop: { zoom: 200, x: 0, y: 0 }, expected: [10, 0, 0] },
    { name: "bottom-right", crop: { zoom: 200, x: 1000, y: 1000 }, expected: [60, 80, 0] }
  ])("uses the zoomed $name crop in a non-square window", ({ crop, expected }: { crop: ImageCrop; expected: number[] }) => {
    expect(backdropSamples(cellBounds(), { width: 320, height: 160 }, { left: 60, top: 50, width: 1, height: 1 }, crop, 0, "#000000"))
      .toEqual([{ min: expected, max: expected }]);
  });

  it("includes a bright patch brought into the text region by the blur halo", () => {
    const bounds = cellBounds(320, 320, (x, y) => x === 7 && y === 5 ? [255, 255, 255, 255] : [0, 0, 0, 255]);
    const frame = { width: 320, height: 320 };
    const region = { left: 177, top: 177, width: 1, height: 1 };
    expect(backdropSamples(bounds, frame, region, CENTRED, 0, "#000000")).toEqual([{ min: [0, 0, 0], max: [0, 0, 0] }]);
    expect(backdropSamples(bounds, frame, region, CENTRED, 16, "#000000")).toEqual([{ min: [0, 0, 0], max: [255, 255, 255] }]);
  });

  it.each([
    { name: "left", left: 0, top: 160 },
    { name: "right", left: 319, top: 160 },
    { name: "top", left: 160, top: 0 },
    { name: "bottom", left: 160, top: 319 }
  ])("includes the solid background blurred in from the pseudo-element's $name paint edge", ({ left, top }) => {
    // The image extends horizontally beyond its paint box, so sampling the source image
    // alone would incorrectly report pure white near either horizontal window edge.
    const bounds = cellBounds(640, 320, () => [255, 255, 255, 255]);
    const frame = { width: 320, height: 320 };
    const region = { left, top, width: 1, height: 1 };
    expect(backdropSamples(bounds, frame, region, CENTRED, 0, "#204060")).toEqual([{ min: [255, 255, 255], max: [255, 255, 255] }]);
    expect(backdropSamples(bounds, frame, region, CENTRED, 8, "#204060")).toEqual([{ min: [32, 64, 96], max: [255, 255, 255] }]);
    expect(backdropSamples(bounds, frame, { left: 160, top: 160, width: 1, height: 1 }, CENTRED, 8, "#204060"))
      .toEqual([{ min: [255, 255, 255], max: [255, 255, 255] }]);
  });

  it("falls back to the full colour range when there is no measurable frame or background", () => {
    const bounds = cellBounds();
    const region = { left: 0, top: 0, width: 100, height: 100 };
    for (const [frame, background] of [[{ width: 0, height: 100 }, "#111111"], [{ width: 100, height: 100 }, "invalid"]] as const) {
      expect(backdropSamples(bounds, frame, region, CENTRED, 0, background)).toEqual([{ min: [0, 0, 0], max: [255, 255, 255] }]);
    }
  });
});
