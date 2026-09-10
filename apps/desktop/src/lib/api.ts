/**
 * Every call into the Rust side, in one place.
 *
 * Typed wrappers rather than raw `invoke` at each call site: the command names
 * and argument shapes are a contract with `main.rs`, and a typo in one of them
 * fails at run time inside a `catch` that renders as an empty panel. Here, at
 * least, there is a single list to check against the `invoke_handler`.
 */
import { invoke } from "@tauri-apps/api/core";

export interface LatencyReport {
  count: number;
  p50_ms: number;
  p95_ms: number;
  worst_ms: number;
  within_budget: boolean;
}

export interface HookStats {
  events: number;
  mod_events: number;
  injected_events: number;
  held_mods: number;
  held_key: number;
  last_vk: number;
  engaged: boolean;
}

export interface MicStatus {
  available: boolean;
  device: string;
  input_rate: number;
  channels: number;
  level: number;
  buffered_secs: number;
}

export type ModelState = "loading" | "ready" | "missing" | "failed";

export interface SttStatus {
  state: ModelState;
  detail: string;
}

export interface EmbedStatus {
  state: ModelState;
  detail: string;
  embedded: number;
  pending: number;
  model_id: string;
  dim: number;
}

export interface RouterStatus {
  state: ModelState;
  detail: string;
}

/** How routing is going, from the correction log. */
export interface RoutingStats {
  total: number;
  /** Routed by the grammar alone, with no model. */
  tier0: number;
  /** Escalated to the local router and routed there. */
  tier1: number;
  /** Neither tier could route it. */
  unrouted: number;
  /** Questions answered, either way. */
  answered: number;
  /** ...of which the top suggestion was already right. */
  accepted: number;
}

export interface Settings {
  hotkey: string;
  hold_threshold_ms: number;
  debug_keys: boolean;
  active_chord: string;
  /** Keep a small pill on screen when nothing is being captured. */
  idle_pill: boolean;
}

export interface Item {
  id: string;
  title: string;
  snippet: string;
  collection: string | null;
  source_url: string | null;
  captured_at: string;
  access_count: number;
  /** Search results only: the fused score, and which retrievers found it. */
  score: number | null;
  why: string | null;
}

export interface CollectionRow {
  id: string;
  name: string;
  path: string;
  depth: number;
  items: number;
}

/** Who is signed in, and whether signing in is offered by this build at all. */
export interface Account {
  /** False when no provider is configured — the section stays hidden. */
  available: boolean;
  /** Email sign-in needs the API, so it can be offered when Google is not. */
  email_available: boolean;
  signed_in: boolean;
  email: string | null;
  display_name: string | null;
  signed_in_at: string | null;
}

export interface TaskRow {
  id: string;
  title: string;
  /** When it is due, if a deadline was ever put on it. RFC 3339. */
  due_at: string | null;
  done: boolean;
  created_at: string;
}

/** The one document (ADR-0010). */
export interface NoteRow {
  id: string;
  title: string;
  /** Markdown. What the editor loads and what it hands back. */
  body: string;
  updated_at: string;
  /** When a person last edited it, as opposed to a capture growing it. */
  edited_at: string | null;
  /** How many captures have been folded in. */
  sources: number;
}

/** One row in the notification centre. */
export interface Notification {
  id: string;
  /** `milestone` — earned. `nudge` — worth knowing. `alert` — wrong. */
  kind: "milestone" | "nudge" | "alert";
  /** The achievement behind it. Absent on alerts, which are not earned. */
  code: string | null;
  title: string;
  body: string;
  /** Page to open on click, when there is somewhere to go. */
  goto: string | null;
  created_at: string;
  read: boolean;
}

/** One person the user brought in, as the API is willing to describe them. */
export interface ReferralRow {
  /** Masked by the server: `t…@gmail.com`. A count and a status, not an address. */
  who: string;
  /** `pending` until the referee has used the app enough, then `qualified`. */
  status: string;
  created_at: string;
  qualified_at: string | null;
}

export interface ReferralStatus {
  code: string;
  link: string;
  qualify_words: number;
  months_per_referral: number;
  referrals: ReferralRow[];
  months_earned: number;
  pro_until: string | null;
  /** Whether this account may still be referred by somebody else. */
  can_apply: boolean;
  applied_code: string | null;
}

/**
 * The referral screen's whole state, including the two reasons it cannot show
 * anything: this build has no API, or nobody is signed in. Neither is an error.
 */
export interface Referrals {
  available: boolean;
  signed_in: boolean;
  status: ReferralStatus | null;
  error: string | null;
}

export interface Invited {
  sent: string[];
  failed: string[];
}

/** The plan, as this machine last understood it. */
export interface Entitlement {
  pro_until: string | null;
  months_earned: number;
}

export interface LibrarySummary {
  items: number;
  collections: number;
  this_week: number;
}

