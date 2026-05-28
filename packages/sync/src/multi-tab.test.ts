// Multi-tab broker unit tests. These exercise election + promotion
// across multiple in-process MultiTabBroker instances sharing a real
// BroadcastChannel (Bun provides one in its test runtime). No SyncEngine
// here — the engine-level integration is covered by separate scenario
// tests that pair this broker with a TestServer.

import { afterEach, describe, expect, test } from "bun:test";

import { MultiTabBroker } from "./multi-tab";

const channel = (suffix: string) => `pylon-test:${suffix}:${Math.random()}`;

interface Tab {
  broker: MultiTabBroker;
  events: { type: string; data: unknown }[];
}

function makeTab(channelName: string): Promise<Tab> {
  return new Promise((resolve) => {
    const events: Tab["events"][number][] = [];
    const broker = new MultiTabBroker();
    let promoted = false;
    let demoted = false;
    broker.start(channelName, {
      onPromote: () => {
        events.push({ type: "promote", data: broker.self.tabId });
        promoted = true;
      },
      onDemote: () => {
        events.push({ type: "demote", data: broker.self.tabId });
        demoted = true;
      },
      onAppMessage: (payload, from) => {
        events.push({ type: "app", data: { payload, from } });
      },
    });
    // Give the election a beat to settle.
    setTimeout(() => resolve({ broker, events }), 350);
    // Silence unused-var lint.
    void promoted;
    void demoted;
  });
}

describe("MultiTabBroker", () => {
  const tabs: Tab[] = [];
  afterEach(() => {
    for (const t of tabs) t.broker.stop();
    tabs.length = 0;
  });

  test("a single tab promotes itself", async () => {
    const ch = channel("single");
    const a = await makeTab(ch);
    tabs.push(a);
    expect(a.broker.isLeader()).toBe(true);
    expect(a.events.some((e) => e.type === "promote")).toBe(true);
  });

  test("two tabs converge on one leader (earlier startTime wins)", async () => {
    const ch = channel("two-tabs");
    const a = await makeTab(ch);
    tabs.push(a);
    // Wait a beat so b's startTime is strictly greater.
    await new Promise((r) => setTimeout(r, 20));
    const b = await makeTab(ch);
    tabs.push(b);

    // Give the second tab a moment to see the first's hello/here.
    await new Promise((r) => setTimeout(r, 200));

    expect(a.broker.isLeader()).toBe(true);
    expect(b.broker.isLeader()).toBe(false);
  });

  test("leader broadcasts reach the other tab as app events", async () => {
    const ch = channel("broadcast");
    const a = await makeTab(ch);
    tabs.push(a);
    await new Promise((r) => setTimeout(r, 20));
    const b = await makeTab(ch);
    tabs.push(b);
    await new Promise((r) => setTimeout(r, 100));

    a.broker.broadcastApp({ type: "applied", n: 1 });
    a.broker.broadcastApp({ type: "applied", n: 2 });

    await new Promise((r) => setTimeout(r, 80));

    const appEvents = b.events.filter((e) => e.type === "app");
    expect(appEvents.length).toBe(2);
    expect((appEvents[0].data as { payload: { n: number } }).payload.n).toBe(1);
    expect((appEvents[1].data as { payload: { n: number } }).payload.n).toBe(2);
  });

  test("a tab does not receive its own broadcasts", async () => {
    const ch = channel("no-echo");
    const a = await makeTab(ch);
    tabs.push(a);
    a.events.length = 0;

    a.broker.broadcastApp({ type: "applied", n: 1 });
    await new Promise((r) => setTimeout(r, 50));

    const appEvents = a.events.filter((e) => e.type === "app");
    expect(appEvents.length).toBe(0);
  });

  test("when the leader stops, a follower is promoted", async () => {
    const ch = channel("promote-on-stop");
    const a = await makeTab(ch);
    tabs.push(a);
    await new Promise((r) => setTimeout(r, 20));
    const b = await makeTab(ch);
    tabs.push(b);

    await new Promise((r) => setTimeout(r, 200));
    expect(a.broker.isLeader()).toBe(true);
    expect(b.broker.isLeader()).toBe(false);

    // Leader exits. The `bye` message lets the follower re-elect
    // without waiting for the heartbeat-timeout.
    a.broker.stop();

    // Re-election needs an election-settle window after the `bye`.
    await new Promise((r) => setTimeout(r, 400));

    expect(b.broker.isLeader()).toBe(true);
    expect(
      b.events.filter((e) => e.type === "promote").length,
    ).toBeGreaterThanOrEqual(1);
  });
});
