/**
 * Three-layer Apple Liquid Glass surface for Classic Liquid Glass controls and popovers.
 *
 * WebKit drops `-apple-visual-effect`'s hosted material layer if `box-shadow`, `backdrop-filter`,
 * `mask`, or scroll clipping sits on the same DOM node, so the refractive lens, soft reading
 * frost + shadow, and Fresnel specular rim each live on their own sibling span behind content.
 */
export function GlassPlatter() {
  return (
    <span className="glass-platter" aria-hidden="true">
      <span className="glass-lens" />
      <span className="glass-frost" />
      <span className="glass-rim" />
    </span>
  );
}