export const api = {
  latency: () => invoke<LatencyReport>("latency_report"),
  hookStats: () => invoke<HookStats>("hook_stats"),
  mic: () => invoke<MicStatus>("mic_status"),
  stt: () => invoke<SttStatus>("stt_status"),
  embed: () => invoke<EmbedStatus>("embed_status"),
  router: () => invoke<RouterStatus>("router_status"),
  routingStats: () => invoke<RoutingStats>("routing_stats"),
  /** Resolves to the path the log was written to. */
  exportCommandLog: () => invoke<string>("export_command_log"),
  /** Resolves to how many commands were erased. */
  forgetCommandLog: () => invoke<number>("forget_command_log"),
  settings: () => invoke<Settings>("get_settings"),
  setHotkey: (spec: string, holdThresholdMs: number) =>
    invoke<Settings>("set_hotkey", { spec, holdThresholdMs }),
  setIdlePill: (enabled: boolean) => invoke<Settings>("set_idle_pill", { enabled }),
  captureCount: () => invoke<number>("capture_count"),
  summary: () => invoke<LibrarySummary>("library_summary"),
  recent: (limit: number) => invoke<Item[]>("recent", { limit }),
  items: (collection: string | null, limit: number, offset: number) =>
    invoke<Item[]>("items", { collection, limit, offset }),
  search: (query: string, limit: number) => invoke<Item[]>("search", { query, limit }),
  collections: () => invoke<CollectionRow[]>("collections"),
  /** Resolves to the new collection's path. `parent` is a path, or null for a root. */
  createCollection: (name: string, parent: string | null) =>
    invoke<string>("create_collection", { name, parent }),
  /** Resolves to the path it now has — which its children now sit under. */
  renameCollection: (id: string, name: string) =>
    invoke<string>("rename_collection", { id, name }),
  /** Children go with it; the memories inside come loose rather than dying. */
  deleteCollection: (id: string) => invoke<void>("delete_collection", { id }),
  /** The one document, or null if nothing has started it. */
  book: () => invoke<NoteRow | null>("book"),
  /** Begin it by hand. Idempotent — an existing document comes back as it is. */
  startBook: () => invoke<NoteRow>("start_book"),
  saveNote: (id: string, body: string) => invoke<void>("save_note", { id, body }),
  /** Follow a link out of a note, into the real browser. */
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  /** Memories filed here or anywhere below — what a delete would unfile. */
  collectionSize: (path: string) => invoke<number>("collection_size", { path }),
  tasks: (limit: number) => invoke<TaskRow[]>("tasks", { limit }),
  setTaskDone: (id: string, done: boolean) =>
    invoke<void>("set_task_done", { id, done }),
  renameTask: (id: string, title: string) => invoke<void>("rename_task", { id, title }),
  /** `null` clears the deadline. */
  setTaskDue: (id: string, dueAt: string | null) =>
    invoke<void>("set_task_due", { id, dueAt }),
  deleteTask: (id: string) => invoke<void>("delete_task", { id }),
  openTaskCount: () => invoke<number>("open_task_count"),
  account: () => invoke<Account>("account"),
  /** Resolves when the browser round trip finishes — which can take minutes. */
  signIn: () => invoke<Account>("sign_in"),
  signOut: () => invoke<Account>("sign_out"),
  emailStart: (email: string) => invoke<void>("email_start", { email }),
  emailVerify: (email: string, code: string) =>
    invoke<Account>("email_verify", { email, code }),
  signInPromptSeen: () => invoke<boolean>("sign_in_prompt_seen"),
  dismissSignInPrompt: () => invoke<void>("dismiss_sign_in_prompt"),
  /** Resolves to what was opened, or `null` for an item with no source. */
  openItem: (id: string) => invoke<string | null>("open_item", { id }),
  notifications: (limit: number) => invoke<Notification[]>("notifications", { limit }),
  unreadNotifications: () => invoke<number>("unread_notifications"),
  /** Resolves to how many were still unread. */
  markNotificationsRead: () => invoke<number>("mark_notifications_read"),
  dismissNotification: (id: string) => invoke<void>("dismiss_notification", { id }),
  dismissAllNotifications: () => invoke<number>("dismiss_all_notifications"),
  referralStatus: () => invoke<Referrals>("referral_status"),
  applyReferral: (code: string) => invoke<ReferralStatus>("apply_referral", { code }),
  sendInvites: (emails: string[]) => invoke<Invited>("send_invites", { emails }),
  /** Read from the local cache; never touches the network. */
  entitlement: () => invoke<Entitlement>("entitlement"),
  /** Re-checks the plan with the API, and pays out a referral if one is due. */
  refreshEntitlement: () => invoke<Entitlement>("refresh_entitlement"),
};

/** Whether a cached entitlement is live right now. */
export function isPro(e: Entitlement | null): boolean {
  return e?.pro_until != null && new Date(e.pro_until) > new Date();
}

/**
 * The free plan's weekly allowance (§16).
 *
 * Still a constant, and still the right one to draw against: Pro has no limit
 * at all rather than a larger one, so the meter is either this bar or the
 * sentence that replaces it. See `memos-license`.
 */
export const CAPTURE_LIMIT = 50;
