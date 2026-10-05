// Compiles src/face.cpp against the system dlib (pacman: dlib). DLIB_PREFIX points at another dlib install, e.g. an
// unpacked package, for building before dlib is installed.
//
// OpenBLAS (pacman: openblas) is linked ahead of dlib, so dlib's cblas calls
// bind to it rather than to the reference BLAS Arch's dlib is built against:
// the face network runs ~7x faster (108 -> 15 ms) with the same output
// (descriptors within 3e-7). Nothing system-wide changes.
use std::env;

fn main() {
    println!("cargo:rerun-if-changed=src/face.cpp");
    println!("cargo:rerun-if-env-changed=DLIB_PREFIX");
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file("src/face.cpp")
        .std("c++17")
        .flag("-O3")
        // dlib's image code is all templates, so it is compiled here: let it use AVX2
        // (every Panther Lake / x86-64-v3 CPU has it).
        .flag("-march=x86-64-v3")
        .flag_if_supported("-Wno-deprecated-declarations")
        // dlib's config.h already turns on BLAS and LAPACK, as the library was built
        .define("DLIB_NO_GUI_SUPPORT", None);
    if let Ok(prefix) = env::var("DLIB_PREFIX") {
        build.flag(format!("-isystem{prefix}/include"));
        println!("cargo:rustc-link-search=native={prefix}/lib");
    }
    build.compile("face");
    for lib in ["openblas", "dlib"] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
    println!("cargo:rustc-link-lib=dylib=stdc++");
}
