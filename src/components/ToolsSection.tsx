import { useMemo, useState } from "react";
import { motion, useReducedMotion } from "motion/react";
import { firstSentence, groupTools } from "../tool-utils";
import type { PackageRecord, ToolCatalogEntry } from "../types";
import { Icon, type IconName } from "./Icons";
import { SettingsHero, stagger } from "./SettingsHero";

/** Settings sections that set up the tools this page doesn't switch. */
export type ToolsElsewhere = "packages" | "memory" | "mcp";

interface PiTool {
  name: string;
  group: "look" | "change";
  title: string;
  detail: string;
  icon: IconName;
  /** grep and find shell out to these; the worker reports the tool unavailable without them. */
  needs?: { name: string; install: string };
}

/**
 * Pi's own tools as a person would describe them, in the order shown. The catalogue's
 * descriptions are written for the model (limits, truncation, rules), so they sit behind each
 * card's "What the model is told". A Pi tool missing here still shows, under "More from Pi".
 */
const PI_TOOLS: PiTool[] = [
  { name: "read", group: "look", title: "Read files", detail: "Opens a file to look at it, images included. Long files are read a page at a time.", icon: "file" },
  {
    name: "grep", group: "look", title: "Search inside files", icon: "textSearch",
    detail: "Finds the lines matching a pattern across the project, skipping whatever .gitignore leaves out.",
    needs: { name: "ripgrep", install: "brew install ripgrep" }
  },
  {
    name: "find", group: "look", title: "Find files by name", icon: "search",
    detail: "Lists the files matching a pattern such as **/*.test.ts, skipping ignored ones.",
    needs: { name: "fd", install: "brew install fd" }
  },
  { name: "ls", group: "look", title: "List folders", detail: "Shows what's in a folder, hidden files included.", icon: "folder" },
  { name: "edit", group: "change", title: "Edit files", detail: "Changes part of a file by replacing exact text, leaving the rest untouched.", icon: "pencil" },
  { name: "write", group: "change", title: "Write files", detail: "Creates a file, or replaces one whole, making any folders it needs.", icon: "filePlus" },
  { name: "bash", group: "change", title: "Run commands", detail: "Runs shell commands in the chat's folder, like tests, builds, git and installs, with your permissions.", icon: "terminal" }
];

const ELSEWHERE: { id: ToolsElsewhere; icon: IconName; title: string; detail: string }[] = [
  { id: "packages", icon: "spark", title: "Built-ins", detail: "Web fetch, browser preview, sub-agents and computer use" },
  { id: "memory", icon: "memory", title: "Memory", detail: "Notes it saves and recalls, per project" },
  { id: "mcp", icon: "plug", title: "MCP servers", detail: "Tools from the servers you connect" }
];

/**
 * The hero's little stage: three tool calls land in a chat card one after another, each ticking
 * off as it finishes, like the transcript's own rows. Rests dashed and dimmed while every tool is
 * off. Pure decoration; the pill says the same in words. The loop lives in styles.css, which
 * stills it under reduced motion.
 */
function ToolsStage({ live }: { live: boolean }) {
  const rows = [
    { y: 22, verb: 16, subject: 44, glyph: (y: number) => `M28 ${y + 5}h6M28 ${y + 9}h4` },
    { y: 46, verb: 20, subject: 30, glyph: (y: number) => `M28 ${y + 10.5}l6-6.5` },
    { y: 70, verb: 12, subject: 52, glyph: (y: number) => `M28 ${y + 4}l3 3-3 3M32.5 ${y + 10}h2.5` }
  ];
  return (
    <svg className={`settings-stage tools-stage ${live ? "live" : ""}`} viewBox="0 0 160 110" aria-hidden="true">
      <rect className="tools-stage-card" x="12" y="10" width="136" height="90" rx="10" />
      {rows.map((row, index) => (
        <g key={row.y} className="tools-stage-row" style={{ "--k": index } as React.CSSProperties}>
          <rect className="tools-stage-icon" x="24" y={row.y} width="14" height="14" rx="4" />
          <path className="tools-stage-glyph" d={row.glyph(row.y)} />
          <rect className="tools-stage-verb" x="46" y={row.y + 5} width={row.verb} height="4" rx="2" />
          <rect className="tools-stage-subject" x={46 + row.verb + 5} y={row.y + 5} width={row.subject} height="4" rx="2" />
          <circle className="tools-stage-dot" cx="130" cy={row.y + 7} r="3" />
          <path className="tools-stage-check" d={`M125.5 ${row.y + 7}l3 3 5.5-6`} />
        </g>
      ))}
    </svg>
  );
}

