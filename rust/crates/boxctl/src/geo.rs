use serde_json::{json, Value};

use crate::download::{self, Policy};
use crate::error::{BoxError, Result};
use crate::paths::Paths;
use crate::util;

const GEOIP_URL: &str = "https://github.com/MetaCubeX/meta-rules-dat/raw/release/country-lite.mmdb";
const GEOSITE_URL: &str = "https://github.com/MetaCubeX/meta-rules-dat/raw/release/geosite.dat";

pub fn update(paths: &Paths, policy: &Policy) -> Result<Value> {
    util::ensure_dir(
        paths
            .geoip_file
            .parent()
            .ok_or_else(|| BoxError::internal("geo file has no parent"))?,
        true,
    )?;
    let geoip_tmp = paths.geoip_file.with_extension("mmdb.tmp");
    let geosite_tmp = paths.geosite_file.with_extension("dat.tmp");
    let _ = util::remove_file_if_exists(&geoip_tmp);
    let _ = util::remove_file_if_exists(&geosite_tmp);

    let geoip = match download::download_to_file(&geoip_tmp, GEOIP_URL, policy, None) {
        Ok(report) => report,
        Err(error) => {
            let _ = util::remove_file_if_exists(&geoip_tmp);
            return Err(error);
        }
    };
    let geosite = match download::download_to_file(&geosite_tmp, GEOSITE_URL, policy, None) {
        Ok(report) => report,
        Err(error) => {
            let _ = util::remove_file_if_exists(&geoip_tmp);
            let _ = util::remove_file_if_exists(&geosite_tmp);
            return Err(error);
        }
    };
    std::fs::rename(&geoip_tmp, &paths.geoip_file)
        .map_err(|error| BoxError::from(error).with_detail("cannot publish Country.mmdb"))?;
    std::fs::rename(&geosite_tmp, &paths.geosite_file).map_err(|error| {
        let _ = util::remove_file_if_exists(&paths.geoip_file);
        BoxError::from(error).with_detail("cannot publish GeoSite.dat")
    })?;
    Ok(json!({
        "geoip": {"bytes": geoip.bytes, "sha256": geoip.sha256},
        "geosite": {"bytes": geosite.bytes, "sha256": geosite.sha256}
    }))
}

#[cfg(test)]
mod tests {
    use super::{GEOIP_URL, GEOSITE_URL};

    #[test]
    fn geo_sources_are_pinned_https() {
        assert!(GEOIP_URL.starts_with("https://"));
        assert!(GEOSITE_URL.starts_with("https://"));
        assert!(GEOIP_URL.contains("MetaCubeX/meta-rules-dat"));
    }
}
