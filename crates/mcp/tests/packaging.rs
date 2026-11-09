#[test]
fn packaging_builds_and_ships_the_mcp_binary() {
    let packager = include_str!("../../../Packager.toml");

    assert!(
        packager.contains("-p vrc-photon-mcp"),
        "beforePackagingCommand must build vrc-photon-mcp"
    );
    assert!(
        packager
            .lines()
            .any(|line| line.trim() == "path = \"vrc-photon-mcp\""),
        "[[binaries]] must include vrc-photon-mcp"
    );
}
