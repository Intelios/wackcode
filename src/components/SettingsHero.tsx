import type { ReactNode } from "react";

/** A card's place in a `.settings-page`'s staggered entrance: the hero is 0, the cards follow. */
export const stagger = (index: number) => ({ "--i": index } as React.CSSProperties);

interface Props {
  /** The region's accessible name, e.g. "Tools overview". */
  label: string;
  /** The page's own little illustration, an `.settings-stage` SVG. Pure decoration. */
  stage: ReactNode;
  /** One or two words of status; the pill says in words what the stage shows. */
  pill: ReactNode;
  /** Lights the border and the pill with the accent. The stage decides what it animates. */
  live: boolean;
  title: ReactNode;
  /** The page's one or two sentences, as `<p>`s. */
  children: ReactNode;
  /** Top right: the page's main switch or button. */
  action?: ReactNode;
}

/**
 * The illustrated header of a Settings page in the finished shape (Tools, MCP servers, Memory):
 * a stage on the left, a status pill, a title and a sentence or two, and an optional action. It
 * sits first in a `.settings-page` column. Computer use and Prompts predate it and keep their own
 * copies of the same look.
 */
export function SettingsHero({ label, stage, pill, live, title, children, action }: Props) {
  return (
    <section className={`settings-hero ${live ? "live" : ""}`} style={stagger(0)} aria-label={label}>
      {stage}
      <div className="settings-hero-text">
        <span className={`settings-pill ${live ? "live" : ""}`}><i aria-hidden="true" />{pill}</span>
        <h3>{title}</h3>
        {children}
      </div>
      {action && <div className="settings-hero-action">{action}</div>}
    </section>
  );
}
