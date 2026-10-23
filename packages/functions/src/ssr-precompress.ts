// Brotli and gzip copies of the client build's text files, written at build
// time. The runtime sends `<file>.br` or `<file>.gz` with Content-Encoding
// when the browser accepts it (serve_pylon_client_bundle in
// crates/runtime/src/frontend.rs), so no request pays to compress.

/** Extensions worth compressing. Images and fonts are compressed already. */
const COMPRESSIBLE = new Set([".js", ".mjs", ".css", ".json", ".map", ".svg", ".wasm", ".txt", ".html"]);

/** Smaller files gain less than the bytes of the encoding headers. */
export const MIN_COMPRESS_BYTES = 1024;

export interface PrecompressResult {
  /** Files that got at least one compressed copy. */
  files: number;
  /** Total bytes of those files, and of their brotli copies. */
  bytes: number;
  brotliBytes: number;
}

/**
 * Write `<file>.br` (brotli, quality 11) and `<file>.gz` (gzip, level 9)
 * next to every compressible file in `outdir` of at least
 * `MIN_COMPRESS_BYTES`. A copy that is not smaller than the file is not
 * written. Copies left by an earlier build are replaced.
 */
export function precompressBuild(fs: any, path: any, zlib: any, outdir: string): PrecompressResult {
  const result: PrecompressResult = { files: 0, bytes: 0, brotliBytes: 0 };
  const walk = (dir: string) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(full);
        continue;
      }
      if (!entry.isFile() || !COMPRESSIBLE.has(path.extname(entry.name))) continue;
      const data: Uint8Array = fs.readFileSync(full);
      if (data.length < MIN_COMPRESS_BYTES) continue;
      const br: Uint8Array = zlib.brotliCompressSync(data, {
        params: {
          [zlib.constants.BROTLI_PARAM_QUALITY]: 11,
          [zlib.constants.BROTLI_PARAM_SIZE_HINT]: data.length,
        },
      });
      const gz: Uint8Array = zlib.gzipSync(data, { level: 9 });
      let wrote = false;
      for (const [ext, copy] of [
        [".br", br],
        [".gz", gz],
      ] as const) {
        const target = full + ext;
        if (copy.length < data.length) {
          fs.writeFileSync(target, copy);
          wrote = true;
        } else if (fs.existsSync(target)) {
          fs.unlinkSync(target);
        }
      }
      if (wrote) {
        result.files++;
        result.bytes += data.length;
        result.brotliBytes += Math.min(br.length, data.length);
      }
    }
  };
  walk(outdir);
  return result;
}
