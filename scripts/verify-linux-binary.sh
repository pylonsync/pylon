#!/usr/bin/env bash
# Checks that a Linux `pylon` binary runs on a stock glibc distro.
#
#   scripts/verify-linux-binary.sh <path-to-pylon>
#
# Fails when the binary:
#   - needs a shared library other than the ones every glibc system has, or
#   - references a glibc symbol newer than MAX_GLIBC.
#
# Both failures stop the binary at load time with no output from pylon, so
# they are checked here, on the file, before it is published.
set -euo pipefail

MAX_GLIBC="${MAX_GLIBC:-2.28}"
bin="${1:?usage: verify-linux-binary.sh <path-to-pylon>}"

# Everything here ships with glibc or libgcc.
allowed='^(libc\.so\.6|libm\.so\.6|libdl\.so\.2|libpthread\.so\.0|librt\.so\.1|libutil\.so\.1|libgcc_s\.so\.1|ld-linux-[a-z0-9_-]+\.so\.[0-9]+)$'

needed="$(readelf -d "$bin" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p')"
extra="$(printf '%s\n' "$needed" | grep -Ev "$allowed" || true)"
if [ -n "$extra" ]; then
    echo "error: $bin needs shared libraries a stock system does not have:" >&2
    printf '  %s\n' $extra >&2
    exit 1
fi

newest="$(readelf --dyn-syms -W "$bin" \
    | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/GLIBC_//' | sort -Vu | tail -1)"
if [ -z "$newest" ]; then
    echo "error: $bin references no glibc symbols; is it a glibc binary?" >&2
    exit 1
fi
if [ "$(printf '%s\n%s\n' "$newest" "$MAX_GLIBC" | sort -V | tail -1)" != "$MAX_GLIBC" ]; then
    echo "error: $bin needs glibc $newest; the limit is $MAX_GLIBC" >&2
    exit 1
fi

echo "ok: $bin needs glibc >= $newest and only these libraries:"
printf '  %s\n' $needed
