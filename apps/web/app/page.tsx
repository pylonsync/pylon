import { Nav } from "@/components/nav";
import { Hero } from "@/components/hero";
import { DemoSection } from "@/components/demo";
import { Features } from "@/components/features";
import { Scale } from "@/components/scale";
import { Compare } from "@/components/compare";
import { Unusual } from "@/components/unusual";
import { Quickstart } from "@/components/quickstart";
import { Footer } from "@/components/footer";

export default function Home() {
  return (
    <>
      <Nav />
      <Hero />
      <DemoSection />
      <Features />
      <Scale />
      <Compare />
      <Unusual />
      <Quickstart />
      <Footer />
    </>
  );
}
