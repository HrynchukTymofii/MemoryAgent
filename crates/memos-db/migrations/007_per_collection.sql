-- 007_per_collection — the document belongs to a collection again.
--
-- 006 folded every note into one file. That was a misreading: what a person
-- wants is one file *per subject* — everything about React in one page with
-- headings, not everything about everything in one page with a tree of them.
-- A low-level collection is that subject; the ones above it are the way there.
--
-- Nothing is parsed back out of the one document. An empty one is dropped,
-- which is every case where 006 ran on a database nobody had written in; one
-- with words in it is kept, belonging to no collection, so that whoever wrote
-- them can still be given them back rather than finding them gone.

DELETE FROM notes WHERE collection_id IS NULL AND trim(body) = '';
