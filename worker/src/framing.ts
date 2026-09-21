export class JsonLineDecoder {
  private pending: Buffer<ArrayBufferLike> = Buffer.alloc(0);

  push(chunk: Buffer): string[] {
    this.pending = this.pending.length === 0 ? chunk : Buffer.concat([this.pending, chunk]);
    const lines: string[] = [];
    let newline = this.pending.indexOf(0x0a);
    while (newline >= 0) {
      const raw = this.pending.subarray(0, newline);
      this.pending = this.pending.subarray(newline + 1);
      const clean = raw.at(-1) === 0x0d ? raw.subarray(0, -1) : raw;
      if (clean.length > 0) lines.push(clean.toString("utf8"));
      newline = this.pending.indexOf(0x0a);
    }
    return lines;
  }
}
