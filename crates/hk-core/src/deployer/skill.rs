//! Skill directory/file deployment.

use super::*;

pub fn deploy_skill(source_path: &Path, target_skill_dir: &Path) -> Result<String, HkError> {
    std::fs::create_dir_all(target_skill_dir)?;
    if source_path.is_dir() {
        let dir_name = source_path
            .file_name()
            .ok_or_else(|| HkError::Validation("Invalid source path".into()))?
            .to_string_lossy()
            .to_string();
        let dest = target_skill_dir.join(&dir_name);
        copy_dir_recursive(source_path, &dest)?;
        Ok(dir_name)
    } else {
        let file_name = source_path
            .file_name()
            .ok_or_else(|| HkError::Validation("Invalid source path".into()))?
            .to_string_lossy()
            .to_string();
        let dest = target_skill_dir.join(&file_name);
        std::fs::copy(source_path, &dest)?;
        Ok(file_name)
    }
}

pub(super) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), HkError> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        // TOCTOU-safe symlink check: use symlink_metadata (lstat) instead of
        // following symlinks. Re-check right before the copy to close the race
        // window between readdir and the actual file operation.
        let meta = match std::fs::symlink_metadata(&src_path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!(
                    "[hk] warning: cannot read metadata for {}: {e}",
                    src_path.display()
                );
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            eprintln!("[hk] warning: skipping symlink: {}", src_path.display());
            continue;
        }
        if meta.file_type().is_dir() {
            if entry.file_name() == ".git" {
                continue;
            }
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

