import { Group, Sheet } from "./Shortcuts";

/**
 * What you can say.
 *
 * Every phrase below is one the grammar actually matches — see `PATTERNS` in
 * `crates/memos-agent/src/grammar.rs`. This list is written from that one, not
 * invented: a cheat sheet that lists a phrase the parser has never heard of
 * teaches people the app is unreliable when in fact they were misled.
 *
 * Each intent shows two or three openings rather than all of them. The grammar
 * knows forty ways to say "save this"; printing forty is a reference manual,
 * and what someone opening this needs is the shape.
 */
export function VoiceCommands({ onClose }: { onClose: () => void }) {
  return (
    <Sheet title="What you can say" onClose={onClose}>
      <p className="sheet-lead">
        Hold the capture shortcut and speak. If nothing below fits what you said, it is kept as a
        note — the app never refuses to remember something because it could not parse it.
      </p>

      <Group label="Saving">
        <Says
          says={["save this to Work", "file this under Reading", "put this in Study/React"]}
          what="Saves what you say into a collection, creating it if it does not exist."
        />
        <Says
          says={["remember that the boiler is serviced in March", "make a note that…"]}
          what="Keeps it with no destination. It is still searchable."
        />
      </Group>

      <Group label="Finding">
        <Says
          says={["what did I save about mortgages", "find the thing about hooks", "search for…"]}
          what="Searches by meaning as well as by word, once the model has indexed."
        />
        <Says says={["show me everything about React"]} what="Opens the Library, filtered." />
        <Says says={["open the pricing page"]} what="Reopens a memory at the source it came from." />
      </Group>

      <Group label="Acting">
        <Says
          says={["remind me to call the bank on Friday", "add a task to renew the insurance"]}
          what="Lands on the Tasks list, with the deadline if you gave one."
        />
        <Says says={["move this to Archive", "tag this as urgent"]} what="Acts on the memory you just made." />
      </Group>

      <Group label="Fixing">
        <Says
          says={["undo"]}
          what="Reverses the last thing that happened. Exact — “undo the react one” is a request
                the app cannot honour, so it does not pretend to."
        />
      </Group>
    </Sheet>
  );
}

function Says({ says, what }: { says: string[]; what: string }) {
  return (
    <div className="say-row">
      <div className="says">
        {says.map((s) => (
          <span key={s} className="says-one">
            “{s}”
          </span>
        ))}
      </div>
      <p>{what}</p>
    </div>
  );
}
