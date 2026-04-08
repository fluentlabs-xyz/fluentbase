//! Build script for `witness-courier`:
//! - generates gRPC client stubs from `witness.proto`
//! - links system libbrotlienc for deterministic brotli compression

fn main() {
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["witness.proto"], &["."])
        .expect("failed to compile witness.proto");

    pkg_config::Config::new()
        .atleast_version("1.1.0")
        .probe("libbrotlienc")
        .expect("system libbrotlienc not found — install libbrotli-dev or brotli");
}