/* Runs only in WKContentWorld.defaultClientWorld. No page bridge or page-owned helpers.
 * Coordinates are CSS viewport coordinates; frame/shadow paths describe capture-time evidence.
 * Work and text are bounded before crossing IPC. Never serialize executable/sensitive content. */
(point => {
  const viewport = { width: innerWidth, height: innerHeight, x: scrollX, y: scrollY, url: location.href };
  const targets = [];
  let visited = 0, truncated = false;
  // Slice by UTF-16 budget without handing Rust a split surrogate pair.
  const clip = (text, limit) => {
    const end = text.charCodeAt(limit - 1);
    return text.slice(0, end >= 0xd800 && end <= 0xdbff ? limit - 1 : limit);
  };
  const selector = (element, root) => {
    for (const attr of ['id', 'data-testid', 'data-test']) {
      const value = element.getAttribute(attr);
      if (!value || value.length > 512) continue;
      const candidate = `[${attr}="${CSS.escape(value)}"]`;
      if (root.querySelectorAll(candidate).length === 1) return candidate;
    }
    const parts = [];
    for (let node = element; node?.nodeType === 1 && parts.length < 64; node = node.parentElement) {
      let part = node.localName;
      const siblings = node.parentElement ? [...node.parentElement.children].filter(other => other.localName === node.localName) : [];
      if (siblings.length > 1) part += `:nth-of-type(${siblings.indexOf(node) + 1})`;
      parts.unshift(part);
      if (root.querySelectorAll(parts.join(' > ')).length === 1) break;
    }
    return parts.join(' > ');
  };
  const editable = node => node.isContentEditable || Boolean(node.closest?.('[contenteditable]:not([contenteditable="false"])'));
  const snippet = element => {
    let nodes = 0, shortened = false;
    const words = [];
    const bounded = (text, limit) => { if (text.length > limit) shortened = true; return clip(text, limit); };
    const escape = text => String(text).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
    const serialize = (node, depth) => {
      if (++nodes > 70 || depth > 5) { shortened = true; return '…'; }
      if (node.nodeType === 3) { const text = bounded(node.textContent || '', 400); words.push(text); return escape(text); }
      if (node.nodeType !== 1 || /^(script|style|noscript|template|meta|link)$/i.test(node.localName)) return '';
      if (node.localName === 'input' && node.type === 'hidden') return '';
      // Serialize text directly. Cloning custom elements into the page's document can run
      // their constructors, even while detached, so inspection never creates page elements.
      const attrs = ['id','class','role','aria-label','aria-labelledby','type','name','placeholder','data-testid','data-test']
        .filter(attr => node.hasAttribute(attr)).map(attr => ` ${attr}="${escape(bounded(node.getAttribute(attr), 300))}"`).join('');
      let children = '';
      if (!/^(input|textarea|select)$/i.test(node.localName) && !editable(node)) {
        for (const child of node.childNodes) { if (nodes >= 70) { shortened = true; break; } children += serialize(child, depth + 1); }
      }
      return `<${node.localName}${attrs}>${children}</${node.localName}>`;
    };
    const dom = bounded(serialize(element, 0), 2048);
    return { dom, text: clip(words.join(''), 90), shortened };
  };
  const describe = (element, root, path, ox, oy, sx, sy) => {
    const rect = element.getBoundingClientRect();
    const bounds = { x: ox + rect.x * sx, y: oy + rect.y * sy, width: rect.width * sx, height: rect.height * sy };
    const { dom, text, shortened } = snippet(element);
    return { selector: selector(element, root), path, bounds, dom,
      label: clip((element.getAttribute('aria-label') || element.getAttribute('title') || (/^(input|textarea|select|option)$/.test(element.localName) || editable(element) ? element.getAttribute('placeholder') : text) || element.localName).trim(), 90),
      truncated: shortened };
  };
  const walk = (root, path = [], ox = 0, oy = 0, sx = 1, sy = 1) => {
    if (path.length > 12) { truncated = true; return; }
    const scan = (node, depth = 0) => {
      if (depth > 64) { truncated = true; return; }
      if (++visited > 12000 || targets.length >= 1200) { truncated = true; return; }
      const rect = node.getBoundingClientRect(), style = getComputedStyle(node);
      if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0 || /^(script|style|noscript|template)$/i.test(node.localName)) return;
      const entry = describe(node, root, path, ox, oy, sx, sy), r = entry.bounds;
      const visible = r.width > 0 && r.height > 0 && r.x < innerWidth && r.y < innerHeight && r.x + r.width > 0 && r.y + r.height > 0;
      if (visible && !/^(html|body)$/i.test(node.localName)) targets.push(entry);
      if (node.shadowRoot) walk(node.shadowRoot, [...path, { kind: 'shadow', selector: entry.selector }], ox, oy, sx, sy);
      if (node.localName === 'iframe' && visible) {
        try {
          if (node.contentDocument?.documentElement) {
            const fx = rect.width / Math.max(node.offsetWidth, 1), fy = rect.height / Math.max(node.offsetHeight, 1);
            walk(node.contentDocument, [...path, { kind: 'frame', selector: entry.selector }], r.x + node.clientLeft * fx * sx, r.y + node.clientTop * fy * sy, sx * fx, sy * fy);
          }
        } catch (_) { /* The containing iframe remains the visual target. */ }
      }
      for (const child of node.children) { if (visited > 12000 || targets.length >= 1200) break; scan(child, depth + 1); }
    };
    for (const node of root.children) scan(node);
  };
  const hit = (root, x, y, path = [], ox = 0, oy = 0, sx = 1, sy = 1) => {
    if (path.length > 12) return null;
    const element = root.elementFromPoint(x, y);
    if (!element) return null;
    const entry = describe(element, root, path, ox, oy, sx, sy);
    if (element.shadowRoot) return hit(element.shadowRoot, x, y, [...path, { kind: 'shadow', selector: entry.selector }], ox, oy, sx, sy) || entry;
    if (element.localName === 'iframe') {
      try {
        const rect = element.getBoundingClientRect();
        const fx = rect.width / Math.max(element.offsetWidth, 1), fy = rect.height / Math.max(element.offsetHeight, 1);
        if (element.contentDocument) return hit(element.contentDocument, (x - rect.x) / fx - element.clientLeft, (y - rect.y) / fy - element.clientTop,
          [...path, { kind: 'frame', selector: entry.selector }], entry.bounds.x + element.clientLeft * fx * sx, entry.bounds.y + element.clientTop * fy * sy, sx * fx, sy * fy) || entry;
      } catch (_) {}
    }
    return entry;
  };
  if (point) {
    const target = hit(document, point.x, point.y);
    return { viewport, targets: target ? [target] : [], title: document.title, truncated: false };
  }
  walk(document);
  return { viewport, targets, title: document.title, truncated };
})
