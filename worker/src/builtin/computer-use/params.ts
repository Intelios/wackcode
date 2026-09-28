/**
 * Parameter schemas for the computer-use tools, as plain JSON Schema like the browser's. The
 * native host re-validates every field (`src-tauri/src/computer_use/request.rs`): these only
 * describe the contract to the model.
 */

export const COMPUTER_APPS_TOOL_NAME = "computer_apps";
export const COMPUTER_OPEN_TOOL_NAME = "computer_open";
export const COMPUTER_SNAPSHOT_TOOL_NAME = "computer_snapshot";
export const COMPUTER_SCREENSHOT_TOOL_NAME = "computer_screenshot";
export const COMPUTER_ACT_TOOL_NAME = "computer_act";
export const COMPUTER_TOOL_NAMES = [
  COMPUTER_APPS_TOOL_NAME,
  COMPUTER_OPEN_TOOL_NAME,
  COMPUTER_SNAPSHOT_TOOL_NAME,
  COMPUTER_SCREENSHOT_TOOL_NAME,
  COMPUTER_ACT_TOOL_NAME,
] as const;

export const MAX_ACTIONS = 20;

const APP = {
  type: "string",
  description: "The app: its name as computer_apps lists it (e.g. \"TextEdit\"), its bundle id, or for computer_open an absolute path to a .app bundle.",
} as const;

const WINDOW = {
  type: "integer",
  description: "A window id from computer_apps or an earlier result. Default: the app's main window.",
} as const;

export const EMPTY_PARAMS = { type: "object", additionalProperties: false, properties: {} } as const;

export const OPEN_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["app"],
  properties: { app: APP },
} as const;

export const OBSERVE_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["app"],
  properties: { app: APP, window: WINDOW },
} as const;

export const ACT_PARAMS = {
  type: "object",
  additionalProperties: false,
  required: ["app", "stateId", "actions"],
  properties: {
    app: APP,
    stateId: { type: "string", description: "The stateId of the latest computer_snapshot or computer_screenshot of this app's window." },
    actions: {
      type: "array",
      minItems: 1,
      maxItems: MAX_ACTIONS,
      description: "Actions performed in order. The batch stops at the first one that fails.",
      items: {
        type: "object",
        additionalProperties: false,
        required: ["kind"],
        properties: {
          kind: {
            type: "string",
            enum: ["press", "click", "setText", "typeText", "keypress", "scroll", "drag", "menu", "raise", "wait"],
            description:
              "press: activate the element `ref` (a button, checkbox, menu item…). click: a `ref`, or screenshot pixel `x`,`y` (coordinates briefly bring the app forward). setText: replace a text field's value. typeText: type `text` into `ref` or the focused element. keypress: one key or chord in `keys`, e.g. \"cmd+s\", \"Enter\", \"shift+tab\". scroll: scroll `ref` or the point `x`,`y` by `dx`/`dy` steps (positive dy scrolls down). drag: from `x`,`y` to `toX`,`toY` (screenshot pixels). menu: choose a menu-bar item by `path`, e.g. [\"File\", \"Save As…\"]. raise: bring the window to the front of the app without activating it. wait: pause `ms` milliseconds.",
          },
          ref: { type: "string", description: "An element ref from the stateId's snapshot, e.g. \"e3-12\"." },
          x: { type: "number", description: "Screenshot pixel x (needs a stateId from computer_screenshot)." },
          y: { type: "number", description: "Screenshot pixel y." },
          toX: { type: "number", description: "drag: end x in screenshot pixels." },
          toY: { type: "number", description: "drag: end y in screenshot pixels." },
          button: { type: "string", enum: ["left", "right", "middle"], description: "click: default left." },
          count: { type: "integer", minimum: 1, maximum: 3, description: "click: 2 for a double click." },
          text: { type: "string", description: "setText / typeText: the text." },
          keys: { type: "string", description: "keypress: the key or chord." },
          dx: { type: "number", description: "scroll: horizontal steps." },
          dy: { type: "number", description: "scroll: vertical steps." },
          path: { type: "array", items: { type: "string" }, minItems: 1, description: "menu: titles from the menu bar down." },
          ms: { type: "integer", minimum: 0, maximum: 5000, description: "wait: milliseconds." },
        },
      },
    },
    expect: {
      type: "object",
      additionalProperties: false,
      description: "Optional postcondition, polled for up to timeoutMs after the actions: an element matching every given field exists (or, with exists:false, none does).",
      properties: {
        role: { type: "string", description: "e.g. \"AXButton\"." },
        titleContains: { type: "string" },
        valueEquals: { type: "string" },
        windowTitleContains: { type: "string" },
        exists: { type: "boolean", description: "Default true." },
        timeoutMs: { type: "integer", minimum: 0, maximum: 5000 },
      },
    },
  },
} as const;
