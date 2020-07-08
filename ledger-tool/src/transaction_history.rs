use log::*;
use solana_sdk::{clock::Slot, pubkey::Pubkey, signature::Signature, sysvar::is_sysvar_id};
use solana_transaction_status::ConfirmedBlock;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, Write},
    path::Path,
};
use crate::bs91;

/// Checks if information about the given confirmed slot is present
pub fn block_exists(slot: Slot) -> bool {
    let block_dir = Path::new("h/block");
    let path = block_dir.join(format!("{}", bs91::encode_u64(slot)));
    path.exists()
}

pub fn write_block(slot: Slot, block: &ConfirmedBlock) -> Result<usize, io::Error> {
    let data = bincode::serialize(&block).unwrap();
    let compressed_data = crate::compression::compress_best(&data).unwrap();

    let block_dir = Path::new("h/block");

    let path = block_dir.join(format!("{}", bs91::encode_u64(slot)));
    //error!("block {} -> {}", path.display(), slot);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let tmp_path = path.with_extension("tmp");

    let mut bytes_written = 0;
    {
        let mut file = File::create(&tmp_path).unwrap();
        let data = bincode::serialize(&compressed_data.method).unwrap();
        file.write_all(&data)?;
        bytes_written += data.len();

        file.write_all(&compressed_data.data)?;
        bytes_written += compressed_data.data.len();
    }
    //fs::rename(tmp_path, path).unwrap();
    assert!(tmp_path.exists(), format!("{} does not exist: {}", tmp_path.display(), bytes_written));
    let _ = fs::remove_file(&path);
    assert!(!path.exists());
    fs::rename(&tmp_path, &path).unwrap_or_else(|err| panic!("{:?}: {} -> {}", err, tmp_path.display(), path.display()));
    assert!(path.exists());
    Ok(bytes_written)
}


pub fn write_transaction_map(
    signature: &Signature,
    locator: (Slot, u32),
) -> Result<usize, io::Error> {
    let data = bincode::serialize(&locator).unwrap();

    let tx_map_dir = Path::new("h/tx-map");
    fs::create_dir_all(&tx_map_dir).unwrap();

    let path = tx_map_dir.join(bs91::encode_signature(signature));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let tmp_path = path.with_extension("tmp");
    //error!("{} -> {:?} - {} bytes", path.display(), locator, data.len());

    let mut bytes_written = 0;
    {
        let mut file = File::create(&tmp_path).unwrap();
        file.write_all(&data)?;
        bytes_written += data.len();
    }
    //fs::rename(tmp_path, path).unwrap();
    assert!(tmp_path.exists(), format!("{} does not exist: {}", tmp_path.display(), bytes_written));
    let _ = fs::remove_file(&path);
    assert!(!path.exists());
    fs::rename(&tmp_path, &path).unwrap_or_else(|err| panic!("{:?}: {} -> {}", err, tmp_path.display(), path.display()));
    assert!(path.exists());
    Ok(bytes_written)
}

pub fn write_by_addr(
    path: &Path,
/*
    address: &Pubkey,
    slot: Slot,
    */
    signatures: &[Signature],
) -> Result<usize, io::Error> {
    //let path = by_addr_dir.join(format!("{}/{}", bs91::encode_pubkey(address), bs91::encode_u64(!slot)));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let tmp_path = path.with_extension("tmp");
    //error!("{} - {}", path.display(), signatures.len());

    let mut bytes_written = 0;
    {
        let mut file = File::create(&tmp_path).unwrap();
        for signature in signatures {
            let data = signature.as_ref();
            file.write_all(data)?;
            bytes_written += data.len();
        }
    }
    //fs::rename(tmp_path, path).unwrap();
    assert!(tmp_path.exists(), format!("{} does not exist: {}", tmp_path.display(), bytes_written));
    let _ = fs::remove_file(&path);
    assert!(!path.exists());
    fs::rename(&tmp_path, &path).unwrap_or_else(|err| panic!("{:?}: {} -> {}", err, tmp_path.display(), path.display()));
    assert!(path.exists());
    Ok(bytes_written)
}

pub fn injest_block(slot: Slot, block: &ConfirmedBlock) -> Result<(), io::Error> {
    if block_exists(slot) {
        return Ok(());
    }

    let by_addr_dir = Path::new("h/by-addr");
//    fs::create_dir_all(&by_addr_basedir)?;

    let mut bytes_written = 0;

    let mut by_addr: HashMap<Pubkey, Vec<Signature>> = HashMap::new();
    for (index, transaction_with_meta) in block.transactions.iter().enumerate() {
        let transaction = transaction_with_meta
            .transaction
            .decode()
            .expect("transaction decode failed");
        let signature = transaction.signatures[0];

        for address in transaction.message.account_keys {
            if !is_sysvar_id(&address) {
                by_addr.entry(address).or_default().push(signature);
            }
        }

        bytes_written += write_transaction_map(&signature, (slot, index as u32))?;
    }

    let not_slot_base91 = bs91::encode_u64(!slot);
    for (address, signatures) in by_addr.into_iter() {
        let path = by_addr_dir.join(bs91::encode_pubkey(&address)).join(&not_slot_base91);
        bytes_written += write_by_addr(&path, &signatures)?;
    }
    bytes_written += write_block(slot, &block)?;

    debug!(
        "slot {}: {} transactions, {} bytes",
        slot,
        block.transactions.len(),
        bytes_written,
    );

    Ok(())
}
