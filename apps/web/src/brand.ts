/**
 * Everything about the site that changes when the product is named.
 *
 * The name is not settled yet, so it is read from here rather than typed into
 * forty places — including the two legal pages, where a half-renamed product is
 * worse than an unnamed one. Change these four values and the site is renamed;
 * the window titles in `index.html`, `terms/index.html` and `privacy/index.html`
 * are the only copies outside this file, because a document title has to exist
 * before any script runs.
 */
export const BRAND = {
  name: "Memory OS",
  /** The company or person the legal pages are an agreement with. */
  entity: "Memory OS",
  /** Where a person writes to about either legal page. */
  contact: "hello@memoryos.app",
  /** Last substantive change to the terms and the policy. */
  updated: "10 September 2026",
} as const;
