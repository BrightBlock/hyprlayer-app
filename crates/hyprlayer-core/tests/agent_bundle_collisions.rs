use std::path::Path;

use hyprlayer_core::agent_bundle::resolve_collisions;

/// The codex target is linked its skills out of the *claude* bundle, so clearing its
/// collisions means running the claude manifest against `~/.codex`.
#[test]
fn clears_a_codex_skills_collision_sourced_from_the_claude_bundle() {
    let tmp = tempfile::tempdir().unwrap();
    let bundle = tmp.path().join("claude");
    let codex = tmp.path().join("dot-codex");

    std::fs::create_dir_all(bundle.join("skills/commit")).unwrap();
    std::fs::write(bundle.join("skills/commit/SKILL.md"), "new").unwrap();
    let sha = "8ecf4d4d1e4e7d4e0b7a0a3b0a8a3d0e3e2b6a1c9f8d7e6c5b4a3928170615ff";
    std::fs::write(
        bundle.join("manifest.json"),
        format!(r#"{{"files":[{{"path":"skills/commit/SKILL.md","sha256":"{sha}"}}]}}"#),
    )
    .unwrap();

    std::fs::create_dir_all(codex.join("skills/commit")).unwrap();
    std::fs::write(codex.join("skills/commit/SKILL.md"), "old").unwrap();

    let report = resolve_collisions(&bundle, &codex, "20260827-120000");

    assert_eq!(report.backed_up, vec!["skills/commit".to_string()]);
    assert!(!codex.join("skills/commit").exists());
    assert_eq!(
        std::fs::read_to_string(codex.join("skills/commit.bak-20260827-120000/SKILL.md")).unwrap(),
        "old"
    );
    assert!(Path::new(&codex).join("skills").is_dir());
}
