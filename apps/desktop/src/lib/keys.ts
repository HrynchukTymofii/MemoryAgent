/**
 * Windows virtual-key codes, and the names people read.
 *
 * The token vocabulary here mirrors `Chord::parse` in `config.rs`; the two must
 * agree, or a recorded shortcut is rejected by the same backend that just
 * watched the user press it. In its own module because the recorder and the
 * diagnostics both read from it.
 */

/* ------------------------------------------------------------ key naming */

/**
 * Modifier bits, paired with the token `Chord::parse` accepts and the label a
 * person reads. Mirrors `bits` in config.rs — this ordering is also the order a
 * chord is written in, so "Ctrl + Win" never comes out as "Win + Ctrl".
 */
export const MODS: { bit: number; spec: string; label: string }[] = [
  { bit: 1 << 0, spec: "lctrl", label: "Left Ctrl" },
  { bit: 1 << 1, spec: "rctrl", label: "Right Ctrl" },
  { bit: 1 << 2, spec: "lalt", label: "Left Alt" },
  { bit: 1 << 3, spec: "ralt", label: "Right Alt" },
  { bit: 1 << 4, spec: "lshift", label: "Left Shift" },
  { bit: 1 << 5, spec: "rshift", label: "Right Shift" },
  { bit: 1 << 6, spec: "lwin", label: "Left Win" },
  { bit: 1 << 7, spec: "rwin", label: "Right Win" },
];

const NAMED_KEYS: Record<number, [string, string]> = {
  0x20: ["space", "Space"],
  0x09: ["tab", "Tab"],
  0x0d: ["enter", "Enter"],
  0x1b: ["esc", "Esc"],
  0x14: ["capslock", "Caps Lock"],
};

const MOD_VK: Record<number, string> = {
  0xa0: "lshift", 0xa1: "rshift", 0xa2: "lctrl", 0xa3: "rctrl",
  0xa4: "lalt", 0xa5: "ralt", 0x5b: "lwin", 0x5c: "rwin",
};

/** Virtual-key code → the token config understands, or null if unbindable. */
export function vkSpec(vk: number): string | null {
  if (vk >= 0x70 && vk <= 0x87) return `f${vk - 0x70 + 1}`;
  if (vk >= 0x41 && vk <= 0x5a) return String.fromCharCode(vk).toLowerCase();
  if (vk >= 0x30 && vk <= 0x39) return String.fromCharCode(vk);
  return NAMED_KEYS[vk]?.[0] ?? null;
}

/** A config token → the label shown to a person. */
export function specLabel(spec: string): string {
  return spec
    .split("+")
    .map((raw) => {
      const t = raw.trim().toLowerCase();
      const mod = MODS.find((m) => m.spec === t);
      if (mod) return mod.label;
      const plain: Record<string, string> = {
        ctrl: "Ctrl", control: "Ctrl", alt: "Alt",
        shift: "Shift", win: "Win", super: "Win", meta: "Win",
      };
      if (plain[t]) return plain[t];
      if (/^f\d+$/.test(t)) return t.toUpperCase();
      const named = Object.values(NAMED_KEYS).find(([s]) => s === t);
      if (named) return named[1];
      if (t.length === 1) return t.toUpperCase();
      return raw;
    })
    .join(" + ");
}

export function vkName(vk: number): string {
  if (!vk) return "—";
  const hex = `0x${vk.toString(16).toUpperCase()}`;
  const modSpec = MOD_VK[vk];
  if (modSpec) return `${specLabel(modSpec)} · ${hex}`;
  const s = vkSpec(vk);
  return s ? `${specLabel(s)} · ${hex}` : hex;
}

/**
 * Build a config token from the peak of what was held.
 *
 * Returns "" for a key that cannot be bound, which the caller reports rather
 * than saving a spec the backend would reject.
 */
export function buildSpec(mods: number, vk: number): string {
  const parts = MODS.filter((m) => (mods & m.bit) !== 0).map((m) => m.spec);
  const key = vk ? vkSpec(vk) : null;
  if (vk && !key) return "";
  if (key) parts.push(key);
  return parts.join("+");
}
