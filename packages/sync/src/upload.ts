// File upload: POST /api/files/init, PUT the bytes, POST /api/files/confirm.
//
// `@pylonsync/react`'s `uploadFile` and the sync engine's offline upload
// queue both call `uploadBytes`, so the two never send a file differently.

import { pylonFetch, pylonFetchRaw, PylonHttpError, type TransportConfig } from "./transport";

export interface UploadedFile {
  id: string;
  /** Where to read the file. Store this on your row. */
  url: string;
  size: number;
}

/**
 * Who can read an uploaded file through `url`.
 *   - "private" (default): only the uploader.
 *   - "public": anyone, signed in or not. Use it for content every visitor
 *     sees, such as feed photos or listing images.
 * Writes and deletes stay with the uploader either way. On a CDN-backed
 * store (Stack0, a public bucket) every file is served by its URL, so the
 * setting only changes reads that go through `/api/files/<id>`.
 */
export type FileVisibility = "private" | "public";

export interface UploadBytesOptions {
  filename?: string;
  contentType?: string;
  visibility?: FileVisibility;
}

/**
 * Upload a file (File, Blob, or raw bytes) in three steps:
 *   1. POST /api/files/init with the name, type, size, and visibility;
 *   2. PUT the bytes to the returned upload URL;
 *   3. POST /api/files/confirm, which returns `{ id, url, size }`.
 *
 * The upload URL is either pylon's own receiver (local disk), which needs
 * the session like any API call, or a presigned storage URL, whose
 * signature is the credential. The session is never sent to a presigned
 * URL on another origin.
 *
 * Throws `PylonHttpError` (with `status`) when the server answers with an
 * error, and the fetch error when there is no network.
 */
export async function uploadBytes(
  transport: TransportConfig,
  input: File | Blob | ArrayBuffer | Uint8Array,
  options: UploadBytesOptions = {},
): Promise<UploadedFile> {
  let body: BodyInit;
  let size: number;
  let filename = options.filename;
  let contentType = options.contentType;

  if (typeof File !== "undefined" && input instanceof File) {
    body = input;
    size = input.size;
    filename ??= input.name;
    contentType ??= input.type || "application/octet-stream";
  } else if (typeof Blob !== "undefined" && input instanceof Blob) {
    body = input;
    size = input.size;
    contentType ??= input.type || "application/octet-stream";
  } else if (input instanceof ArrayBuffer) {
    body = input;
    size = input.byteLength;
  } else {
    // Newer TS lib types refuse `Uint8Array<ArrayBufferLike>` as BodyInit
    // directly even though every runtime accepts it. Hand fetch the
    // underlying ArrayBuffer slice to sidestep the type narrowing.
    const u8 = input as Uint8Array;
    body = u8.buffer.slice(u8.byteOffset, u8.byteOffset + u8.byteLength) as ArrayBuffer;
    size = u8.byteLength;
  }

  filename ??= "upload";
  contentType ??= "application/octet-stream";

  const slot = await pylonFetch<{ uploadUrl: string; assetId: string }>(
    transport,
    "/api/files/init",
    {
      method: "POST",
      json: {
        filename,
        mimeType: contentType,
        size,
        visibility: options.visibility ?? "private",
      },
    },
  );

  const base = new URL(transport.baseUrl || "http://localhost");
  const target = new URL(slot.uploadUrl, base);
  const put =
    target.origin === base.origin
      ? await pylonFetchRaw(transport, `${target.pathname}${target.search}`, {
          method: "PUT",
          body,
          headers: { "Content-Type": contentType },
        })
      : await fetch(target.toString(), {
          method: "PUT",
          body,
          headers: { "Content-Type": contentType },
        });
  // Read the response body to the end, even an empty 204. Chrome cancels a
  // fetch whose body is never read once the Response is dropped, and
  // DevTools then lists the upload as failed (net::ERR_ABORTED) although
  // the server stored the bytes.
  const putBody = await put.text().catch(() => "");
  if (!put.ok) {
    let message = `Upload of ${filename} failed: ${put.status}`;
    let code: string | undefined;
    try {
      const err = JSON.parse(putBody) as { error?: { code?: string; message?: string } };
      if (err.error?.message) message = err.error.message;
      code = err.error?.code;
    } catch {
      // Not JSON (storage provider XML, proxy page): keep the status text.
    }
    throw new PylonHttpError(message, put.status, code);
  }

  return pylonFetch<UploadedFile>(transport, "/api/files/confirm", {
    method: "POST",
    json: { assetId: slot.assetId },
  });
}
