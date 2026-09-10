import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { BRAND } from "./brand";
import { Document } from "./components/Shell";
import "./styles/site.css";

/**
 * The terms.
 *
 * Written to be read: short sections, plain sentences, and the awkward parts —
 * beta, no warranty, what happens if we stop — said outright rather than buried
 * in capitals. It is not a substitute for a lawyer's review before launch, and
 * the governing-law clause is deliberately unfilled until there is a company to
 * fill it with.
 */
function Terms() {
  return (
    <Document title="Terms of service" updated={BRAND.updated}>
      <p className="doc-lede">
        These terms are the agreement between you and {BRAND.entity} for using {BRAND.name}. By
        installing or using it, you accept them.
      </p>

      <h2>1. What you are getting</h2>
      <p>
        {BRAND.name} is a desktop application that captures what you say or keep, stores it on
        your own computer, and helps you find it again. It is licensed to you, not sold: you may
        install and use it on machines you control, for yourself or for your work.
      </p>

      <h2>2. What is yours</h2>
      <p>
        Everything you capture is yours. It lives in a database on your disk in a documented
        format, and you may export, copy, back up or delete it at any time without asking us.
        We claim no licence over it, and we cannot read it — see the{" "}
        <a href="/privacy/">privacy policy</a>.
      </p>

      <h2>3. Accounts</h2>
      <p>
        An account is optional. Everything except syncing between machines works without one. If
        you create one, keep the credentials to yourself and tell us promptly if they are lost —
        you are responsible for what happens under your account until you do.
      </p>

      <h2>4. What you may not do</h2>
      <ul>
        <li>Record people who have not agreed to be recorded, where your law requires that they do.</li>
        <li>Resell, sublicense or rent the application itself.</li>
        <li>Work around the licence checks, or use the referral scheme through accounts you made up.</li>
        <li>Use it to build a competing product out of the parts, or to break the law.</li>
      </ul>

      <h2>5. Price</h2>
      <p>
        {BRAND.name} is free while it is in beta. Paid plans will come, and when they do we will
        say so clearly before anything is charged. Anything you have already captured stays
        readable and exportable whatever plan you are on: a free account is never a locked
        archive.
      </p>

      <h2>6. Beta software</h2>
      <p>
        This is unfinished software and it will misbehave. Capture can miss, recognition can
        mishear, and an update can change how something works. Keep your own backups of anything
        you cannot afford to lose. The application is provided as it is, without warranties of
        any kind, and to the extent the law allows, {BRAND.entity} is not liable for lost data,
        lost profit or consequential damage arising from its use.
      </p>

      <h2>7. Stopping</h2>
      <p>
        You may stop at any time by uninstalling it; your data stays on your disk until you
        delete it. We may suspend an account that is being used to attack the service or other
        people, and we will say why. If we ever discontinue {BRAND.name} entirely, we will
        publish a final build that runs without any server of ours.
      </p>

      <h2>8. Changes</h2>
      <p>
        We will update these terms as the product changes. Material changes will be announced in
        the application before they take effect, and the date at the top of this page always
        says when it was last touched.
      </p>

      <h2>9. Law</h2>
      <p>
        These terms are governed by the law of <em>[jurisdiction to be completed]</em>, and any
        dispute belongs to the courts there. Nothing here removes rights you have as a consumer
        under the law where you live.
      </p>

      <h2>10. Reaching us</h2>
      <p>
        Write to <a href={`mailto:${BRAND.contact}`}>{BRAND.contact}</a>. A person answers.
      </p>
    </Document>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Terms />
  </StrictMode>,
);
