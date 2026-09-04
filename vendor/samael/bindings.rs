//!
//! XmlSec Bindings Generation
//!
use bindgen::Builder as BindgenBuilder;

use pkg_config::Config as PkgConfig;

use std::env;
use std::path::PathBuf;
use std::process::Command;

const BINDINGS: &str = "xmlsec_bindings.rs";

fn main() {
    // Tell the compiler about our custom cfg flags
    println!("cargo:rustc-check-cfg=cfg(xmlsec_dynamic)");
    println!("cargo:rustc-check-cfg=cfg(xmlsec_static)");
    println!("cargo:rerun-if-changed=bindings.h");

    if env::var_os("CARGO_FEATURE_XMLSEC").is_none() {
        return;
    }

    let path_out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let path_bindings = path_out.join(BINDINGS);

    // The autotools build installs `xmlsec1-config`, which reports the
    // exact flags the library was built with. MSVC has neither that script
    // nor pkg-config; vcpkg is the supported way to get xmlsec1 there, so
    // ask vcpkg instead.
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let clang_args = if msvc {
        configure_msvc()
    } else {
        configure_autotools()
    };

    let bindbuild = BindgenBuilder::default()
        .header("bindings.h")
        .clang_args(clang_args)
        .layout_tests(true)
        .generate_comments(true);

    let bindings = bindbuild.generate().expect("Unable to generate bindings");

    bindings
        .write_to_file(path_bindings)
        .expect("Couldn't write bindings!");
}

/// Locate xmlsec1 via `xmlsec1-config` + pkg-config, and emit the link
/// directives for it. Returns the clang arguments bindgen needs.
fn configure_autotools() -> Vec<String> {
    // Determine which API/ABI is available on this platform:
    let cflags = fetch_xmlsec_config_flags();
    let dynamic = if cflags
        .iter()
        .any(|s| s == "-DXMLSEC_CRYPTO_DYNAMIC_LOADING=1")
    {
        println!("cargo:rustc-cfg=xmlsec_dynamic");
        true
    } else {
        println!("cargo:rustc-cfg=xmlsec_static");
        false
    };

    if !dynamic {
        println!("cargo:rustc-link-lib=xmlsec1-openssl"); // -lxmlsec1-openssl
    }
    println!("cargo:rustc-link-lib=xmlsec1"); // -lxmlsec1
    println!("cargo:rustc-link-lib=xml2"); // -lxml2
    println!("cargo:rustc-link-lib=ssl"); // -lssl
    println!("cargo:rustc-link-lib=crypto"); // -lcrypto

    PkgConfig::new()
        .probe("xmlsec1")
        .expect("Could not find xmlsec1 using pkg-config");

    let mut args = cflags;
    args.extend(fetch_xmlsec_config_libs());
    args
}

/// Locate xmlsec1 via vcpkg and emit the link directives for it. Returns
/// the clang arguments bindgen needs.
///
/// The vcpkg `xmlsec` port builds the same two libraries the autotools
/// build produces, `xmlsec1` and `xmlsec1-openssl`. vcpkg-rs reads the
/// installed-port status database, so one `find_package` emits both plus
/// the libxml2 and OpenSSL libraries they were linked against, under
/// their MSVC names — `libxml2.lib` and `libcrypto.lib`, which the bare
/// `-lxml2` / `-lcrypto` names of the autotools path would not resolve.
fn configure_msvc() -> Vec<String> {
    let library = vcpkg::Config::new()
        .emit_includes(true)
        .find_package("xmlsec")
        .expect(
            "Could not find xmlsec using vcpkg. Install it with \
             `vcpkg install xmlsec:x64-windows-static-md` and set VCPKG_ROOT \
             to the vcpkg tree.",
        );

    // The port leaves crypto dynamic loading off, so the OpenSSL backend is
    // linked in rather than resolved through xmlsec's own dlopen shim. That
    // is the same shape as the non-dynamic autotools build.
    println!("cargo:rustc-cfg=xmlsec_static");

    let mut args: Vec<String> = XMLSEC_MSVC_DEFINES
        .iter()
        .map(|define| format!("-D{}", define))
        .collect();
    for dir in &library.include_paths {
        args.push(format!("-I{}", dir.display()));
    }
    args
}

/// Preprocessor defines the vcpkg `xmlsec` port marks PUBLIC on its two
/// targets. A CMake consumer inherits them; bindgen has to be told.
/// Source of truth: `ports/xmlsec/CMakeLists.txt` in microsoft/vcpkg.
///
/// `XMLSEC_STATIC` carries the weight. Without it every declaration in the
/// headers is `__declspec(dllimport)`, and linking those against the static
/// libraries the default triplet builds fails.
///
/// The port's generated `xmlsec1.pc` is not usable as a substitute: it
/// interpolates a CMake list into a single `-D`, producing
/// `-DXMLSEC_NO_XSLT;XMLSEC_CRYPTO_OPENSSL;...`, and points `-I` at an
/// `xmlsec1` include subdirectory the port does not create.
const XMLSEC_MSVC_DEFINES: &[&str] = &[
    "XMLSEC_STATIC",
    "XMLSEC_CRYPTO_OPENSSL",
    "XMLSEC_NO_CRYPTO_DYNAMIC_LOADING",
    "XMLSEC_NO_XSLT",
    "XMLSEC_NO_FTP",
    "XMLSEC_NO_HTTP",
    "XMLSEC_NO_MD5",
    "XMLSEC_NO_RIPEMD160",
    "XMLSEC_NO_GOST",
    "XMLSEC_NO_GOST2012",
    "XMLSEC_NO_MLDSA",
    "XMLSEC_NO_SLHDSA",
];

fn fetch_xmlsec_config_flags() -> Vec<String> {
    let out = Command::new("xmlsec1-config")
        .arg("--cflags")
        .output()
        .expect("Failed to get --cflags from xmlsec1-config. Is xmlsec1 installed?")
        .stdout;

    args_from_output(out)
}

fn fetch_xmlsec_config_libs() -> Vec<String> {
    let out = Command::new("xmlsec1-config")
        .arg("--libs")
        .output()
        .expect("Failed to get --libs from xmlsec1-config. Is xmlsec1 installed?")
        .stdout;

    args_from_output(out)
}

fn args_from_output(args: Vec<u8>) -> Vec<String> {
    let decoded = String::from_utf8(args).expect("Got invalid UTF8 from xmlsec1-config");

    decoded.split_whitespace().map(|p| p.to_owned()).collect()
}
