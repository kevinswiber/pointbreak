#[test]
fn windows_installer_loads_zip_support_before_reading_archives() {
    let installer =
        std::fs::read_to_string("scripts/install.ps1").expect("read Windows installer source");
    let load = installer
        .find("Add-Type -AssemblyName System.IO.Compression.FileSystem")
        .expect("installer loads Windows PowerShell zip support");
    let open = installer
        .find("[IO.Compression.ZipFile]::OpenRead")
        .expect("installer validates zip archive layout");

    assert!(
        load < open,
        "zip support must load before archive validation"
    );
}

#[test]
fn windows_installer_selftest_uses_the_documented_powershell_runtime() {
    let justfile = std::fs::read_to_string("Justfile").expect("read Justfile");

    assert!(justfile.contains(
        "powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/install-selftest.ps1"
    ));
}

#[test]
fn windows_installer_checksum_does_not_require_get_file_hash() {
    let installer =
        std::fs::read_to_string("scripts/install.ps1").expect("read Windows installer source");
    let selftest = std::fs::read_to_string("scripts/install-selftest.ps1")
        .expect("read Windows installer self-test source");

    for source in [&installer, &selftest] {
        assert!(!source.contains("Get-FileHash"));
        assert!(source.contains("[Security.Cryptography.SHA256]::Create()"));
    }
}

#[test]
fn installers_require_exact_release_build_identity_with_one_v070_transition() {
    let unix = std::fs::read_to_string("scripts/install.sh").expect("read Unix installer source");
    let windows =
        std::fs::read_to_string("scripts/install.ps1").expect("read Windows installer source");
    let unix_selftest =
        std::fs::read_to_string("scripts/install-selftest.sh").expect("read Unix self-test");
    let windows_selftest =
        std::fs::read_to_string("scripts/install-selftest.ps1").expect("read Windows self-test");

    for source in [&unix, &windows] {
        for field in ["source", "commit", "describe", "dirty"] {
            assert!(
                source.contains(field),
                "installer does not validate build.{field}"
            );
        }
        assert!(source.contains("0.7.0"));
        assert!(source.contains("pointbreak.version") || source.contains(r"pointbreak\.version"));
    }

    assert!(!windows.contains("$expectedProperties"));
    assert!(windows.contains("$document.build"));
    assert!(unix.contains("expected_version\" = \"0.7.0"));

    for selftest in [&unix_selftest, &windows_selftest] {
        for scenario in [
            "wrong-tag",
            "dirty-build",
            "package-build",
            "short-commit",
            "malformed-document",
            "null-build",
            "missing-build-after-v0.7.0",
            "legacy-v0.7.0",
            "additive-fields-and-order",
        ] {
            assert!(
                selftest.contains(scenario),
                "installer self-test does not cover {scenario}"
            );
        }
    }
}
