use prost_build::Config;

fn main() {
    Config::new()
        .protoc_arg("--experimental_allow_proto3_optional")
        .type_attribute(
            ".",
            "#[derive(::serde::Deserialize, ::serde::Serialize)] #[serde(rename_all = \"snake_case\")]",
        )
        .compile_protos(&["src/proto/gtfs-realtime.proto"], &["src/proto"])
        .expect("Failed to compile proto files");
}
