/**
 * The slash menu's plumbing: what `/` offers, and where to draw it.
 *
 * Split from the component because it is the part with no React in it — a
 * ProseMirror plugin and a list of commands — and because the list is the thing
 * most likely to grow. Every entry here has to survive the Markdown round trip;
 * a block that cannot be written down is a block this document cannot hold.
 */
import { Extension } from "@tiptap/core";
import type { Editor, Range } from "@tiptap/core";
import Suggestion from "@tiptap/suggestion";

export interface Block {
  /** What the row says. Also what it is matched on. */
  title: string;
  /** The Markdown it stands for — the shortcut, shown as a hint. */
  hint: string;
  keywords: string;
  run: (editor: Editor, range: Range) => void;
}

/** Everything Markdown can hold, in the order a person reaches for it. */
export const BLOCKS: Block[] = [
  {
    title: "Text",
    hint: "",
    keywords: "paragraph plain body",
    run: (e, r) => e.chain().focus().deleteRange(r).setNode("paragraph").run(),
  },
  {
    title: "Heading 1",
    hint: "#",
    keywords: "h1 title big section",
    run: (e, r) => e.chain().focus().deleteRange(r).setNode("heading", { level: 1 }).run(),
  },
  {
    title: "Heading 2",
    hint: "##",
    keywords: "h2 subtitle section",
    run: (e, r) => e.chain().focus().deleteRange(r).setNode("heading", { level: 2 }).run(),
  },
  {
    title: "Heading 3",
    hint: "###",
    keywords: "h3 subsection",
    run: (e, r) => e.chain().focus().deleteRange(r).setNode("heading", { level: 3 }).run(),
  },
  {
    title: "Bulleted list",
    hint: "-",
    keywords: "ul bullet point unordered",
    run: (e, r) => e.chain().focus().deleteRange(r).toggleBulletList().run(),
  },
  {
    title: "Numbered list",
    hint: "1.",
    keywords: "ol ordered number steps",
    run: (e, r) => e.chain().focus().deleteRange(r).toggleOrderedList().run(),
  },
  {
    title: "To-do list",
    hint: "- [ ]",
    keywords: "task checkbox check tick",
    run: (e, r) => e.chain().focus().deleteRange(r).toggleTaskList().run(),
  },
  {
    title: "Quote",
    hint: ">",
    keywords: "blockquote cite",
    run: (e, r) => e.chain().focus().deleteRange(r).toggleBlockquote().run(),
  },
  {
    title: "Code",
    hint: "```",
    keywords: "codeblock snippet monospace",
    run: (e, r) => e.chain().focus().deleteRange(r).toggleCodeBlock().run(),
  },
  {
    title: "Divider",
    hint: "---",
    keywords: "hr rule line separator break",
    run: (e, r) => e.chain().focus().deleteRange(r).setHorizontalRule().run(),
  },
];

export function search(query: string): Block[] {
  const q = query.trim().toLowerCase();
  if (!q) return BLOCKS;
  return BLOCKS.filter(
    (b) => b.title.toLowerCase().includes(q) || b.keywords.includes(q),
  );
}

/** What the component needs to draw the menu, or `null` to put it away. */
export interface SlashState {
  items: Block[];
  range: Range;
  /** Where the `/` is on screen, so the menu can sit under it. */
  rect: DOMRect;
}

/**
 * The `/` trigger.
 *
 * Only at the start of a block or after a space — a slash inside a URL or a
 * date is a slash, and a menu that opens on `and/or` is a menu in the way.
 */
export function slashMenu(handlers: {
  onChange: (state: SlashState | null) => void;
  /** Returns true if the menu took the key. */
  onKey: (event: KeyboardEvent) => boolean;
}) {
  return Extension.create({
    name: "slashMenu",
    addProseMirrorPlugins() {
      return [
        Suggestion({
          editor: this.editor,
          char: "/",
          startOfLine: false,
          allowSpaces: false,
          // A slash inside a URL or a date is a slash. The menu opens only at
          // the start of a block or after a space.
          allowedPrefixes: [" "],
          items: ({ query }) => search(query),
          command: ({ editor, range, props }) => (props as Block).run(editor, range),
          render: () => ({
            onStart: (props) => {
              const rect = props.clientRect?.();
              if (rect) {
                handlers.onChange({
                  items: props.items as Block[],
                  range: props.range,
                  rect,
                });
              }
            },
            onUpdate: (props) => {
              const rect = props.clientRect?.();
              if (rect) {
                handlers.onChange({
                  items: props.items as Block[],
                  range: props.range,
                  rect,
                });
              }
            },
            onKeyDown: (props) => handlers.onKey(props.event),
            onExit: () => handlers.onChange(null),
          }),
        }),
      ];
    },
  });
}
