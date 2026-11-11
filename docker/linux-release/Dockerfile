# syntax=docker/dockerfile:1
#
# Build environment for the published Linux `pylon` binary.
#
# The binary links two things from this image:
#   - glibc 2.28 (AlmaLinux 8). The result runs on any glibc >= 2.28 distro:
#     Debian 10+, Ubuntu 20.04+, RHEL 8+, Amazon Linux 2023.
#   - static libxml2, xmlsec1 and OpenSSL built below. SAML signature
#     verification (samael's xmlsec feature) needs them, and a binary that
#     takes them from the distro does not start where they are not installed.
#
# Use scripts/build-linux-release.sh. It builds this image, runs cargo in it
# and checks the result with scripts/verify-linux-binary.sh.

FROM quay.io/pypa/manylinux_2_28@sha256:8ba480e6fdcd18967512389b0d5aa32fda7d4458631eeef3acd1e83133c54a53

# clang-devel: libclang for bindgen (samael generates the xmlsec bindings).
# perl-core: OpenSSL's Configure script.
RUN dnf install -y clang clang-devel perl-core \
    && dnf clean all

ENV DEPS_PREFIX=/opt/pylon-deps
WORKDIR /tmp/deps

# One OpenSSL for the whole binary. xmlsec1-openssl links it here, and the
# cargo build is pointed at the same copy (OPENSSL_NO_VENDOR below), so the
# `vendored` feature of the openssl crate does not add a second one.
# no-module builds the legacy provider into libcrypto.a instead of a
# loadable legacy.so.
ARG OPENSSL_VERSION=3.5.9
ARG OPENSSL_SHA256=603f5602e2eef00d77fbd429d34dcd5822bb301757a1bc9cdb24c670f1eb859a
RUN curl -fsSLO "https://github.com/openssl/openssl/releases/download/openssl-${OPENSSL_VERSION}/openssl-${OPENSSL_VERSION}.tar.gz" \
    && echo "${OPENSSL_SHA256}  openssl-${OPENSSL_VERSION}.tar.gz" | sha256sum -c - \
    && tar xzf "openssl-${OPENSSL_VERSION}.tar.gz" \
    && cd "openssl-${OPENSSL_VERSION}" \
    && ./Configure "linux-$(uname -m)" no-shared no-module no-tests no-docs -fPIC \
        --prefix="${DEPS_PREFIX}" --libdir=lib --openssldir=/usr/local/ssl \
    && make -j"$(nproc)" \
    && make install_sw \
    && cd .. && rm -rf "openssl-${OPENSSL_VERSION}"*

ARG LIBXML2_VERSION=2.15.3
ARG LIBXML2_SHA256=78262a6e7ac170d6528ebfe2efccdf220191a5af6a6cd61ea4a9a9a5042c7a07
RUN curl -fsSLO "https://download.gnome.org/sources/libxml2/${LIBXML2_VERSION%.*}/libxml2-${LIBXML2_VERSION}.tar.xz" \
    && echo "${LIBXML2_SHA256}  libxml2-${LIBXML2_VERSION}.tar.xz" | sha256sum -c - \
    && tar xJf "libxml2-${LIBXML2_VERSION}.tar.xz" \
    && cd "libxml2-${LIBXML2_VERSION}" \
    && ./configure --prefix="${DEPS_PREFIX}" --enable-static --disable-shared --with-pic \
        --without-python --without-zlib --without-icu --without-readline --without-history \
    && make -j"$(nproc)" \
    && make install \
    && cd .. && rm -rf "libxml2-${LIBXML2_VERSION}"*

# --disable-crypto-dl links the OpenSSL backend in. With crypto-dl on, xmlsec
# loads libxmlsec1-openssl.so with dlopen at run time.
ARG XMLSEC_VERSION=1.3.13
ARG XMLSEC_SHA256=7e500061eed734c55d74c786998e7562e55b1aaa72a2dff8ce3130b4c5b832d5
RUN curl -fsSLO "https://github.com/lsh123/xmlsec/releases/download/${XMLSEC_VERSION}/xmlsec1-${XMLSEC_VERSION}.tar.gz" \
    && echo "${XMLSEC_SHA256}  xmlsec1-${XMLSEC_VERSION}.tar.gz" | sha256sum -c - \
    && tar xzf "xmlsec1-${XMLSEC_VERSION}.tar.gz" \
    && cd "xmlsec1-${XMLSEC_VERSION}" \
    && PKG_CONFIG_PATH="${DEPS_PREFIX}/lib/pkgconfig" ./configure --prefix="${DEPS_PREFIX}" \
        --enable-static --disable-shared --with-pic \
        --disable-crypto-dl --disable-apps-crypto-dl --disable-apps --disable-docs \
        --with-openssl="${DEPS_PREFIX}" --with-libxml="${DEPS_PREFIX}" \
        --without-libxslt --without-nss --without-nspr --without-gnutls --without-gcrypt \
    && make -j"$(nproc)" \
    && make install \
    && cd .. && rm -rf "xmlsec1-${XMLSEC_VERSION}"*

# A shared object in the prefix would let the linker pick it over the archive.
RUN test -z "$(find "${DEPS_PREFIX}/lib" -name '*.so*')" \
    && test -f "${DEPS_PREFIX}/lib/libxmlsec1-openssl.a" \
    && test -f "${DEPS_PREFIX}/lib/libxmlsec1.a" \
    && test -f "${DEPS_PREFIX}/lib/libxml2.a" \
    && test -f "${DEPS_PREFIX}/lib/libssl.a" \
    && test -f "${DEPS_PREFIX}/lib/libcrypto.a"

ARG RUST_VERSION=1.98
ENV RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal \
        --default-toolchain "${RUST_VERSION}" \
    && chmod -R a+rX /opt/rustup /opt/cargo

# PKG_CONFIG_ALL_STATIC: the libxml crate and samael's build script both find
#   their library with pkg-config, and this makes them link the archives.
# OPENSSL_NO_VENDOR + OPENSSL_DIR + OPENSSL_STATIC: openssl-sys links the
#   OpenSSL above instead of building its own.
ENV PATH="${DEPS_PREFIX}/bin:/opt/cargo/bin:${PATH}" \
    PKG_CONFIG_PATH="${DEPS_PREFIX}/lib/pkgconfig" \
    PKG_CONFIG_ALL_STATIC=1 \
    OPENSSL_NO_VENDOR=1 \
    OPENSSL_DIR="${DEPS_PREFIX}" \
    OPENSSL_STATIC=1

WORKDIR /work
