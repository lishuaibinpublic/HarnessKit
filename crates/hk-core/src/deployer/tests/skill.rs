//! Skill deploy and dir-copy tests.

use super::*;
use tempfile::TempDir;

#[test]
fn test_deploy_skill_directory() {
    let src_dir = TempDir::new().unwrap();
    let skill_dir = src_dir.path().join("my-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "# My Skill").unwrap();
    std::fs::write(skill_dir.join("helper.py"), "print('hello')").unwrap();

    let target_dir = TempDir::new().unwrap();
    let name = deploy_skill(&skill_dir, target_dir.path()).unwrap();
    assert_eq!(name, "my-skill");
    assert!(target_dir.path().join("my-skill").join("SKILL.md").exists());
    assert!(target_dir
        .path()
        .join("my-skill")
        .join("helper.py")
        .exists());
}

#[test]
fn test_deploy_skill_file() {
    let src_dir = TempDir::new().unwrap();
    let skill_file = src_dir.path().join("solo-skill.md");
    std::fs::write(&skill_file, "# Solo Skill").unwrap();

    let target_dir = TempDir::new().unwrap();
    let name = deploy_skill(&skill_file, target_dir.path()).unwrap();
    assert_eq!(name, "solo-skill.md");
    assert!(target_dir.path().join("solo-skill.md").exists());
}

#[test]
fn test_deploy_skill_skips_git_dir() {
    let src_dir = TempDir::new().unwrap();
    let skill_dir = src_dir.path().join("git-skill");
    std::fs::create_dir_all(skill_dir.join(".git")).unwrap();
    std::fs::write(skill_dir.join(".git").join("HEAD"), "ref: refs/heads/main").unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "# Git Skill").unwrap();

    let target_dir = TempDir::new().unwrap();
    deploy_skill(&skill_dir, target_dir.path()).unwrap();
    assert!(target_dir
        .path()
        .join("git-skill")
        .join("SKILL.md")
        .exists());
    assert!(!target_dir.path().join("git-skill").join(".git").exists());
}

#[test]
fn test_copy_dir_recursive_skips_symlinks() {
    let src_dir = TempDir::new().unwrap();
    let skill_dir = src_dir.path().join("my-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "# My Skill").unwrap();

    // Create a symlink to a file outside the skill directory
    let secret = src_dir.path().join("secret.txt");
    std::fs::write(&secret, "TOP SECRET").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&secret, skill_dir.join("link-to-secret")).unwrap();

    let target_dir = TempDir::new().unwrap();
    deploy_skill(&skill_dir, target_dir.path()).unwrap();

    assert!(target_dir.path().join("my-skill").join("SKILL.md").exists());
    // Symlink should NOT have been followed/copied
    #[cfg(unix)]
    assert!(!target_dir
        .path()
        .join("my-skill")
        .join("link-to-secret")
        .exists());
}

#[cfg(unix)]
#[test]
fn test_copy_dir_recursive_uses_symlink_metadata_recheck() {
    // Verify that copy_dir_recursive uses symlink_metadata to avoid following
    // symlinks even if a TOCTOU race replaces a file with a symlink between
    // the readdir check and the copy. We test by creating a symlinked directory
    // and verifying it's not traversed.
    let src_dir = TempDir::new().unwrap();
    let skill_dir = src_dir.path().join("skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), "# Skill").unwrap();

    // Create a symlinked subdirectory pointing outside
    let outside = TempDir::new().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "SECRET DATA").unwrap();
    std::os::unix::fs::symlink(outside.path(), skill_dir.join("evil-link")).unwrap();

    let dst = TempDir::new().unwrap();
    let dst_dir = dst.path().join("skill");
    copy_dir_recursive(&skill_dir, &dst_dir).unwrap();

    assert!(dst_dir.join("SKILL.md").exists());
    // The symlinked directory should be skipped entirely
    assert!(!dst_dir.join("evil-link").exists());
}
