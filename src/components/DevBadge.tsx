// The DEV badge in the title bar of every dev build (`devBuild` from bootstrap). Shell chrome,
// so it goes see-through over an image like the rest of the header; `data-tauri-drag-region`
// keeps it from blocking window drags. Dev only — nothing else should render it.

export function DevBadge() {
  return (
    <span className="dev-badge" data-tauri-drag-region title="Development build">
      DEV
    </span>
  );
}
