/** Pi 0.86.1's prompt-template argument rules, applied to the exact selected template. */
export function expandTemplate(content: string, input: string): string {
  const args: string[] = [];
  let current = "";
  let quote: string | null = null;
  for (const char of input) {
    if (quote) {
      if (char === quote) quote = null;
      else current += char;
    } else if (char === "'" || char === '"') quote = char;
    else if (/\s/.test(char)) {
      if (current) { args.push(current); current = ""; }
    } else current += char;
  }
  if (current) args.push(current);
  const all = args.join(" ");
  return content.replace(/\$\{(\d+|ARGUMENTS|@):-([^}]*)\}|\$\{@:(\d+)(?::(\d+))?\}|\$(ARGUMENTS|@|\d+)/g,
    (_match, target: string | undefined, fallback: string | undefined, sliceFrom: string | undefined, sliceCount: string | undefined, simple: string | undefined) => {
      if (target) {
        const value = target === "@" || target === "ARGUMENTS" ? all : args[Number(target) - 1];
        return value || fallback || "";
      }
      if (sliceFrom) {
        const start = Math.max(0, Number(sliceFrom) - 1);
        return args.slice(start, sliceCount ? start + Number(sliceCount) : undefined).join(" ");
      }
      if (simple === "@" || simple === "ARGUMENTS") return all;
      return args[Number(simple) - 1] ?? "";
    });
}
