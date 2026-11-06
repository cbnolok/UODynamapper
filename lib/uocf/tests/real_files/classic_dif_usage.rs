//! The dif usage checker over every Classic `client.exe` under the client roots: the builds up to 5.0.9.1 apply the
//! land and statics patches (their lookups have callers), 6.0.14.3 loads the lists without a lookup, and every 7.0
//! build loads them and has dead lookups. A Classic executable not in the table must not be `Unknown`.
//!
//! The roots come from `UO_CLIENT_ROOTS` (see `client_exe_facet_dimensions`); the files are only read.

use std::collections::HashSet;

use crate::client_exe_facet_dimensions::{client_exes, exe_kind, ExeKind};

use uocf::client::classic_difs::{inspect_classic_difs, ClassicDifUsage, DifKind, DifStatus};
use uocf::client::facet_dimensions::{ClientExeInspection, ClientFamily};

/// A build of the table: its SHA-256, its label and whether its lookups are called.
struct Expected {
    sha256: &'static str,
    name: &'static str,
    applied: bool,
    /// Whether it has a lookup per kind (none in 6.0.14.3).
    lookups: bool,
}

const EXPECTED: [Expected; 9] = [
    Expected {
        sha256: "aa21bb3ea16b6c5e00837569466b052e978baa6e38fd945e2694af4f14fa28c2",
        name: "UO Gold",
        applied: true,
        lookups: true,
    },
    Expected {
        sha256: "17ab35e10bf38077fa101938a4a0b85d1b4caa505245fc6bc83d3b131a0c2918",
        name: "Mondain's Legacy folder",
        applied: true,
        lookups: true,
    },
    Expected {
        sha256: "6250963022ec03855dedf89176c54dc7ad603e230b415edaa4b08d1509f568e9",
        name: "5.0.9.1",
        applied: true,
        lookups: true,
    },
    Expected {
        sha256: "9104cea0ea14bd8ee81ee5aa111f1e3f39eca50ac33f818813202fadfdd3718d",
        name: "6.0.14.3",
        applied: false,
        lookups: false,
    },
    Expected {
        sha256: "27eef2c0a4679f204b923f29dd1506bb2c7e649410054aefad680264bb504c9e",
        name: "7.0.20.0",
        applied: false,
        lookups: true,
    },
    Expected {
        sha256: "6507e703b0ae65fa7ba174f53f86317ac3b736f0b12c1a8e86240b4cb03b5146",
        name: "7.0.29.97 client_tc",
        applied: false,
        lookups: true,
    },
    Expected {
        sha256: "8b3af41390d34930ac1dc914b865edb7961c5448e0592b43a912790814987940",
        name: "7.0.50.0",
        applied: false,
        lookups: true,
    },
    Expected {
        sha256: "4d857f2f5f8bf350bc50a87ee66b2236bfa480e712e9546c740ff04f9106690a",
        name: "7.0.103.0",
        applied: false,
        lookups: true,
    },
    Expected {
        sha256: "694afa07dc850ec9ffbe156dcd7d9ff5fa2eb57f6b398a16d43aa63f3a9bfe40",
        name: "7.0.115.0 (Classic fp folder)",
        applied: false,
        lookups: true,
    },
];

fn check(expected: &Expected, usage: &ClassicDifUsage) {
    let status = if expected.applied { DifStatus::Applied } else { DifStatus::LoadedNotApplied };
    for kind in DifKind::ALL {
        assert_eq!(usage.status(kind), &status, "{} {}: {:#?}", expected.name, kind.label(), usage.evidence);
        let loaders: Vec<_> = usage.loaders.iter().filter(|loader| loader.kind == kind).collect();
        assert_eq!(loaders.len(), 1, "{} {}: one loader", expected.name, kind.label());
        assert!(loaders[0].function_va.is_some() && loaders[0].maps.count.is_some(), "{}: {:?}", expected.name, loaders[0]);
        let lookups: Vec<_> = usage.lookups.iter().filter(|lookup| lookup.kind == kind).collect();
        assert_eq!(lookups.len(), usize::from(expected.lookups), "{} {}: {lookups:?}", expected.name, kind.label());
        assert!(lookups.iter().all(|lookup| lookup.callers.is_empty() != expected.applied));
    }
    assert!(!usage.evidence.iter().any(|line| line.contains("other references")), "{}: every reference is explained", expected.name);
}

#[test]
#[ignore = "needs real clients: set UO_CLIENT_ROOTS and run with --ignored"]
fn classic_dif_usage_matches_the_traced_builds() {
    let mut seen = HashSet::new();
    for path in client_exes().into_iter().filter(|exe| exe_kind(exe) == ExeKind::Classic) {
        let bytes = std::fs::read(&path).unwrap();
        let Ok(inspection) = ClientExeInspection::inspect(&bytes) else {
            continue;
        };
        if inspection.family != ClientFamily::Classic {
            continue;
        }
        let usage = inspect_classic_difs(&bytes, &inspection.pe);
        println!("{}: land {}, statics {}", path.display(), usage.land.describe(), usage.statics.describe());
        for line in &usage.evidence {
            println!("  {line}");
        }
        match EXPECTED.iter().find(|expected| expected.sha256 == inspection.sha256) {
            Some(expected) => {
                check(expected, &usage);
                seen.insert(expected.sha256);
            }
            None => {
                for kind in DifKind::ALL {
                    assert!(usage.status(kind).applies().is_some(), "{} {}: {:#?}", path.display(), kind.label(), usage.evidence);
                }
            }
        }
    }
    assert!(!seen.is_empty(), "no build of the table under the client roots");
}
