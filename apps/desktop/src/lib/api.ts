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
  signed_in: boolean;
  email: string | null;
  display_name: string | null;
  signed_in_at: string | null;
}

export interface TaskRow {
  id: string;
  title: string;
  /** The memory the task was spoken alongside, if there was one. */
  about: string | null;
  due_at: string | null;
  done: boolean;
  created_at: string;
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
  tasks: (limit: number) => invoke<TaskRow[]>("tasks", { limit }),
  setTaskDone: (id: string, done: boolean) =>
    invoke<void>("set_task_done", { id, done }),
  openTaskCount: () => invoke<number>("open_task_count"),
  account: () => invoke<Account>("account"),
  /** Resolves when the browser round trip finishes — which can take minutes. */
  signIn: () => invoke<Account>("sign_in"),
  signOut: () => invoke<Account>("sign_out"),
  /** Resolves to what was opened, or `null` for an item with no source. */
  openItem: (id: string) => invoke<string | null>("open_item", { id }),
};

/** The free plan's weekly allowance (§16). */
export const CAPTURE_LIMIT = 50;
