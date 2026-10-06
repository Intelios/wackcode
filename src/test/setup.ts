import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

// jsdom has no canvas and logs "Not implemented" for every getContext call. Components treat
// a null context as "no drawing" (CompactingStage's gravity well), so stub it once here.
vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
