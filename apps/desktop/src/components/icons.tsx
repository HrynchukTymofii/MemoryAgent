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
