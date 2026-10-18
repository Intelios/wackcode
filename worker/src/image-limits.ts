/**
 * The inline limits every image WackCode sends is reduced to, at the point it enters a session.
 *
 * Pi reads `inputLimits.images.resize` off the model for prompt images and for every tool
 * result's images (`read`, `browser_screenshot`, `computer_screenshot`, MCP), so declaring it on
 * the model in `models.json` shrinks images at the source instead of leaving them at Pi's roomy
 * defaults (2000 px, 4.5 MB of base64 each). Pi appends a dimension note whenever it resizes, so
 * screenshot coordinates stay mappable.
 *
 * A request's *total* image size is bounded separately, in `builtin/image-budget.ts`: one image
 * fits here, but a chat that reads a dozen screenshots would otherwise still grow every request.
 */
export const INLINE_IMAGE_LIMITS = {
  maxWidth: 1600,
  maxHeight: 1600,
  // Of base64 payload, like Pi's own limit. Keeps a photo-sized image in the low hundreds of KB.
  maxBytes: 2 * 1024 * 1024,
  jpegQuality: 80
} as const;