interface RowProps {
  tool: ToolCatalogEntry;
  title: string;
  detail: string;
  icon: IconName;
  needs?: PiTool["needs"];
  /** Package tools have no friendly title: their name is the title, in mono. */
  mono?: boolean;
  enabled: boolean;
  busy: boolean;
  onToggle: () => void;
}

function ToolRow({ tool, title, detail, icon, needs, mono = false, enabled, busy, onToggle }: RowProps) {
  const reduce = useReducedMotion();
  const [open, setOpen] = useState(false);
  const on = tool.available && enabled;
  const told = tool.description.trim();
  return (
    <li className={`tool-card ${on ? "" : "off"} ${tool.available ? "" : "unavailable"}`}>
      <span className="tool-card-mark" aria-hidden="true"><Icon name={icon} /></span>
      <div className="tool-card-text">
        <span className="tool-card-head">
          <strong className={mono ? "mono" : ""}>{title}</strong>
          {!mono && <code>{tool.name}</code>}
        </span>
        {detail && <span className="tool-card-detail">{detail}</span>}
        {!tool.available && (
          <span className="tool-card-needs">
            {needs ? (
              <>Needs {needs.name}, which isn't installed. Install it with Homebrew (<code>{needs.install}</code>); chats pick it up the next time they start.</>
            ) : tool.unavailableReason}
          </span>
        )}
        {told && told !== detail && (
          <button type="button" className="tool-card-more" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
            <Icon name="chevron" /> What the model is told
          </button>
        )}
        {open && (
          <motion.p
            className="tool-card-model"
            initial={reduce ? false : { opacity: 0, y: -3 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.18 }}
          >
            {told}
          </motion.p>
        )}
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={on}
        aria-label={tool.name}
        className={`toggle ${on ? "on" : ""}`}
        disabled={busy || !tool.available}
        onClick={onToggle}
      >
        <span />
      </button>
    </li>
  );
}

interface Props {
  catalog: ToolCatalogEntry[];
  disabled: string[];
  /** Installed packages, to name a package tool's group by its display name. */
  packages: PackageRecord[];
  /** The agent's name in the app's own copy (Settings › Appearance). */
  agentName?: string;
  onSetDisabled: (disabled: string[]) => Promise<void>;
  /** Opens another Settings section; the list of where the other tools live is shown with it. */
  onOpen?: (section: ToolsElsewhere) => void;
  /** MCP servers is only listed once its actions are wired. */
  mcpAvailable?: boolean;
}

/**
 * Settings › Tools: Pi's own tools and package tools, switched through the tool denylist.
 * WackCode's built-in tools and MCP tools have their own settings (`groupTools` leaves them out),
 * so the page ends by pointing to them.
 */
