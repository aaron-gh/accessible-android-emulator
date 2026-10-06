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
    // Prism, linked statically, uses these system frameworks on macOS, but
    // prismer's static build doesn't ask the linker for them.
    let macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    if macos && std::env::var_os("CARGO_FEATURE_SPEECH").is_some() {
        for framework in ["AVFoundation", "AppKit", "IOKit", "ApplicationServices"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }

    let descriptors = protox::compile(protos, ["proto"])?;
    tonic_prost_build::configure()
        .build_server(false)
        .compile_fds(descriptors)?;
    Ok(())
}
