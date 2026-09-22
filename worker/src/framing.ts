export class JsonLineDecoder {
  // Chunks of the line still being received. Kept as a list so a multi-megabyte line (a prompt
  // carrying images) is joined once, not re-copied on every chunk.
  private pending: Buffer[] = [];

  push(chunk: Buffer): string[] {
    const lines: string[] = [];
    let start = 0;
    let newline = chunk.indexOf(0x0a);
    while (newline >= 0) {
      this.pending.push(chunk.subarray(start, newline));
      const raw = this.pending.length === 1 ? this.pending[0] : Buffer.concat(this.pending);
      this.pending = [];
      const clean = raw.at(-1) === 0x0d ? raw.subarray(0, -1) : raw;
      if (clean.length > 0) lines.push(clean.toString("utf8"));
      start = newline + 1;
      newline = chunk.indexOf(0x0a, start);
    }
    if (start < chunk.length) this.pending.push(chunk.subarray(start));
    return lines;
  }
}