export function ToolsSection({ catalog, disabled, packages, agentName = "WackCode", onSetDisabled, onOpen, mcpAvailable = false }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const disabledSet = useMemo(() => new Set(disabled), [disabled]);
  const groups = useMemo(() => groupTools(catalog), [catalog]);

  async function toggle(name: string): Promise<void> {
    const next = disabledSet.has(name) ? disabled.filter((item) => item !== name) : [...disabled, name];
    setBusy(true);
    setError(undefined);
    try {
      await onSetDisabled(next);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const builtin = groups.find((group) => group.id === "builtin")?.tools ?? [];
  const packageGroups = groups.filter((group) => group.id !== "builtin");
  const byName = new Map(builtin.map((tool) => [tool.name, tool]));
  const known = (group: PiTool["group"]) => PI_TOOLS.filter((spec) => spec.group === group && byName.has(spec.name));
  const looking = known("look");
  const changing = known("change");
  const otherPi = builtin.filter((tool) => !PI_TOOLS.some((spec) => spec.name === tool.name));

  const isOn = (tool: ToolCatalogEntry) => tool.available && !disabledSet.has(tool.name);
  const usable = groups.flatMap((group) => group.tools).filter((tool) => tool.available);
  const on = usable.filter(isOn).length;
  const pill = catalog.length === 0 ? "Not loaded yet"
    : on === usable.length ? "All on"
    : on === 0 ? "All off"
    : `${on} of ${usable.length} on`;
  const handsOff = changing.length > 0 && changing.every((spec) => !isOn(byName.get(spec.name)!));
  const packageName = (id: string) => packages.find((entry) => entry.source === id)?.displayName ?? (id === "other" ? "Other" : id);
  const elsewhere = ELSEWHERE.filter((item) => item.id !== "mcp" || mcpAvailable);

  let order = 0;
  const row = (tool: ToolCatalogEntry, spec: Omit<RowProps, "tool" | "enabled" | "busy" | "onToggle">) => (
    <ToolRow key={tool.name} tool={tool} {...spec} enabled={!disabledSet.has(tool.name)} busy={busy} onToggle={() => void toggle(tool.name)} />
  );
  const piRows = (specs: PiTool[]) => specs.map((spec) => row(byName.get(spec.name)!, spec));

  return (
    <div className="settings-scroll tools-settings">
      <div className="settings-page">
        <SettingsHero
          label="Tools overview"
          stage={<ToolsStage live={on > 0} />}
          live={catalog.length > 0 && on === usable.length}
          pill={pill}
          title={`The tools ${agentName} works with`}
        >
          <p>
            Every chat reaches for these to look around your project and change it. Switch one off and the model is no
            longer offered it; running chats pick that up from their next message.
          </p>
        </SettingsHero>
        {error && <div className="error-banner" role="alert">{error}</div>}

        {catalog.length === 0 ? (
          <section className="settings-block" style={stagger(++order)}>
            <div className="package-empty tools-empty">
              <span className="package-empty-icon"><Icon name="wrench" /></span>
              <h4>Tools appear after your first chat</h4>
              <p>WackCode learns which tools are available from a running chat. Open or start one, then come back here.</p>
            </div>
          </section>
        ) : (
          <>
            {looking.length > 0 && (
              <section className="settings-block" style={stagger(++order)} aria-labelledby="tools-look">
                <h3 className="settings-block-title" id="tools-look">Looking around</h3>
                <p className="settings-block-sub">Read-only, so these also work in Plan mode.</p>
                <ul className="tool-cards">{piRows(looking)}</ul>
              </section>
            )}
            {changing.length > 0 && (
              <section className="settings-block" style={stagger(++order)} aria-labelledby="tools-change">
                <h3 className="settings-block-title" id="tools-change">Making changes</h3>
                <p className="settings-block-sub">
                  These change your files or run programs on your Mac. Plan mode holds edit and write back, and lets bash run
                  read-only commands only.
                </p>
                <ul className="tool-cards">{piRows(changing)}</ul>
                {handsOff && (
                  <p className="tools-note" role="status">
                    <Icon name="lock" />
                    <span>With all three off, {agentName} can look but not touch, in every chat.</span>
                  </p>
                )}
              </section>
            )}
            {otherPi.length > 0 && (
              <section className="settings-block" style={stagger(++order)} aria-labelledby="tools-pi">
                <h3 className="settings-block-title" id="tools-pi">More from Pi</h3>
                <p className="settings-block-sub">Tools that came with this version of Pi.</p>
                <ul className="tool-cards">
                  {otherPi.map((tool) => row(tool, { title: tool.name, detail: firstSentence(tool.description), icon: "wrench", mono: true }))}
                </ul>
              </section>
            )}
            {packageGroups.map((group) => (
              <section className="settings-block" style={stagger(++order)} key={group.id} aria-label={`${packageName(group.id)} tools`}>
                <h3 className="settings-block-title">{packageName(group.id)}</h3>
                <p className="settings-block-sub">
                  {group.id !== "other" && <><code>{group.id}</code> · </>}
                  From a package you installed. Package tools are held back in Plan mode.
                </p>
                <ul className="tool-cards">
                  {group.tools.map((tool) => row(tool, { title: tool.name, detail: firstSentence(tool.description), icon: "spark", mono: true }))}
                </ul>
              </section>
            ))}
          </>
        )}

        {onOpen && (
          <section className="settings-block" style={stagger(++order)} aria-labelledby="tools-elsewhere">
            <h3 className="settings-block-title" id="tools-elsewhere">More tools, set up elsewhere</h3>
            <p className="settings-block-sub">{agentName}'s own tools have settings of their own.</p>
            <div className="tools-elsewhere">
              {elsewhere.map((item) => (
                <button type="button" className="tools-elsewhere-link" key={item.id} onClick={() => onOpen(item.id)}>
                  <span className="tools-elsewhere-icon" aria-hidden="true"><Icon name={item.icon} /></span>
                  <span className="tools-elsewhere-text">
                    <strong>{item.title}</strong>
                    <span>{item.detail}</span>
                  </span>
                  <Icon name="chevron" />
                </button>
              ))}
            </div>
            <p className="tools-note">
              <Icon name="lock" />
              <span>Asking you questions, keeping a to-do list and handing in a plan are part of the app, so they're always on.</span>
            </p>
          </section>
        )}
      </div>
    </div>
  );
}
