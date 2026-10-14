//! Hard stop before a map is read into memory or uploaded.
//!
//! The largest retail package that already loads is DownTown: 304 MB on disk,
//! 2.63 million vertices, 1.89 million triangles, and no portable collision
//! (retail collision lives in an RWCM archive). The documented custom import
//! laru is a 68 MB FBX, 0.76 million vertices, 0.50 million triangles, and
//! 0.50 million collision triangles.
//!
//! Helsinki Kallio is a 1.47 GB FBX that imports to a 1.12 GB package: 11.0
//! million vertices, 9.23 million triangles, and 9.23 million collision
//! triangles. Decoding that keeps a second copy for the GPU. On an 8 GB
//! laptop the device allocation fails, and the next frame's view and cluster
//! uniform buffers then panic. These caps sit above DownTown and laru and
//! below that city. They are not a promise that every file under the cap fits
//! in VRAM.

/// Import reads the whole FBX, inflates its arrays, then builds the package.
/// 768 MB is above laru (68 MB) and below both Helsinki exports (991 MB and 1.47 GB).
pub const MAX_IMPORT_SOURCE_BYTES: u64 = 768 * 1024 * 1024;

/// 640 MB is above DownTown (304 MB) and below the Helsinki package (1.12 GB).
pub const MAX_PACKAGE_BYTES: u64 = 640 * 1024 * 1024;

/// Above DownTown's 2.63 million vertices, below Helsinki's 11.0 million.
pub const MAX_VERTICES: u64 = 4_000_000;

/// Above DownTown's 1.89 million triangles, below Helsinki's 9.23 million.
pub const MAX_TRIANGLES: u64 = 3_000_000;

/// Above laru's 0.50 million collision triangles, below Helsinki's 9.23 million.
/// Retail maps store collision in an RWCM archive, so this count is zero for them.
pub const MAX_COLLISION_TRIANGLES: u64 = 3_000_000;

/// `None` when the FBX may be read. The file itself is not opened here.
pub fn reject_import_source(file_bytes: u64) -> Option<String> {
    if file_bytes <= MAX_IMPORT_SOURCE_BYTES {
        return None;
    }
    Some(format!(
        "This FBX is {} MB, over the {} MB import limit, so it was not read. A file this large inflates into a mesh the GPU cannot hold. Nothing was imported.",
        mebibytes(file_bytes),
        mebibytes(MAX_IMPORT_SOURCE_BYTES)
    ))
}

/// `None` when the package file may be read.
pub fn reject_package_file(file_bytes: u64) -> Option<String> {
    if file_bytes <= MAX_PACKAGE_BYTES {
        return None;
    }
    Some(format!(
        "This map file is {} MB, over the {} MB limit, so it was not read.",
        mebibytes(file_bytes),
        mebibytes(MAX_PACKAGE_BYTES)
    ))
}

/// `None` when the header counts are small enough to decode.
pub fn reject_package_shape(vertices: u64, triangles: u64, collision_triangles: u64) -> Option<String> {
    if vertices > MAX_VERTICES {
        return Some(format!(
            "This map has {vertices} vertices. The limit is {MAX_VERTICES}, so it was not loaded."
        ));
    }
    if triangles > MAX_TRIANGLES {
        return Some(format!(
            "This map has {triangles} triangles. The limit is {MAX_TRIANGLES}, so it was not loaded."
        ));
    }
    if collision_triangles > MAX_COLLISION_TRIANGLES {
        return Some(format!(
            "This map has {collision_triangles} collision triangles. The limit is {MAX_COLLISION_TRIANGLES}, so it was not loaded."
        ));
    }
    None
}

fn mebibytes(bytes: u64) -> u64 {
    (bytes + 512 * 1024) / (1024 * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_import_and_package_are_refused_and_retail_sizes_pass() {
        let city = reject_import_source(1_545_792_236).unwrap();
        assert!(city.contains("1474 MB"), "{city}");
        assert!(city.contains("768 MB"), "{city}");
        assert!(city.contains("Nothing was imported"), "{city}");
        assert!(reject_import_source(71_701_068).is_none(), "laru FBX");
        assert!(reject_import_source(1_039_254_912).is_some(), "helsinki centre FBX");

        let package = reject_package_file(1_174_041_708).unwrap();
        assert!(package.contains("1120 MB"), "{package}");
        assert!(package.contains("640 MB"), "{package}");
        assert!(package.contains("not read"), "{package}");
        assert!(reject_package_file(318_843_790).is_none(), "DownTown file");
        assert!(reject_package_file(4_937_754).is_none(), "StartPark file");
        assert!(reject_package_file(63_347_003).is_none(), "laru package");

        assert!(reject_package_shape(2_626_756, 1_888_894, 0).is_none(), "DownTown");
        assert!(reject_package_shape(15_957, 13_176, 0).is_none(), "StartPark");
        assert!(reject_package_shape(757_241, 496_302, 496_304).is_none(), "laru");
        let vertices = reject_package_shape(10_972_270, 9_228_548, 9_228_550).unwrap();
        assert!(vertices.contains("10972270 vertices"), "{vertices}");
        assert!(vertices.contains("4000000"), "{vertices}");
        assert!(reject_package_shape(1_000, MAX_TRIANGLES + 1, 0)
            .unwrap()
            .contains("triangles"));
        assert!(reject_package_shape(1_000, 1_000, MAX_COLLISION_TRIANGLES + 1)
            .unwrap()
            .contains("collision"));
    }
}
