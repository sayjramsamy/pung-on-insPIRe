extern crate capnpc;
extern crate cc;
extern crate cmake;

use std::path::PathBuf;

fn main() {

    // Compile RPC schema file into rust code
    capnpc::CompilerCommand::new()
        .src_prefix("schema")
        .file("schema/pung.capnp")
        .run().expect("schema compiler command");


    // Compile and link pung C++ PIR shim
    cc::Build::new()
                 .file("src/pir/cpp/pungPIR.cpp")
                 .include("deps/xpir/")
                 .flag("-std=c++11")
                 .flag("-fopenmp")
                 .flag("-Wno-unused-parameter")
                 .pic(true)
                 .cpp(true)
                 .compile("libpung_pir.a");

    // Compile and link XPIR c++ shim
    let dst = cmake::Config::new("deps/xpir")
                             .define("CMAKE_BUILD_TYPE", "Release")
                             .define("MULTI_THREAD", "OFF")
                             .define("PERF_TIMERS", "OFF")
                             .build();

    println!("cargo:rustc-link-search=native={}/build/pir", dst.display());
    println!("cargo:rustc-link-lib=static=pir_static");

    // Dynamic libraries needed by XPIR
    println!("cargo:rustc-link-lib=gomp");
    println!("cargo:rustc-link-lib=gmp");
    println!("cargo:rustc-link-lib=mpfr");
    println!("cargo:rustc-link-lib=boost_thread");

    // --- SealPIR backend ---
    let seal_install = PathBuf::from("deps/SEAL/install");
    let seal_include = seal_install.join("include").join("SEAL-3.6");
    let sealpir_src = PathBuf::from("deps/SealPIR/src");

    // SealPIR has no CMake install target (and no stable ABI to link against
    // as a prebuilt lib), so its sources are compiled directly alongside our
    // shim rather than built separately and linked.
    cc::Build::new()
        .cpp(true)
        .file("src/pir/cpp/pungSealPIR.cpp")
        .file(sealpir_src.join("pir.cpp"))
        .file(sealpir_src.join("pir_client.cpp"))
        .file(sealpir_src.join("pir_server.cpp"))
        .include(&sealpir_src)
        .include(&seal_include)
        .flag("-std=c++17")
        .flag("-O3")
        .pic(true)
        .compile("pungsealpir");

    println!(
        "cargo:rustc-link-search=native={}",
        seal_install.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=seal-3.6");
    println!("cargo:rustc-link-lib=pthread");
}
