import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { Header, Footer } from "./components/Shell";
import { Hero } from "./sections/Hero";
import { Lost } from "./sections/Lost";
import { Demo } from "./sections/Demo";
import { Features } from "./sections/Features";
import { Curve } from "./sections/Curve";
import { Get } from "./sections/Get";
import "./styles/site.css";

/**
 * The landing page, in the order the argument is made: here is the thing, here
 * is what happens without it, here it is working, here is how, here is why it
 * matters by Thursday, here is the button.
 */
function Page() {
  return (
    <>
      <Header />
      <main>
        <Hero />
        <Lost />
        <Demo />
        <Features />
        <Curve />
        <Get />
      </main>
      <Footer />
    </>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Page />
  </StrictMode>,
);
