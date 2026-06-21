/**
 * "Clicking around" latency-tail probe — for chasing RANDOM HANGS, not
 * throughput. Low request rate (like a real user navigating), but enough
 * samples to expose a fat tail. Each iteration hits a random SSR route and
 * records latency tagged by route, so we see WHICH page hangs and how often.
 *
 * A healthy SSR app: tight p50/p99. A random hang shows as max ≫ p99 (a few
 * multi-second renders hiding in an otherwise-fast distribution) — the
 * fingerprint of a Fly cold-start, a cold Bun runner, or a wedged SSR runner.
 *
 *   k6 run -e BASE_URL=https://www.notbehind.com benchmarks/k6/clickaround.js
 */
import http from "k6/http";
import { Trend } from "k6/metrics";

const BASE_URL = __ENV.BASE_URL || "https://www.notbehind.com";
const RATE = parseInt(__ENV.RATE || "12", 10); // gentle: ~12 req/s against prod
const DURATION = __ENV.DURATION || "30s";

const ROUTES = [
  "/",
  "/learn",
  "/learn/01-first-conversation/first-conversation",
  "/learn/02-how-it-works/how-it-works",
  "/learn/03-prompting/prompting",
  "/learn/04-everyday-life/everyday-life",
  "/learn/05-at-work/at-work",
  "/learn/06-beyond-text/beyond-text",
  "/learn/07-judgment-privacy/judgment-privacy",
  "/learn/08-staying-current/staying-current",
];

const slow = new Trend("slow_ms", true); // every request, ms

export const options = {
  scenarios: {
    click: {
      executor: "constant-arrival-rate",
      rate: RATE,
      timeUnit: "1s",
      duration: DURATION,
      preAllocatedVUs: 50,
      maxVUs: 200, // generous so a hung request never starves the arrival rate
    },
  },
};

export default function () {
  const route = ROUTES[Math.floor(Math.random() * ROUTES.length)];
  const res = http.get(`${BASE_URL}${route}`, { tags: { route } });
  slow.add(res.timings.duration);
  // Flag any individual hang loudly so it shows in the log, with the route.
  if (res.timings.duration > 1500) {
    console.warn(
      `HANG ${Math.round(res.timings.duration)}ms  ${route}  status=${res.status}`,
    );
  }
}
