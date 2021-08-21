use tokio::io::AsyncWriteExt;

pub async fn write_bytes(folder: &str, hash: &[u8], bytes: &[u8]) -> std::io::Result<()> {
    let hex_hash = hex::encode(&hash);
    tracing::debug!("writing {} to {}", hex_hash, folder);

    let hash_folder = std::path::PathBuf::from(folder)
        .join(&hex_hash[0..2])
        .join(&hex_hash[2..4]);

    match tokio::fs::create_dir_all(&hash_folder).await {
        Ok(_) => (),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(err) => return Err(err),
    }

    let file_path = hash_folder.join(hex_hash);
    let mut file = match tokio::fs::File::create(file_path).await {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(err) => return Err(err),
    };

    file.write_all(bytes).await?;

    Ok(())
}
