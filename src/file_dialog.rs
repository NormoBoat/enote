use std::path::PathBuf;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn folder() -> Result<Option<PathBuf>, String> {
    Ok(rfd::AsyncFileDialog::new()
        .pick_folder()
        .await
        .map(|file| file.path().to_owned()))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub async fn folder() -> Result<Option<PathBuf>, String> {
    Err("A native mobile folder picker is not connected yet".into())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn open() -> Result<Option<PathBuf>, String> {
    Ok(rfd::AsyncFileDialog::new()
        .pick_file()
        .await
        .map(|file| file.path().to_owned()))
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub async fn save(directory: Option<PathBuf>, name: String) -> Result<Option<PathBuf>, String> {
    let mut dialog = rfd::AsyncFileDialog::new().set_file_name(name);
    if let Some(directory) = directory {
        dialog = dialog.set_directory(directory);
    }
    Ok(dialog.save_file().await.map(|file| file.path().to_owned()))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub async fn open() -> Result<Option<PathBuf>, String> {
    Err("A native mobile file picker is not connected yet".into())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub async fn save(_directory: Option<PathBuf>, _name: String) -> Result<Option<PathBuf>, String> {
    Err("A native mobile file picker is not connected yet".into())
}
