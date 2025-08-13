use std::{env, path::PathBuf};

use tokio::fs;

pub async fn write_port_file(port: u16) -> std::io::Result<PathBuf> {
    let dir = env::temp_dir().join("vibe-kanban");
    let path = dir.join("vibe-kanban.port");
    tracing::debug!("Writing port {} to {:?}", port, path);
    fs::create_dir_all(&dir).await?;
    fs::write(&path, port.to_string()).await?;
    Ok(path)
}

/// Conditionally writes the port file based on environment.
/// In debug builds, always writes the file.
/// In release builds, only writes if ENABLE_PORT_FILE env var is set.
pub async fn maybe_write_port_file(port: u16) -> std::io::Result<()> {
    let should_write = if cfg!(debug_assertions) {
        true
    } else {
        env::var_os("ENABLE_PORT_FILE").is_some()
    };

    if should_write {
        write_port_file(port).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::env;

    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn test_maybe_write_port_file_with_env_var() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("TMPDIR", temp_dir.path());
            env::set_var("ENABLE_PORT_FILE", "1");
        }

        let result = maybe_write_port_file(8080).await;
        assert!(result.is_ok());

        // Check if file was created
        let port_file = temp_dir.path().join("vibe-kanban").join("vibe-kanban.port");
        assert!(port_file.exists());

        let content = tokio::fs::read_to_string(&port_file).await.unwrap();
        assert_eq!(content, "8080");

        unsafe {
            env::remove_var("ENABLE_PORT_FILE");
        }
    }

    #[tokio::test]
    async fn test_maybe_write_port_file_without_env_var() {
        let temp_dir = TempDir::new().unwrap();
        unsafe {
            env::set_var("TMPDIR", temp_dir.path());
            env::remove_var("ENABLE_PORT_FILE");
        }

        let result = maybe_write_port_file(8080).await;
        assert!(result.is_ok());

        // In release builds without env var, file should not be created
        // In debug builds, file is always created
        let port_file = temp_dir.path().join("vibe-kanban").join("vibe-kanban.port");
        #[cfg(debug_assertions)]
        assert!(port_file.exists());
        #[cfg(not(debug_assertions))]
        assert!(!port_file.exists());
    }
}
