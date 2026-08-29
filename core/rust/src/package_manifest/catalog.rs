use super::{PackageCatalogAdmission, PackageManifestError};

pub(super) fn admit(
    format: &str,
    source: &str,
) -> Result<PackageCatalogAdmission, PackageManifestError> {
    if format != "std.typed.catalog/2" {
        return Err(PackageManifestError::new(
            "package/catalog-unsupported",
            format!("unsupported :schema/catalog :format {format}"),
        ));
    }
    let value: serde_json::Value = serde_json::from_str(source).map_err(|error| {
        PackageManifestError::new(
            "package/catalog-invalid",
            format!("catalog is not valid JSON: {error}"),
        )
    })?;
    // Semantic `std.typed` catalog validation is a source-package concern.
    // The host validates format identity, JSON syntax, and the archive digest;
    // package publication supplies the language-level proof without embedding
    // that library in every native artifact.
    let report = serde_json::to_string(&value).map_err(|error| {
        PackageManifestError::new(
            "package/catalog-invalid",
            format!("cannot canonicalise catalog JSON: {error}"),
        )
    })?;
    Ok(PackageCatalogAdmission {
        format: format.to_owned(),
        report,
    })
}
