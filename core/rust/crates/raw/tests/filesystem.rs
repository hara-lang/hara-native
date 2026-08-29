use hara_native_raw::file::{logical_normalise, FileProvider, MemoryFileProvider};

#[test]
fn raw_crate_links_the_shared_filesystem_module_with_bytecode_vm() {
    assert_eq!(
        logical_normalise("technology//hara").unwrap(),
        "/technology/hara"
    );
    let files = MemoryFileProvider::new("raw-crate");
    assert!(files.exists_value("/").unwrap());
}
