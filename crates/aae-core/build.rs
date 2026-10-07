// Compiles the Android Emulator's gRPC definitions with protox, a protobuf
// compiler written in Rust, so building AAE never needs protoc installed.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protos = [
        "proto/emulator_controller.proto",
        "proto/snapshot_service.proto",
    ];
    for proto in &protos {
        println!("cargo:rerun-if-changed={proto}");
    }
    let descriptors = protox::compile(protos, ["proto"])?;
    tonic_prost_build::configure()
        .build_server(false)
        .compile_fds(descriptors)?;
    Ok(())
}
