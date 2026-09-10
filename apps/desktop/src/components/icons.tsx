/**
 * The navigation icons.
 *
 * Line drawings on the same 18-unit grid and the same stroke as the title bar,
 * so the two sets of controls in the chrome look like one family. They carry no
 * background of their own: the row behind them is what shows selection, and an
 * icon that also has a raised chip around it makes every row look selected.
 *
 * They have to work at two sizes of the same list — with a label beside them,
 * and alone in the collapsed rail — so each one is legible with nothing else to
 * read: a house, a list, a folder, a ticked circle.
 */
function Icon({ children }: { children: React.ReactNode }) {
  return (
    <svg viewBox="0 0 18 18" className="ic" aria-hidden="true">
      {children}
    </svg>
  );
}

export function HomeIcon() {
  return (
    <Icon>
      <path d="M2.9 7.4 9 2.6l6.1 4.8v6.9a1.3 1.3 0 0 1-1.3 1.3H4.2a1.3 1.3 0 0 1-1.3-1.3z" />
      <path d="M7 15.6v-4.3h4v4.3" />
    </Icon>
  );
}

export function LibraryIcon() {
  return (
    <Icon>
      <path d="M3.3 4.9h11.4" />
      <path d="M3.3 9h11.4" />
      <path d="M3.3 13.1h7.2" />
    </Icon>
  );
}

export function CollectionsIcon() {
  return (
    <Icon>
      <path d="M2.6 5.6a1.5 1.5 0 0 1 1.5-1.5h2.8l1.6 1.9h5.4a1.5 1.5 0 0 1 1.5 1.5v5.9a1.5 1.5 0 0 1-1.5 1.5H4.1a1.5 1.5 0 0 1-1.5-1.5z" />
    </Icon>
  );
}

export function TasksIcon() {
  return (
    <Icon>
      <circle cx="9" cy="9" r="6.5" />
      <path d="M5.9 9.2 8 11.3l4.1-4.4" />
    </Icon>
  );
}

export function DictionaryIcon() {
  return (
    <Icon>
      <path d="M9 5.2a2 2 0 0 0-1.8-1.1H3.1v9.6h4.4A1.7 1.7 0 0 1 9 14.9z" />
      <path d="M9 5.2a2 2 0 0 1 1.8-1.1h4.1v9.6h-4.4A1.7 1.7 0 0 0 9 14.9z" />
    </Icon>
  );
}

export function HistoryIcon() {
  return (
    <Icon>
      <path d="M3 9a6 6 0 1 0 1.9-4.4" />
      <path d="M2.7 3v3h3" />
      <path d="M9 5.7V9l2.4 1.5" />
    </Icon>
  );
}

/* Sliders rather than a cog: a cog at 18px with a 1.5 stroke is a grey blob,
   and the rail shows it with nothing beside it to explain what it was. */
export function SettingsIcon() {
  return (
    <Icon>
      <path d="M3 6.3h2.6" />
      <path d="M9.4 6.3h5.6" />
      <path d="M3 11.7h3.6" />
      <path d="M10.4 11.7h4.6" />
      <circle cx="7.5" cy="6.3" r="1.7" />
      <circle cx="8.5" cy="11.7" r="1.7" />
    </Icon>
  );
}

/* ------------------------------------------------------------------ help */
/* The same grid and stroke as the nav marks above, because they appear in the
   same column: Help is the last row of the sidebar, and its menu sits directly
   over the list these came from. */

export function HelpIcon() {
  return (
    <Icon>
      <circle cx="9" cy="9" r="6.9" />
      <path d="M7 7.1a2 2 0 1 1 2.6 1.9c-.5.2-.8.6-.8 1.1v.5" />
      <path d="M9 13.1v.1" />
    </Icon>
  );
}

export function MicIcon() {
  return (
    <Icon>
      <rect x="6.7" y="1.9" width="4.6" height="8.4" rx="2.3" />
      <path d="M3.9 8.5a5.1 5.1 0 0 0 10.2 0" />
      <path d="M9 13.6v2.5" />
    </Icon>
  );
}

export function GlobeIcon() {
  return (
    <Icon>
      <circle cx="9" cy="9" r="6.9" />
      <path d="M2.1 9h13.8" />
      <path d="M9 2.1a10.6 10.6 0 0 1 0 13.8 10.6 10.6 0 0 1 0-13.8z" />
    </Icon>
  );
}

export function BookIcon() {
  return (
    <Icon>
      <path d="M3 3.4h4.1A2.2 2.2 0 0 1 9 4.6v10a1.7 1.7 0 0 0-1.5-.9H3z" />
      <path d="M15 3.4h-4.1A2.2 2.2 0 0 0 9 4.6v10a1.7 1.7 0 0 1 1.5-.9H15z" />
    </Icon>
  );
}

export function MailIcon() {
  return (
    <Icon>
      <rect x="2.1" y="4.2" width="13.8" height="9.6" rx="1.9" />
      <path d="M2.6 5.4L9 9.9l6.4-4.5" />
    </Icon>
  );
}

export function BugIcon() {
  return (
    <Icon>
      <rect x="5.4" y="5.9" width="7.2" height="8.6" rx="3.6" />
      <path d="M6.9 5.4a2.1 2.1 0 0 1 4.2 0" />
      <path d="M5.4 8.4H2.9M5.4 12h-2.2M12.6 8.4h2.5M12.6 12h2.2" />
    </Icon>
  );
}

export function StarIcon() {
  return (
    <Icon>
      <path d="M9 2.4l2 4.2 4.5.6-3.3 3.2.8 4.5L9 12.8l-4 2.1.8-4.5L2.5 7.2 7 6.6z" />
    </Icon>
  );
}

export function SparkIcon() {
  return (
    <Icon>
      <path d="M7.6 2.3l1.5 3.6 3.6 1.5-3.6 1.5-1.5 3.6-1.5-3.6L2.5 7.4l3.6-1.5z" />
      <path d="M13.3 11.2l.7 1.7 1.7.7-1.7.7-.7 1.7-.7-1.7-1.7-.7 1.7-.7z" />
    </Icon>
  );
}

export function GiftIcon() {
  return (
    <Icon>
      <rect x="2.5" y="7.6" width="13" height="7.9" rx="1.6" />
      <path d="M1.8 5.1h14.4v2.5H1.8zM9 5.1v10.4" />
      <path d="M9 5.1S8.2 2.2 6.4 2.2a1.5 1.5 0 0 0 0 2.9zM9 5.1s.8-2.9 2.6-2.9a1.5 1.5 0 0 1 0 2.9z" />
    </Icon>
  );
}
