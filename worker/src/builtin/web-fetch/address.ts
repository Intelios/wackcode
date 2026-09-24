/**
 * Which addresses `web_fetch` may connect to: public unicast only. Page content can steer the
 * model, so without this a prompt injection could aim the tool at the user's router, a local dev
 * server, the mock provider, or a cloud metadata endpoint. The check runs on the address the
 * socket actually connects to (see `fetch.ts`), never on a hostname alone.
 */
import { isIP } from "node:net";

/** Names that always mean this machine or the local network, refused before any lookup. */
export function isBlockedHostname(hostname: string): boolean {
  const name = hostname.toLowerCase().replace(/\.$/, "");
  return (
    name === "localhost" ||
    name.endsWith(".localhost") ||
    name.endsWith(".local") ||
    name.endsWith(".internal") ||
    name.endsWith(".home.arpa")
  );
}

/** True unless `address` is a public unicast IPv4 or IPv6 address. Unparseable input is blocked. */
export function isBlockedAddress(address: string): boolean {
  const bare = address.replace(/^\[|\]$/g, "").replace(/%.*$/, "");
  switch (isIP(bare)) {
    case 4:
      return isBlockedIpv4(parseIpv4(bare));
    case 6: {
      const words = parseIpv6(bare);
      return words ? isBlockedIpv6(words) : true;
    }
    default:
      return true;
  }
}

/** [first address as a 32-bit number, prefix length] */
const BLOCKED_IPV4: ReadonlyArray<readonly [string, number]> = [
  ["0.0.0.0", 8], // "this" network
  ["10.0.0.0", 8], // private
  ["100.64.0.0", 10], // carrier-grade NAT
  ["127.0.0.0", 8], // loopback
  ["169.254.0.0", 16], // link-local, including cloud metadata
  ["172.16.0.0", 12], // private
  ["192.0.0.0", 24], // IETF protocol assignments
  ["192.0.2.0", 24], // documentation
  ["192.88.99.0", 24], // 6to4 relay anycast
  ["192.168.0.0", 16], // private
  ["198.18.0.0", 15], // benchmarking
  ["198.51.100.0", 24], // documentation
  ["203.0.113.0", 24], // documentation
  ["224.0.0.0", 4], // multicast
  ["240.0.0.0", 4], // reserved, including broadcast
];
const BLOCKED_IPV4_RANGES = BLOCKED_IPV4.map(([base, bits]) => [parseIpv4(base), bits] as const);

function parseIpv4(address: string): number {
  return address.split(".").reduce((value, part) => value * 256 + Number(part), 0);
}

function isBlockedIpv4(value: number): boolean {
  return BLOCKED_IPV4_RANGES.some(([base, bits]) => {
    const size = 2 ** (32 - bits);
    return value >= base && value < base + size;
  });
}

/** Eight 16-bit words, or undefined when `address` is not valid IPv6. */
function parseIpv6(address: string): number[] | undefined {
  let text = address.toLowerCase();
  // A trailing dotted quad (::ffff:1.2.3.4) becomes its two words.
  const dotted = /(\d+\.\d+\.\d+\.\d+)$/.exec(text);
  if (dotted) {
    const value = parseIpv4(dotted[1]);
    text = `${text.slice(0, dotted.index)}${(value >>> 16).toString(16)}:${(value & 0xffff).toString(16)}`;
  }
  const halves = text.split("::");
  if (halves.length > 2) return undefined;
  const head = halves[0] ? halves[0].split(":") : [];
  const tail = halves.length === 2 && halves[1] ? halves[1].split(":") : [];
  const missing = 8 - head.length - tail.length;
  if (halves.length === 1 ? missing !== 0 : missing < 1) return undefined;
  const words = [...head, ...Array<string>(halves.length === 2 ? missing : 0).fill("0"), ...tail].map((word) =>
    Number.parseInt(word, 16),
  );
  return words.length === 8 && words.every((word) => word >= 0 && word <= 0xffff) ? words : undefined;
}

function embeddedIpv4(high: number, low: number): number {
  return high * 0x10000 + low;
}

function isBlockedIpv6(words: number[]): boolean {
  const [w0, w1, w2, w3, w4, w5, w6, w7] = words;
  const zeroPrefix = w0 === 0 && w1 === 0 && w2 === 0 && w3 === 0 && w4 === 0;
  // IPv4-mapped (::ffff:a.b.c.d): judge the embedded IPv4 address. The rest of ::/96 is
  // unspecified, loopback, or the deprecated IPv4-compatible form, none of which is public.
  if (zeroPrefix && w5 === 0xffff) return isBlockedIpv4(embeddedIpv4(w6, w7));
  if (zeroPrefix && w5 === 0) return true;
  // NAT64 well-known prefix 64:ff9b::/96 carries an IPv4 address in its last 32 bits.
  if (w0 === 0x64 && w1 === 0xff9b && w2 === 0 && w3 === 0 && w4 === 0 && w5 === 0) {
    return isBlockedIpv4(embeddedIpv4(w6, w7));
  }
  // 6to4 (2002::/16) carries an IPv4 address in bits 16-47.
  if (w0 === 0x2002) return isBlockedIpv4(embeddedIpv4(w1, w2));
  // Only global unicast (2000::/3) is public. That excludes loopback, unique-local fc00::/7,
  // link-local fe80::/10, multicast ff00::/8, discard 100::/64 and local-use NAT64 64:ff9b:1::/48.
  if ((w0 & 0xe000) !== 0x2000) return true;
  // Teredo (2001::/32) tunnels to arbitrary IPv4 hosts; documentation 2001:db8::/32 and
  // 3fff::/20 are never real.
  if (w0 === 0x2001 && (w1 === 0 || w1 === 0xdb8)) return true;
  if (w0 === 0x3fff && w1 < 0x1000) return true;
  return false;
}
