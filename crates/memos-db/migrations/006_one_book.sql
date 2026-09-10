-- 006_one_book — one document, not one per collection (ADR-0010, revised).
--
-- A note per collection is still a pile of files. What a person wants is one
-- thing they can read top to bottom, with their own structure as its headings
-- — the collection path becomes the heading trail inside it rather than a
-- filename. Collections stay what they always were: the destinations the
-- router routes to, and now the outline of the document.
--
-- The existing notes are folded into the first of them rather than dropped:
-- each becomes a top-level section of the one document.

UPDATE notes
   SET body = (
         SELECT group_concat('# ' || n.title || char(10) || char(10) || n.body, char(10))
           FROM notes n
       ),
       title = 'Everything',
       collection_id = NULL
 WHERE id = (SELECT id FROM notes ORDER BY created_at LIMIT 1);

-- Every capture that had been integrated now points at the survivor.
UPDATE knowledge_items
   SET note_id = (SELECT id FROM notes WHERE collection_id IS NULL ORDER BY created_at LIMIT 1)
 WHERE note_id IS NOT NULL;

DELETE FROM notes WHERE collection_id IS NOT NULL;
