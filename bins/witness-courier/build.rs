//! Build script for `witness-courier`: generates gRPC client stubs from `witness.proto`.

fn main() {
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["witness.proto"], &["."])
        .expect("failed to compile witness.proto");
}