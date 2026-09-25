import type { SVGProps } from "react";

export type IconName = "plus" | "settings" | "folder" | "git" | "refresh" | "panel" | "copy" | "archive" | "send" | "stop" | "chevron" | "back" | "key" | "trash" | "spark" | "branch" | "more" | "check" | "checklist" | "terminal" | "file" | "pencil" | "external" | "brain" | "search" | "wrench" | "question" | "image" | "close" | "rewind" | "agents" | "flame" | "palette" | "plug" | "book" | "comment" | "commit" | "push" | "pullRequest";

const paths: Record<IconName, React.ReactNode> = {
  plus: <><path d="M12 5v14M5 12h14" /></>,
  settings: <><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.83 2.83-.06-.06A1.7 1.7 0 0 0 15 19.4a1.7 1.7 0 0 0-1 .6 1.7 1.7 0 0 0-.4 1.1V21h-4v-.1A1.7 1.7 0 0 0 8.6 19.4a1.7 1.7 0 0 0-1.88.34l-.06.06-2.83-2.83.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-.6-1 1.7 1.7 0 0 0-1.1-.4H3v-4h.1A1.7 1.7 0 0 0 4.6 8.6a1.7 1.7 0 0 0-.34-1.88l-.06-.06 2.83-2.83.06.06A1.7 1.7 0 0 0 9 4.6a1.7 1.7 0 0 0 1-.6 1.7 1.7 0 0 0 .4-1.1V3h4v.1A1.7 1.7 0 0 0 15.4 4a1.7 1.7 0 0 0 1.88-.34l.06-.06 2.83 2.83-.06.06A1.7 1.7 0 0 0 19.4 9c.13.37.34.7.6 1 .3.27.67.4 1.1.4h.1v4h-.1A1.7 1.7 0 0 0 19.4 15Z" /></>,
  folder: <path d="M3 6.5h6l2 2h10v10.5H3z" />,
  search: <><circle cx="11" cy="11" r="7" /><path d="m20 20-3.5-3.5" /></>,
  wrench: <path d="M15.6 3.6a5.5 5.5 0 0 0-7.1 6.7L3.7 15.1a2 2 0 0 0 0 2.8l2.4 2.4a2 2 0 0 0 2.8 0l4.8-4.8a5.5 5.5 0 0 0 6.7-7.1l-3.1 3.1-3-.6-.6-3z" />,
  git: <><circle cx="6" cy="5" r="2" /><circle cx="18" cy="7" r="2" /><circle cx="8" cy="19" r="2" /><path d="M6 7v3c0 3 2 3 2 7M8 13c0-4 8-1 10-4" /></>,
  refresh: <><path d="M20 7v5h-5" /><path d="M19 12a7 7 0 1 0-2 5" /></>,
  panel: <><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M15 4v16" /></>,
  copy: <><rect x="8" y="8" width="11" height="11" rx="2" /><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3" /></>,
  archive: <><path d="M4 7h16v13H4zM3 4h18v3H3z" /><path d="M9 11h6" /></>,
  send: <><path d="m4 4 17 8-17 8 3-8z" /><path d="M7 12h14" /></>,
  stop: <rect x="6" y="6" width="12" height="12" rx="2" />,
  chevron: <path d="m9 18 6-6-6-6" />,
  back: <path d="m15 18-6-6 6-6" />,
  key: <><circle cx="8" cy="15" r="4" /><path d="m11 12 9-9M15 8l3 3M17 6l2 2" /></>,
  trash: <><path d="M4 7h16M9 7V4h6v3M7 7l1 13h8l1-13" /></>,
  spark: <><path d="m12 3 1.2 4.8L18 9l-4.8 1.2L12 15l-1.2-4.8L6 9l4.8-1.2z" /><path d="m18 15 .7 2.3L21 18l-2.3.7L18 21l-.7-2.3L15 18l2.3-.7z" /></>,
  branch: <><circle cx="7" cy="5" r="2" /><circle cx="7" cy="19" r="2" /><circle cx="17" cy="8" r="2" /><path d="M7 7v10M9 15c4 0 6-2 6-5" /></>,
  more: <><circle cx="5" cy="12" r="1.4" /><circle cx="12" cy="12" r="1.4" /><circle cx="19" cy="12" r="1.4" /></>,
  check: <path d="m4.5 12.5 5 5 10-11" />,
  checklist: <><path d="M9 6h11M9 12h11M9 18h11" /><path d="m3 5.5 1.5 1.5L7.5 4" /><path d="m3 11.5 1.5 1.5L7.5 10" /><path d="m3 17.5 1.5 1.5L7.5 16" /></>,
  terminal: <><rect x="3" y="4" width="18" height="16" rx="2" /><path d="m7 9 3 3-3 3M12.5 15H17" /></>,
  file: <><path d="M6 2.5h8l4 4V21.5H6z" /><path d="M14 2.5V7h4" /></>,
  pencil: <><path d="M4 20l1-4L16.5 4.5a2.1 2.1 0 0 1 3 3L8 19z" /><path d="m14.5 6.5 3 3" /></>,
  external: <><path d="M14 4h6v6" /><path d="M20 4 10.5 13.5" /><path d="M19 14v5.5h-14.5V9.5H10" /></>,
  brain: (
    <g transform="translate(0, 1.5)">
      <path d="M12 5a3 3 0 1 0-5.997.125 4 4 0 0 0-2.526 5.77 4 4 0 0 0 .556 6.588A4 4 0 1 0 12 18Z" />
      <path d="M12 5a3 3 0 1 1 5.997.125 4 4 0 0 1 2.526 5.77 4 4 0 0 1-.556 6.588A4 4 0 1 1 12 18Z" />
      <path d="M12 5v13" />
      <path d="M15.5 13a3.5 3.5 0 0 0-3.5 3.5" />
      <path d="M8.5 13A3.5 3.5 0 0 1 12 16.5" />
    </g>
  ),
  image: <><rect x="3" y="4" width="18" height="16" rx="2" /><circle cx="9" cy="10" r="1.8" /><path d="m21 16-5-5-9 9" /></>,
  close: <path d="M6 6l12 12M18 6 6 18" />,
  rewind: <><path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3" /><path d="M4.5 4.5v4.2h4.2" /></>,
  agents: <><circle cx="9" cy="8" r="3" /><path d="M3.5 19.5a5.5 5.5 0 0 1 11 0" /><circle cx="17" cy="9" r="2.4" /><path d="M16.2 14.1A4.5 4.5 0 0 1 21 18.6" /></>,
  question: <><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" /><path d="M9.1 9a3 3 0 0 1 5.8 1c0 2-3 3-3 3" /><path d="M12 17h.01" /></>,
  palette: <><path d="M12 3a9 9 0 0 0 0 18c1.1 0 1.8-.8 1.8-1.7 0-.5-.2-.9-.5-1.2-.3-.3-.5-.7-.5-1.2 0-.9.8-1.7 1.8-1.7H17a4 4 0 0 0 4-4C21 6.6 17 3 12 3Z" /><circle cx="7.5" cy="11" r="1.2" /><circle cx="10" cy="7" r="1.2" /><circle cx="15" cy="7" r="1.2" /></>,
  plug: <><path d="M9 3v5M15 3v5" /><path d="M6 8h12v3a6 6 0 0 1-12 0z" /><path d="M12 17v4" /></>,
  book: <><path d="M5 5a2 2 0 0 1 2-2h12v14H7a2 2 0 0 0-2 2z" /><path d="M5 19a2 2 0 0 0 2 2h12v-4" /><path d="M9 7.5h6" /></>,
  flame: <path d="M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.07-2.14-.22-4.05 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.15.43-2.29 1-3a2.5 2.5 0 0 0 2.5 2.5z" />,
  comment: <path d="M21 11.5a8.4 8.4 0 0 1-9 8.4 8.6 8.6 0 0 1-3.2-.6L3 21l1.7-5.8A8.4 8.4 0 1 1 21 11.5Z" />,
  commit: <><circle cx="12" cy="12" r="3.5" /><path d="M3.5 12h5M15.5 12h5" /></>,
  push: <><path d="M12 19V5" /><path d="m6 11 6-6 6 6" /><path d="M4.5 21.5h15" /></>,
  pullRequest: <><circle cx="6" cy="5.5" r="2" /><circle cx="6" cy="18.5" r="2" /><path d="M6 7.5v9" /><circle cx="18" cy="8.5" r="2" /><path d="M18 10.5c0 4-4 3.5-7 5.5" /></>
};

export function Icon({ name, ...props }: SVGProps<SVGSVGElement> & { name: IconName }) {
  return <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" {...props}>{paths[name]}</svg>;
}
