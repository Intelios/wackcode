/**
 * What fills the context window, itemised.
 *
 * The provider gives one exact number per turn: how many tokens the request carried. This module
 * explains it. Each row is an estimate from the worker's own view of the request — Pi's per-message
 * estimate, the live prompt's named sections, the active tools' declarations — and the provider's
 * count is the only exact figure, so the rows are scaled to it: they always add up to what the
 * model was actually charged, and the proportions the estimates found survive. A row that reads
 * zero is a row the request really did not carry.
 *
 * The coarse four-part `entries` stays alongside for history views, which have no request to
 * measure: a saved session's tools and prompt of the moment cannot be reconstructed honestly.
 */

/** One item under a group: a skill, a project file, a tool, a memory note. */
export interface BreakdownRow {
  label: string;
  tokens: number;
}

export interface BreakdownGroup {
  id: string;
  label: string;
  tokens: number;
  /** Present when the group has more than one item worth showing. */
  children?: BreakdownRow[];
}

/** A section's own measured size plus the rows it listed. */
export interface BreakdownSection {
  total: number;
  rows: BreakdownRow[];
}

export interface ContextDetail {
  groups: BreakdownGroup[];
  /** Tokens Pi keeps clear of the conversation before it compacts automatically. */
  reserveTokens: number;
}

export interface ContextDetailInput {
  /** Per-message estimates: the conversation itself. */
  messages: { user: number; assistant: number; tool: number };
  /** Hidden custom messages: the Plan/Ultra mode contract. */
  contract: number;
  /** Other app-injected custom messages, e.g. background sub-agent results. */
  appContext: number;
  /** Compaction and branch summaries carried forward. */
  summary: number;
  prompt: {
    /** The preamble and the working directory Pi appends to it. */
    preamble: number;
    /** Everything Pi appends, in tokens, minus the memory index (which is its own group). */
    addendum: number;
    /** Known addendum parts, in serving order: the app's guide, Team, Ultra. */
    addendumParts: BreakdownRow[];
    /** Prompt sections WackCode does not own, e.g. one a package added. */
    others: BreakdownRow[];
  };
  /** One row per project instructions file (AGENTS.md, CLAUDE.md…). */
  project: BreakdownSection;
  skills: BreakdownSection;
  memory: BreakdownSection;
  /** Tool groups in display order, already named for their origin. */
  tools: { id: string; label: string; tools: BreakdownRow[] }[];
  /** The provider's count for the whole request. */
  used: number;
  reserveTokens: number;
}

/** Pi's estimate for text that is not a message: roughly four characters per token. */
export function textTokens(text: string | undefined): number {
  return charsToTokens(text?.length ?? 0);
}

export function charsToTokens(chars: number): number {
  return chars > 0 ? Math.ceil(chars / 4) : 0;
}

function sum(values: readonly number[]): number {
  return values.reduce((total, value) => total + value, 0);
}

/**
 * Divide `total` over `values` in their own proportions, in whole tokens: floor each share, then
 * hand the leftover tokens to the largest remainders. The result always sums to `total`.
 */
export function apportion(values: readonly number[], total: number): number[] {
  const weights = values.map((value) => (Number.isFinite(value) && value > 0 ? value : 0));
  const weight = sum(weights);
  const wanted = Math.max(0, Math.round(Number.isFinite(total) ? total : 0));
  if (weight <= 0 || wanted === 0) return weights.map(() => 0);
  const exact = weights.map((value) => (value * wanted) / weight);
  const result = exact.map((value) => Math.floor(value));
  let left = wanted - sum(result);
  const order = exact
    .map((value, index) => ({ index, fraction: value - Math.floor(value) }))
    .sort((a, b) => b.fraction - a.fraction);
  for (let position = 0; position < order.length && left > 0; position += 1) {
    result[order[position].index] += 1;
    left -= 1;
  }
  return result;
}

interface Draft {
  id: string;
  label: string;
  /** The group's own estimate; defaults to the sum of its children. */
  tokens: number;
  children: BreakdownRow[];
}

function section(id: string, label: string, value: BreakdownSection): Draft {
  return { id, label, tokens: Math.max(value.total, sum(value.rows.map((row) => row.tokens))), children: value.rows };
}

function plain(id: string, label: string, tokens: number): Draft {
  return { id, label, tokens, children: [] };
}

export function contextDetail(input: ContextDetailInput): ContextDetail {
  const knownAddendum = sum(input.prompt.addendumParts.map((part) => part.tokens));
  const promptChildren: BreakdownRow[] = [
    ...(input.prompt.preamble > 0 ? [{ label: "Core prompt", tokens: input.prompt.preamble }] : []),
    ...input.prompt.addendumParts,
    ...(input.prompt.addendum > knownAddendum
      ? [{ label: "Other instructions", tokens: input.prompt.addendum - knownAddendum }]
      : []),
    ...input.prompt.others
  ];
  const systemTokens = Math.max(
    input.prompt.preamble + input.prompt.addendum + sum(input.prompt.others.map((row) => row.tokens)),
    sum(promptChildren.map((row) => row.tokens))
  );

  const drafts: Draft[] = [
    {
      id: "messages",
      label: "Messages",
      tokens: input.messages.user + input.messages.assistant + input.messages.tool,
      children: [
        { label: "User messages", tokens: input.messages.user },
        { label: "Assistant replies", tokens: input.messages.assistant },
        { label: "Tool results", tokens: input.messages.tool }
      ]
    },
    ...(input.summary > 0 ? [plain("summary", "Compacted summary", input.summary)] : []),
    ...(input.contract > 0 ? [plain("contract", "Mode contract", input.contract)] : []),
    ...(input.appContext > 0 ? [plain("app-context", "App context", input.appContext)] : []),
    ...input.tools.map((group) => ({
      id: group.id,
      label: group.label,
      tokens: sum(group.tools.map((row) => row.tokens)),
      children: group.tools
    })),
    ...(systemTokens > 0 ? [{ id: "system", label: "System prompt", tokens: systemTokens, children: promptChildren }] : []),
    ...(input.project.total > 0 || input.project.rows.length > 0 ? [section("project", "Project instructions", input.project)] : []),
    ...(input.skills.total > 0 || input.skills.rows.length > 0 ? [section("skills", "Skills", input.skills)] : []),
    ...(input.memory.total > 0 || input.memory.rows.length > 0 ? [section("memory", "Memory", input.memory)] : [])
  ];

  const totals = apportion(drafts.map((draft) => draft.tokens), input.used);
  const groups: BreakdownGroup[] = [];
  drafts.forEach((draft, index) => {
    const tokens = totals[index];
    if (tokens <= 0) return;
    const sized = apportion(draft.children.map((row) => row.tokens), tokens);
    const children = draft.children
      .map((row, position) => ({ label: row.label, tokens: sized[position] }))
      .filter((row) => row.tokens > 0);
    groups.push({ id: draft.id, label: draft.label, tokens, ...(children.length > 1 ? { children } : {}) });
  });
  return { groups, reserveTokens: input.reserveTokens };
}
