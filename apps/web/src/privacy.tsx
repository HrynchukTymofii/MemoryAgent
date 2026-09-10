import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { BRAND } from "./brand";
import { Document } from "./components/Shell";
import "./styles/site.css";

/**
 * The privacy policy.
 *
 * For a product whose whole claim is that nothing leaves the machine, this page
 * is a specification, not a disclaimer. It states what stays, what leaves, and
 * what leaves only because this website uses a font server — the last of which
 * most policies quietly omit.
 */
function Privacy() {
  return (
    <Document title="Privacy policy" updated={BRAND.updated}>
      <p className="doc-lede">
        {BRAND.name} keeps what you capture on your computer. We cannot read it, and there is no
        copy of it on a server of ours.
      </p>

      <h2>What stays on your machine</h2>
      <p>
        Audio, transcripts, notes, pages you keep, the search index and the numbers the search
        is built from are all written to a database in your user profile. Speech recognition and
        the search models run locally — they are files shipped with the application, not calls to
        a service. None of it is transmitted anywhere, including to us.
      </p>

      <h2>What leaves, and why</h2>
      <ul>
        <li>
          <strong>If you sign in:</strong> your email address and an account identifier, so the
          licence can be checked and so a second machine can later be recognised as yours.
        </li>
        <li>
          <strong>If you use a referral code:</strong> the code and which account redeemed it, so
          the reward can be granted once.
        </li>
        <li>
          <strong>Update checks:</strong> the application asks whether a newer version exists.
          That request carries the current version and the platform, and nothing about you.
        </li>
      </ul>
      <p>
        There is no telemetry of what you capture, search for, or say. If we ever add anything
        that reports usage, it will be off unless you turn it on, and this page will say so
        before the build ships.
      </p>

      <h2>Syncing</h2>
      <p>
        Sync between your own machines is not finished. When it arrives it will be something you
        switch on, it will be encrypted with a key derived on your machine, and this page will
        describe exactly what the server can see — which is meant to be nothing but ciphertext
        and sizes.
      </p>

      <h2>This website</h2>
      <p>
        No cookies, no analytics, no tracking pixels, no advertising. The pages are static files.
        Two things reach outside: the typefaces are served by Google Fonts, which necessarily
        sees your IP address when the page loads, and downloads are served from our file host,
        whose server logs hold the usual request records for a short period.
      </p>

      <h2>Recordings and other people</h2>
      <p>
        Captures can contain other people's voices and words. That material stays on your
        machine, and the responsibility for recording anyone lawfully is yours — the application
        will not check the law where you are.
      </p>

      <h2>Deleting things</h2>
      <p>
        Delete a capture in the application and it is removed from the database, along with the
        index entries derived from it. Uninstalling removes the application; the database is left
        alone deliberately, so an uninstall is never an accidental erasure. If you have an
        account, write to us and we will delete it and everything attached to it.
      </p>

      <h2>Children</h2>
      <p>
        {BRAND.name} is not intended for children under 13, and we do not knowingly create
        accounts for them.
      </p>

      <h2>Changes</h2>
      <p>
        If what leaves the machine ever changes, this page changes first, and the application
        says so on the next launch. The date at the top is the last substantive edit.
      </p>

      <h2>Reaching us</h2>
      <p>
        Questions about any of this go to <a href={`mailto:${BRAND.contact}`}>{BRAND.contact}</a>.
      </p>
    </Document>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Privacy />
  </StrictMode>,
);
