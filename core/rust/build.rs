// Build-time metadata for the HAL-free native host.
//
// A Hara source distribution is a signed package consumed by this host. It
// must never be discovered from a sibling checkout or copied into a native
// release, so the generated embedded-resource tables are deliberately empty.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo must provide OUT_DIR"))
        .join("embedded_hal.rs");
    fs::write(
        output,
        "pub(crate) static EMBEDDED_HAL_RESOURCES: &[(&str, &str, &str)] = &[];\n\
         pub(crate) static EMBEDDED_CLI_RESOURCES: &[(&str, &str, &str)] = &[];\n\
         #[cfg(test)] pub(crate) static FOUNDATION_BOOTSTRAP_INVENTORY: &[&str] = &[];\n\
         #[cfg(test)] pub(crate) static CLI_BOOTSTRAP_INVENTORY: &[&str] = &[];\n",
    )
    .expect("cannot write embedded resource declaration");
}
