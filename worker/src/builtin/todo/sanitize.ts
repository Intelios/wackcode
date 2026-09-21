/**
 * Remove terminal control characters from model-controlled task text before it reaches a
 * transcript. Ported from `@juicesharp/rpiv-todo` v2.11.0 (MIT) `tool/sanitize.ts`
 * (built for Pi's terminal renderer; the same hygiene keeps task text inert here).
 * Complete CSI/OSC escape sequences are dropped whole (no printable remnants like `[31m`),
 * newlines and tabs become spaces so task fields cannot change the layout, and bidi
 * controls are removed so a field cannot reorder how neighbouring output reads.
 *
 * One mechanical deviation: upstream inlines control-character literals in its regexes;
 * here the same characters are built with `String.fromCharCode` so this source file stays
 * free of control bytes. The matched set is identical.
 */
const ESC = String.fromCharCode(0x1b);
const C1_CSI = String.fromCharCode(0x9b);
const C1_OSC = String.fromCharCode(0x9d);
const C1_ST = String.fromCharCode(0x9c);
const BEL = String.fromCharCode(0x07);
const NEWLINE = String.fromCharCode(0x0a);
const CARRIAGE_RETURN = String.fromCharCode(0x0d);
const TAB = String.fromCharCode(0x09);
const char = (code: number) => String.fromCharCode(code);

// CSI sequences, via both the ESC-[ form and the C1 single-byte introducer.
const CSI_SEQUENCE = new RegExp(`(?:${ESC}\\[|${C1_CSI})[0-?]*[ -/]*[@-~]`, "g");
// OSC sequences with their payload; an unterminated OSC swallows the rest of the string,
// matching how a real terminal would treat it.
const OSC_SEQUENCE = new RegExp(`(?:${ESC}\\]|${C1_OSC})[^${BEL}${C1_ST}${ESC}]*(?:${BEL}|${C1_ST}|${ESC}\\\\)?`, "g");
// Any remaining two-character ESC sequence.
const ESC_PAIR = new RegExp(`${ESC}.`, "g");
// Unicode line/paragraph separators join lines like newlines do below.
const LINE_SEPARATORS = new RegExp(`[${char(0x2028)}${char(0x2029)}]`, "g");
const CONTROL_CHARACTERS = new RegExp(`[${char(0x00)}-${char(0x1f)}${char(0x7f)}-${char(0x9f)}]`, "g");
// Bidi embedding/override/isolate controls and LRM/RLM marks.
const BIDI_CONTROLS = new RegExp(
  `[${char(0x200e)}${char(0x200f)}${char(0x202a)}-${char(0x202e)}${char(0x2066)}-${char(0x2069)}]`,
  "g",
);

export function sanitizeTerminalText(value: string): string {
  return value
    .replace(CSI_SEQUENCE, "")
    .replace(OSC_SEQUENCE, "")
    .replace(ESC_PAIR, "")
    .replace(LINE_SEPARATORS, " ")
    .replace(CONTROL_CHARACTERS, (character) =>
      character === NEWLINE || character === CARRIAGE_RETURN || character === TAB ? " " : "",
    )
    .replace(BIDI_CONTROLS, "");
}
