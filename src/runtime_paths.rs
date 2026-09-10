//! macOS bundles keep resources immutable and user data outside the signed app.
//! Windows and development launches retain the existing working-directory layout.
use std::path::PathBuf;

#[cfg(any(target_os = "macos", test))]
fn resources_for_executable(exe: &std::path::Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    if macos.file_name()? != "MacOS" || contents.file_name()? != "Contents" {
        return None;
    }
    Some(contents.join("Resources"))
}

pub fn bundle_resources() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        return resources_for_executable(&std::env::current_exe().ok()?);
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

pub fn resource(relative: &str) -> PathBuf {
    bundle_resources().map_or_else(|| PathBuf::from(relative), |root| root.join(relative))
}

#[cfg(any(target_os = "macos", test))]
fn prepare_user_directory(
    resources: &std::path::Path,
    user: &std::path::Path,
) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(user.join("logs"))?;
    let defaults = resources.join("settings.json");
    if defaults.is_file() {
        let bytes = std::fs::read(defaults)?;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(user.join("settings.json"))
        {
            Ok(mut file) => file.write_all(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Called before settings, saves, threads or windows are created.
pub fn initialize() -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    if let Some(resources) = bundle_resources() {
        let home = std::env::var_os("HOME").ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "HOME is unavailable; cannot locate player data",
            )
        })?;
        let user = PathBuf::from(home).join("Library/Application Support/Voxel Project");
        prepare_user_directory(&resources, &user)?;
        std::env::set_current_dir(user)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_layout_and_development_launches_are_distinct() {
        let exe = PathBuf::from("Games/Voxel Project.app/Contents/MacOS/voxelproject");
        assert_eq!(
            resources_for_executable(&exe),
            Some(PathBuf::from("Games/Voxel Project.app/Contents/Resources"))
        );
        assert!(
            resources_for_executable(std::path::Path::new("target/release/voxelproject")).is_none()
        );
        assert!(
            resources_for_executable(std::path::Path::new("target/debug/voxelproject.exe"))
                .is_none()
        );
    }

    #[test]
    fn user_setup_preserves_preferences_and_never_modifies_bundle() {
        let root = std::env::temp_dir().join(format!(
            "voxel-path-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let resources = root.join("Resources");
        let user = root.join("Application Support/Voxel Project");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(resources.join("settings.json"), b"default").unwrap();
        prepare_user_directory(&resources, &user).unwrap();
        assert_eq!(
            std::fs::read(user.join("settings.json")).unwrap(),
            b"default"
        );
        std::fs::write(user.join("settings.json"), b"personal").unwrap();
        prepare_user_directory(&resources, &user).unwrap();
        assert_eq!(
            std::fs::read(user.join("settings.json")).unwrap(),
            b"personal"
        );
        assert_eq!(
            std::fs::read(resources.join("settings.json")).unwrap(),
            b"default"
        );
        assert!(user.join("logs").is_dir());
        std::fs::remove_dir_all(root).unwrap();
    }
}
