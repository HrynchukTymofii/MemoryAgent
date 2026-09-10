/**
 * Release notes, bundled with the build that they describe.
 *
 * A file rather than a fetch. The app must work with no network — that is the
 * whole premise — and "what changed in the version you are running" is a fact
 * the build already knows. Fetching it would make the one screen that explains
 * an update the one screen an offline user cannot read.
 */
export interface Release {
  version: string;
  /** ISO date. Rendered in the reader's locale, not this string. */
  date: string;
  changes: string[];
}

export const RELEASES: Release[] = [
  {
    version: "0.1.0",
    date: "2026-09-10",
    changes: [
      "Milestones. Words dictated, memories saved, tasks finished and days in " +
        "a row are counted, and the bell tells you when you cross one.",
      "A notification centre behind the bell, with what you have read marked " +
        "as read.",
      "Help lives at the foot of the sidebar: the shortcut you actually have " +
        "bound, and the phrases the app understands.",
      "The sidebar collapses to an icon rail.",
    ],
  },
];
