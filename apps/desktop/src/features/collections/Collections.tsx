import { useEffect, useState } from "react";

import { api, type CollectionRow } from "../../lib/api";
import { Empty } from "../../components/ItemList";

export function Collections({
  revision,
  onOpen,
}: {
  revision: number;
  onOpen: (path: string | null) => void;
}) {
  const [rows, setRows] = useState<CollectionRow[] | null>(null);

  useEffect(() => {
    let live = true;
    void api
      .collections()
      .then((r) => live && setRows(r))
      .catch(() => live && setRows([]));
    return () => {
      live = false;
    };
  }, [revision]);

  return (
    <div className="panel">
      <h1>Collections</h1>
      <p className="sub">
        Where your memories are filed. These names are also the destinations the router
        understands — saying “save this to reading” only works because a collection called Reading
        exists, and speech recognition is biased toward these words.
      </p>

      {rows === null ? null : rows.length === 0 ? (
        <Empty title="No collections yet">
          Captures land unfiled until there is somewhere to put them.
        </Empty>
      ) : (
        <div className="tree">
          {rows.map((c) => (
            <button
              key={c.id}
              type="button"
              className="node"
              // Indent by depth rather than nesting the markup: the paths are
              // already materialised in the database, and rebuilding a tree in
              // the interface would be a second source of truth for the shape.
              style={{ marginLeft: c.depth * 18 }}
              onClick={() => onOpen(c.path)}
              title={`Show everything in ${c.path.replace(/\//g, " / ")}`}
            >
              <span className="g">{c.depth === 0 ? "◱" : "└"}</span>
              <span className="n">{c.name}</span>
              <span className="c">{c.items}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
