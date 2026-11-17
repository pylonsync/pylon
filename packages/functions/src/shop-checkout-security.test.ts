import { afterAll, beforeEach, expect, test } from "bun:test";
import { mkdtempSync, readdirSync, rmdirSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const dir = mkdtempSync(join(tmpdir(), "pylon-shop-security-"));
const root = new URL("../../create-pylon/templates/shop/", import.meta.url);
const stripePath = join(dir, "stripe.ts");
await Bun.write(stripePath, `
export { assertSafeRedirectUrl } from ${JSON.stringify(new URL("../../plugins/stripe/src/safe-url.ts", import.meta.url).pathname)};
export const requests = [];
export const state = { fail: false };
export async function stripeRequest(...args) {
  requests.push(args);
  if (state.fail) throw new Error("local payment failure");
  return { id: "session-test", url: "https://payments.example.test/session" };
}
`);
const stripe = await import(stripePath);

async function loadHandler(name: string) {
  const source = (await Bun.file(new URL(`functions/${name}.ts`, root)).text())
    .replace('"@pylonsync/functions"', JSON.stringify(new URL("./index.ts", import.meta.url).pathname))
    .replace('"@pylonsync/stripe"', JSON.stringify(stripePath))
    .replace('"../lib/shop"', JSON.stringify(new URL("lib/shop.ts", root).pathname))
    .replace('"../lib/site.config"', JSON.stringify(new URL("lib/site.config.ts", root).pathname));
  const path = join(dir, `${name}.ts`);
  await Bun.write(path, source);
  return (await import(path)).default;
}

const checkout = await loadHandler("checkout");
const reserve = await loadHandler("reserveCart");
const release = await loadHandler("releaseGroup");

afterAll(() => {
  for (const name of readdirSync(dir)) unlinkSync(join(dir, name));
  rmdirSync(dir);
});
beforeEach(() => {
  stripe.requests.length = 0;
  stripe.state.fail = false;
});

function fixture(stripeEnabled = true) {
  const product = { id: "product", slug: "shirt", name: "Shirt", stock: 3, priceCents: 2000 };
  const orders: Record<string, unknown>[] = [];
  const mutations: string[] = [];
  const db = {
    advisoryLock: async () => {},
    unsafe: {
      lookup: async () => product,
      list: async () => orders,
      insert: async (_entity: string, row: Record<string, unknown>) => {
        const id = `order-${orders.length}`;
        orders.push({ ...row, id });
        return id;
      },
      update: async (entity: string, id: string, patch: Record<string, unknown>) => {
        Object.assign(entity === "Product" ? product : orders.find(row => row.id === id)!, patch);
      },
    },
  };
  const ctx = {
    auth: { userId: null },
    env: { STRIPE_SECRET_KEY: stripeEnabled ? "local-test-key" : "", PYLON_PUBLIC_URL: "https://shop.example.test" },
    error: (code: string, message: string) => Object.assign(new Error(message), { code }),
    runMutation: async (name: string, args: unknown) => {
      mutations.push(name);
      if (name === "reserveCart") return reserve.handler({ db }, args);
      if (name === "releaseGroup") return release.handler({ db }, args);
      throw new Error(`Unexpected mutation: ${name}`);
    },
  };
  const args = {
    items: [{ slug: "shirt", qty: 3 }], customerName: "Test", customerEmail: "test@example.test",
    successUrl: "https://shop.example.test/success", cancelUrl: "https://shop.example.test/cart",
  };
  return { ctx, args, product, orders, mutations };
}

for (const field of ["successUrl", "cancelUrl"] as const) {
  for (const url of ["not-a-url", "https://untrusted.example.test/"]) {
    test(`rejects ${field} ${url} without holding stock`, async () => {
      const f = fixture();
      f.args[field] = url;
      await expect(checkout.handler(f.ctx, f.args)).rejects.toThrow();
      expect(f.product.stock).toBe(3);
      expect(f.orders).toEqual([]);
      expect(f.mutations).toEqual([]);
      expect(stripe.requests).toEqual([]);
    });
  }
}

test("valid checkout holds stock and creates a payment session", async () => {
  const f = fixture();
  expect(await checkout.handler(f.ctx, f.args)).toMatchObject({ ok: true, mode: "stripe" });
  expect(f.product.stock).toBe(0);
  expect(f.orders[0].status).toBe("pending");
  expect(stripe.requests).toHaveLength(1);
});

test("payment failure restores held stock", async () => {
  const f = fixture();
  stripe.state.fail = true;
  await expect(checkout.handler(f.ctx, f.args)).rejects.toMatchObject({ code: "STRIPE_ERROR" });
  expect(f.product.stock).toBe(3);
  expect(f.orders[0].status).toBe("cancelled");
});

test("manual orders do not require unused payment redirects", async () => {
  const f = fixture(false);
  f.args.successUrl = "";
  f.args.cancelUrl = "";
  expect(await checkout.handler(f.ctx, f.args)).toMatchObject({ ok: true, mode: "reserved" });
  expect(f.product.stock).toBe(0);
  expect(f.orders[0].status).toBe("reserved");
  expect(stripe.requests).toEqual([]);
});
