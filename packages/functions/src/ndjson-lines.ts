/**
 * Incremental NDJSON line splitter for the runner's stdin.
 *
 * Each chunk is scanned once for newlines. A line that spans many chunks is
 * kept as a list of parts and joined once when its newline arrives. The old
 * reader appended every chunk to one string and split the whole string on
 * each read, which is quadratic in the line length: a 48MB frame (a large
 * function argument, for example a deploy upload) arrived as ~750 64KB
 * chunks and was rescanned on each one, so the runner stalled and the host's
 * write failed with a broken pipe.
 */
export function createLineSplitter(onLine: (line: string) => void) {
  const decoder = new TextDecoder();
  let pending: string[] = [];

  function emit(line: string) {
    if (line.trim()) onLine(line);
  }

  function consume(text: string) {
    let start = 0;
    let nl = text.indexOf("\n");
    while (nl !== -1) {
      const tail = text.slice(start, nl);
      if (pending.length) {
        pending.push(tail);
        emit(pending.join(""));
        pending = [];
      } else {
        emit(tail);
      }
      start = nl + 1;
      nl = text.indexOf("\n", start);
    }
    if (start < text.length) pending.push(text.slice(start));
  }

  return {
    /** Feed one chunk of bytes from the stream. */
    push(chunk: Uint8Array) {
      consume(decoder.decode(chunk, { stream: true }));
    },
    /** The stream ended: flush the decoder and emit a final unterminated line. */
    end() {
      consume(decoder.decode());
      if (pending.length) {
        emit(pending.join(""));
        pending = [];
      }
    },
  };
}
