//! Embeds the Windows manifest and version details in the Windows app.

fn main() {
    // Development builds say so; see windows/build.sh.
    println!("cargo:rerun-if-env-changed=AAE_BUILD_LABEL");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=res/aae.manifest");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let numbers: Vec<&str> = version.split(['.', '-']).take(3).chain(["0"]).collect();
    let numeric = numbers.join(",");
    let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("res/aae.manifest");
    let rc = format!(
        r#"#include <winuser.h>
#include <winver.h>

CREATEPROCESS_MANIFEST_RESOURCE_ID RT_MANIFEST "{manifest}"

VS_VERSION_INFO VERSIONINFO
FILEVERSION     {numeric}
PRODUCTVERSION  {numeric}
FILEOS          VOS_NT_WINDOWS32
FILETYPE        VFT_APP
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "FileDescription", "Accessible Android Emulator"
      VALUE "FileVersion", "{version}"
      VALUE "ProductName", "Accessible Android Emulator"
      VALUE "ProductVersion", "{version}"
      VALUE "InternalName", "AAE"
      VALUE "OriginalFilename", "AccessibleAndroidEmulator.exe"
      VALUE "LegalCopyright", "Apache License 2.0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        manifest = manifest.display().to_string().replace('\\', "/"),
    );
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("aae.rc");
    std::fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_required()
        .unwrap();
}
