// <FileUpload offline attach>: each file is kept on the device and shown
// from there at once, set on its record, and uploaded when there is a
// network.

import { afterEach, describe, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { db, init } from "@pylonsync/react";
import { createTestEnv, type TestEnv } from "@pylonsync/sync/src/test-harness";

import { FileUpload } from "./FileUpload";

let env: TestEnv | null = null;
afterEach(async () => {
	cleanup();
	if (env) {
		await env.dispose();
		env = null;
	}
});

describe("FileUpload offline", () => {
	test("queues each file, shows it from the device, and attaches it once uploaded", async () => {
		env = createTestEnv({ transport: "poll" });
		env.signIn({ userId: "rep1" });
		env.server.seed("Photo", [{ id: "p1", fileId: null, url: null }]);
		// The component reads the default engine: point it at the harness.
		init({
			baseUrl: "http://test.invalid",
			appName: "harness",
			persist: false,
			multiTab: false,
			transport: "poll",
			storage: (env.engine as unknown as { storage: never }).storage,
		});
		await db.sync.start();
		// Hold the upload so the waiting stage is observable.
		let release!: () => void;
		env.server.files.putGate = new Promise<void>((r) => (release = r));

		const queued: string[] = [];
		const { container } = render(
			<FileUpload
				offline
				accept="image/*"
				attach={{ entity: "Photo", id: "p1", field: "fileId", urlField: "url" }}
				onQueued={(q) => queued.push(q.ref)}
			/>,
		);
		const input = container.querySelector("input[type=file]") as HTMLInputElement;
		const file = new File([new Uint8Array(32)], "gutter.jpg", { type: "image/jpeg" });
		await act(async () => {
			fireEvent.change(input, { target: { files: [file] } });
		});
		await waitFor(() => expect(queued).toHaveLength(1));
		// The record holds the local reference and the picker shows the copy.
		expect(db.sync.store.get("Photo", "p1")).toMatchObject({ fileId: queued[0], url: queued[0] });
		expect(screen.getByText("Waiting to upload")).toBeTruthy();
		expect(container.querySelector("img")?.getAttribute("src")).toMatch(/^blob:/);

		env.server.files.putGate = null;
		await act(async () => {
			release();
		});
		await waitFor(() => expect(screen.getByText("Uploaded")).toBeTruthy());
		await waitFor(() =>
			expect(db.sync.store.get("Photo", "p1")).toMatchObject({ fileId: "f_1", url: "/api/files/f_1" }),
		);
		db.sync.stop();
	});
});
