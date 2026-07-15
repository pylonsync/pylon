import { describe, expect, test } from "bun:test";

import {
	assertPublicDestination,
	isBlockedIp,
	postWebhookSafely,
} from "./safe-request";

const publicDns = async () => [{ address: "93.184.216.34", family: 4 }];

describe("webhook egress guard", () => {
	test("blocks non-HTTPS URLs, embedded credentials, and private DNS results", async () => {
		await expect(
			assertPublicDestination(new URL("http://example.com/hook"), publicDns),
		).rejects.toThrow("must use HTTPS");
		await expect(
			assertPublicDestination(new URL("https://user:pass@example.com/hook"), publicDns),
		).rejects.toThrow("credentials are not allowed");
		await expect(
			assertPublicDestination(new URL("https://example.com/hook"), async () => [
				{ address: "169.254.169.254", family: 4 },
			]),
		).rejects.toThrow("non-public address");
	});

	test("blocks private, link-local, reserved, and mapped addresses", () => {
		for (const address of [
			"127.0.0.1",
			"10.0.0.1",
			"172.16.0.1",
			"192.168.1.1",
			"169.254.169.254",
			"100.64.0.1",
			"0.1.2.3",
			"224.0.0.1",
			"::1",
			"fe80::1",
			"fd12::1",
			"::192.168.1.1",
			"::ffff:169.254.169.254",
			"2001:db8::1",
		]) {
			expect(isBlockedIp(address)).toBe(true);
		}
		expect(isBlockedIp("8.8.8.8")).toBe(false);
		expect(isBlockedIp("2606:4700:4700::1111")).toBe(false);
	});

	test("revalidates same-origin redirect hops", async () => {
		const seen: string[] = [];
		let resolutions = 0;
		const result = await postWebhookSafely(
			"https://example.com/first",
			{},
			"{}",
			{
				resolve: async () => {
					resolutions++;
					return publicDns();
				},
				requestOnce: async (url) => {
					seen.push(url.pathname);
					return seen.length === 1
						? { status: 307, location: "/second" }
						: { status: 204 };
				},
			},
		);

		expect(result).toEqual({ status: 204, ok: true });
		expect(seen).toEqual(["/first", "/second"]);
		expect(resolutions).toBe(2);
	});

	test("rejects cross-origin and method-changing redirects", async () => {
		await expect(
			postWebhookSafely("https://example.com/hook", {}, "{}", {
				resolve: publicDns,
				requestOnce: async () => ({
					status: 307,
					location: "https://other.example/hook",
				}),
			}),
		).rejects.toThrow("another origin");

		await expect(
			postWebhookSafely("https://example.com/hook", {}, "{}", {
				resolve: publicDns,
				requestOnce: async () => ({ status: 302, location: "/other" }),
			}),
		).rejects.toThrow("unsafe redirect");
	});

	test("caps redirect chains", async () => {
		await expect(
			postWebhookSafely("https://example.com/hook", {}, "{}", {
				resolve: publicDns,
				requestOnce: async () => ({ status: 307, location: "/again" }),
			}),
		).rejects.toThrow("redirect limit");
	});

	test("aborts requests at the delivery deadline", async () => {
		await expect(
			postWebhookSafely("https://example.com/hook", {}, "{}", {
				resolve: publicDns,
				timeoutMs: 5,
				requestOnce: async (_url, { signal }) =>
					new Promise((_resolve, reject) => {
						signal.addEventListener("abort", () => reject(new Error("aborted")));
					}),
			}),
		).rejects.toThrow("timed out");
	});

	test("applies the delivery deadline while resolving DNS", async () => {
		await expect(
			postWebhookSafely("https://example.com/hook", {}, "{}", {
				resolve: async () => new Promise(() => {}),
				timeoutMs: 5,
			}),
		).rejects.toThrow("timed out");
	});
});
