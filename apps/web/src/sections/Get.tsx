import { BRAND } from "../brand";

/**
 * The ask.
 *
 * One button, and next to it the three facts a person weighs before installing
 * anything: what it runs on, what it costs them, and whether it will phone
 * home. The macOS line says what is missing rather than collecting an address
 * for a launch that has no date.
 */
export function Get() {
  return (
    <section className="get" id="get">
      <h2>Say the next one out loud</h2>
      <p>
        Install it, hold the shortcut, and tell it the thing you would otherwise have half
        remembered on Thursday.
      </p>

      <a className="btn big" href="/download/windows">
        Download for Windows
      </a>

      <ul className="terms-of-trade">
        <li>
          <strong>Windows 11</strong>
          <span>macOS runs, but capture is not finished there yet</span>
        </li>
        <li>
          <strong>No account</strong>
          <span>Sign in only if you want your things on a second machine later</span>
        </li>
        <li>
          <strong>Free in beta</strong>
          <span>
            Questions to <a href={`mailto:${BRAND.contact}`}>{BRAND.contact}</a>
          </span>
        </li>
      </ul>
    </section>
  );
}
