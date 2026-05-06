import { LandingPage } from "./landing-page";

// pylonsync.com homepage. Sourced from a Claude Design handoff — the
// implementation lives in landing-page.tsx so the route file stays a
// thin entry-point and the design ships as a single client component
// (it owns the live-data simulation in the hero product mock).
export default function Home() {
  return <LandingPage />;
}
