/* Regression for RUSTSEC-2026-0459 in bundled SQLite.
 * Compile against the amalgamation selected by Cargo:
 *   cc -DSQLITE_SOURCE='"/path/to/sqlite3.c"' tools/sqlite-wal-regression.c \
 *      -o /tmp/sqlite-wal-regression -lpthread -lm
 * Run /tmp/sqlite-wal-regression. Exit 1 means the page tail was not cleared.
 *
 * Include the amalgamation to test the internal WAL reader directly.
 * A controlled file reader supplies a 512-byte frame to a 4096-byte page.
 * The page starts with known bytes. No process secrets or real files are read.
 */
#define SQLITE_OMIT_LOAD_EXTENSION 1
#include SQLITE_SOURCE

static int read_frame(sqlite3_file *file, void *out, int size, sqlite3_int64 offset) {
  (void)file;
  if (size != 512 || offset != 56) return SQLITE_IOERR;
  memset(out, 0x42, size);
  return SQLITE_OK;
}

int main(void) {
  sqlite3_io_methods methods = {0};
  sqlite3_file file = {0};
  Wal wal = {0};
  unsigned char page[4096];
  methods.iVersion = 1;
  methods.xRead = read_frame;
  file.pMethods = &methods;
  wal.pWalFd = &file;
  wal.hdr.szPage = 512;
  memset(page, 0xa5, sizeof(page));
  if (sqlite3WalReadFrame(&wal, 1, sizeof(page), page) != SQLITE_OK) return 2;
  for (int i = 0; i < 512; i++) if (page[i] != 0x42) return 3;
  for (int i = 512; i < sizeof(page); i++) {
    if (page[i] != 0) {
      puts("FAIL: WAL read left old bytes in the page tail");
      return 1;
    }
  }
  puts("PASS: WAL read cleared the page tail");
  return 0;
}
