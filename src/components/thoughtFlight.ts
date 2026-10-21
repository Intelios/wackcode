import { Component, type ReactNode } from "react";

/**
 * The hand-off of a reasoning row's duck to the thought mark in an Explored head. Chips fly with
 * motion's shared `layoutId`, but a reasoning row usually leaves in the same commit that grows
 * the group above it, and motion measures a leaving node mid-commit, so the mark set off from
 * where the duck would have been pushed to. Here the ducks are measured before React touches
 * the DOM (`ThoughtFlightScope`, a class for `getSnapshotBeforeUpdate`), and the mark mounting
 * in that commit flies from the last box measured.
 *
 * Boxes are in the transcript scroller's content space, so follow-scrolling between the
 * snapshot and the mark's mount doesn't skew the flight.
 */

interface Box { x: number; y: number }

const ducks = new Map<string, HTMLElement>();
let snapshot = new Map<string, Box>();

export function contentBox(element: HTMLElement): Box {
  const rect = element.getBoundingClientRect();
  const scroller = element.closest(".conversation-scroll");
  const frame = scroller?.getBoundingClientRect();
  return {
    x: rect.left - (frame?.left ?? 0) + (scroller?.scrollLeft ?? 0),
    y: rect.top - (frame?.top ?? 0) + (scroller?.scrollTop ?? 0)
  };
}

/** A duck that may fly, while it is on screen. Returns the unregister. */
export function registerDuck(id: string, element: HTMLElement): () => void {
  ducks.set(id, element);
  return () => {
    if (ducks.get(id) === element) ducks.delete(id);
  };
}

/** Where the duck for `id` was just before this commit, once: a mark flies from it only on landing. */
export function takeDuckBox(id: string): Box | undefined {
  const box = snapshot.get(id);
  snapshot.delete(id);
  return box;
}

/** Measures every duck before each transcript commit. */
export class ThoughtFlightScope extends Component<{ children: ReactNode }> {
  getSnapshotBeforeUpdate() {
    const next = new Map<string, Box>();
    for (const [id, element] of ducks) if (element.isConnected) next.set(id, contentBox(element));
    snapshot = next;
    return null;
  }

  componentDidUpdate() {}

  render() {
    return this.props.children;
  }
}
