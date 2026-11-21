// WackCode's canvas runtime: put at the top of every artboard's page by the canvas site
// (store.rs), before the agent's own markup.
//
// It does three things and nothing else:
// - applies the artboard's tweaks (`__wackCanvas.tweaks`) to <html> before the first paint, and
//   again when the canvas sets one (`__wack.setTweak`);
// - measures the page once it has settled (`__wack.settle()`): its content's real height (from
//   the elements' boxes, not scrollHeight, so a shorter revision shrinks), how far it overflows,
//   and a map of its visible elements for point-and-comment;
// - stays out of the way: no globals but `__wack`, no network, no listeners on the page's input.
//
// The engine reads it only through DevTools (`Runtime.evaluate`), never through a channel the
// page could reach on its own, and treats everything it returns as data from an agent-written
// page: sizes are clamped, text is cut short.
(function () {
  "use strict";
  var config = window.__wackCanvas || { tweaks: [] };
  try { delete window.__wackCanvas; } catch (e) { window.__wackCanvas = undefined; }
  var root = document.documentElement;

  function applyTweak(tweak) {
    if (!tweak || typeof tweak.value !== "string") return;
    if (typeof tweak.var === "string" && tweak.var.indexOf("--") === 0) {
      root.style.setProperty(tweak.var, tweak.value);
    } else if (typeof tweak.attr === "string" && /^[a-z0-9-]+$/.test(tweak.attr)) {
      root.setAttribute("data-" + tweak.attr, tweak.value);
    }
  }
  (config.tweaks || []).forEach(applyTweak);

  var SKIP = { SCRIPT: 1, STYLE: 1, META: 1, LINK: 1, HEAD: 1, TITLE: 1, NOSCRIPT: 1, TEMPLATE: 1, BR: 1, WBR: 1 };
  var MAX_ELEMENTS = 2000;

  function nextFrame() {
    return new Promise(function (resolve) { requestAnimationFrame(function () { resolve(); }); });
  }

  function loaded() {
    if (document.readyState === "complete") return Promise.resolve();
    return new Promise(function (resolve) { window.addEventListener("load", function () { resolve(); }, { once: true }); });
  }

  function quiet(ms) {
    return new Promise(function (resolve) { setTimeout(resolve, ms); });
  }

  function ownText(element) {
    var text = "";
    for (var node = element.firstChild; node && text.length < 120; node = node.nextSibling) {
      if (node.nodeType === 3) text += node.nodeValue;
    }
    text = text.replace(/\s+/g, " ").trim();
    if (!text && (element.tagName === "IMG" || element.tagName === "INPUT")) {
      text = element.getAttribute("alt") || element.getAttribute("placeholder") || element.getAttribute("value") || "";
    }
    if (!text) text = element.getAttribute("aria-label") || "";
    return text.slice(0, 60);
  }

  // Every visible element under <body>, depth first: its child-index path from <body>, tag, id,
  // first classes, a few words and its box in page pixels.
  function map() {
    var out = [];
    var scrollX = window.scrollX, scrollY = window.scrollY;
    function walk(element, path) {
      if (out.length >= MAX_ELEMENTS) return;
      var index = 0;
      for (var child = element.firstElementChild; child; child = child.nextElementSibling, index++) {
        if (out.length >= MAX_ELEMENTS) return;
        if (SKIP[child.tagName]) continue;
        var childPath = path ? path + "/" + index : String(index);
        var style = getComputedStyle(child);
        if (style.display === "none") continue;
        var rect = child.getBoundingClientRect();
        if (rect.width > 0 && rect.height > 0 && style.visibility !== "hidden") {
          var classes = typeof child.className === "string" ? child.className.trim().split(/\s+/).slice(0, 3).join(" ") : "";
          out.push({
            p: childPath,
            t: child.tagName.toLowerCase(),
            i: (child.id || "").slice(0, 60),
            c: classes.slice(0, 80),
            s: ownText(child),
            x: Math.round((rect.left + scrollX) * 2) / 2,
            y: Math.round((rect.top + scrollY) * 2) / 2,
            w: Math.round(rect.width * 2) / 2,
            h: Math.round(rect.height * 2) / 2,
          });
        }
        if (child.tagName !== "svg" && child.tagName !== "SVG") walk(child, childPath);
      }
    }
    if (document.body) walk(document.body, "");
    return out;
  }

  // The bottom of the content, from the boxes of what is drawn, so a page that got shorter
  // measures shorter (scrollHeight never drops below the viewport).
  function contentHeight(elements) {
    var bottom = 0;
    for (var i = 0; i < elements.length; i++) {
      var e = elements[i];
      if (e.y + e.h > bottom) bottom = e.y + e.h;
    }
    var body = document.body;
    if (body) {
      var style = getComputedStyle(body);
      bottom += (parseFloat(style.paddingBottom) || 0) + (parseFloat(style.marginBottom) || 0) + (parseFloat(style.borderBottomWidth) || 0);
    }
    return Math.ceil(bottom);
  }

  var api = {
    settle: function () {
      return loaded()
        .then(function () { return document.fonts && document.fonts.ready; })
        .then(function () {
          var images = Array.prototype.slice.call(document.images || []);
          return Promise.all(images.map(function (img) {
            return img.complete ? null : new Promise(function (resolve) { img.onload = img.onerror = resolve; setTimeout(resolve, 2000); });
          }));
        })
        .then(nextFrame)
        .then(nextFrame)
        .then(function () { return quiet(120); })
        .then(function () { return api.measure(); });
    },
    measure: function () {
      var elements = map();
      var width = Math.max(root.scrollWidth, document.body ? document.body.scrollWidth : 0);
      var height = contentHeight(elements);
      return {
        viewport: [window.innerWidth, window.innerHeight],
        scroll: [width, Math.max(root.scrollHeight, height)],
        content: height,
        title: (document.title || "").slice(0, 80),
        elements: elements,
      };
    },
    setTweak: function (tweak) { applyTweak(tweak); return true; },
    // The markup of the element at a child-index path from <body> (as `map` writes it), cut
    // short: what a comment on it is about.
    markup: function (path, max) {
      var element = document.body;
      var steps = typeof path === "string" && path ? path.split("/") : [];
      for (var i = 0; element && i < steps.length; i++) element = element.children[Number(steps[i])];
      return element ? element.outerHTML.slice(0, Math.max(0, Math.min(max || 0, 4000))) : "";
    },
  };
  try {
    Object.defineProperty(window, "__wack", { value: Object.freeze(api), configurable: false, writable: false });
  } catch (e) {}
})();
