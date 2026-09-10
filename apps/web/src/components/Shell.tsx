import type { ReactNode } from "react";

import { BRAND } from "../brand";
import { Logo } from "./Logo";

/**
 * The chrome every page shares: the bar at the top and the small print at the
 * bottom.
 *
 * The header is the same object on all three pages and does not change with
 * scroll — a bar that shrinks, blurs or reappears on scroll-up is motion the
 * reader did not ask for, on the one element they need to stay put.
 */
export function Header() {
  return (
    <header className="bar">
      <a className="wordmark" href="/">
        <Logo size={30} />
        <span>{BRAND.name}</span>
      </a>
      <nav className="bar-nav">
        <a href="/#how">How it works</a>
        <a href="/#proof">Why it sticks</a>
        <a href="/privacy/">Privacy</a>
      </nav>
      <a className="btn small" href="/#get">
        Download
      </a>
    </header>
  );
}

export function Footer() {
  return (
    <footer className="foot">
      <div className="foot-in">
        <a className="wordmark" href="/">
          <Logo size={24} />
          <span>{BRAND.name}</span>
        </a>
        <nav>
          <a href="/terms/">Terms</a>
          <a href="/privacy/">Privacy</a>
          <a href={`mailto:${BRAND.contact}`}>{BRAND.contact}</a>
        </nav>
        <p>Runs on your machine. © {new Date().getFullYear()} {BRAND.entity}.</p>
      </div>
    </footer>
  );
}

/**
 * A page of prose: the terms and the policy.
 *
 * One column, no reveal animations and no cards. Someone reading this page is
 * either checking a specific clause or deciding whether to trust the product,
 * and both are badly served by text that has to be scrolled into existence.
 */
export function Document({
  title,
  updated,
  children,
}: {
  title: string;
  updated: string;
  children: ReactNode;
}) {
  return (
    <>
      <Header />
      <main className="doc">
        <h1>{title}</h1>
        <p className="doc-date">Last updated {updated}</p>
        {children}
      </main>
      <Footer />
    </>
  );
}
