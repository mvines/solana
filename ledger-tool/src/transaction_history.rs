use log::*;
use solana_sdk::{clock::Slot, pubkey::Pubkey, signature::Signature, sysvar::is_sysvar_id};
use solana_transaction_status::ConfirmedBlock;
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, Write},
    path::Path,
};

fn slot_object_name(slot: Slot) -> String {
    let h = &format!("{:016x}", slot);
    format!(
        "{}/{}/{}/{}",
        h.get(0..4).unwrap(),
        h.get(4..8).unwrap(),
        h.get(8..12).unwrap(),
        h.get(12..16).unwrap()
    )
}

/// Checks if information about the given confirmed slot is present
pub fn block_exists(slot: Slot) -> bool {
    let block_dir = Path::new("block");
    let path = block_dir.join(format!("{}", slot_object_name(slot)));
    path.exists()
}

pub fn write_block(slot: Slot, block: &ConfirmedBlock) -> Result<usize, io::Error> {
    let data = bincode::serialize(&block).unwrap();
    let compressed_data = crate::compression::compress_best(&data).unwrap();

    let block_dir = Path::new("h/block");

    let path = block_dir.join(format!("{}", slot_object_name(slot)));
    fs::create_dir_all(path.parent().unwrap())?;
    let tmp_path = path.with_extension(".tmp");

    let mut bytes_written = 0;
    {
        let mut file = File::create(&tmp_path)?;
        let data = bincode::serialize(&compressed_data.method).unwrap();
        file.write_all(&data)?;
        bytes_written += data.len();

        file.write_all(&compressed_data.data)?;
        bytes_written += compressed_data.data.len();
    }
    fs::rename(tmp_path, path)?;
    Ok(bytes_written)
}

pub fn write_transaction_map(
    signature: &Signature,
    locator: (Slot, u32),
) -> Result<usize, io::Error> {
    let data = bincode::serialize(&locator).unwrap();

    let tx_map_dir = Path::new("h/tx-map");
    fs::create_dir_all(&tx_map_dir)?;

    let path = tx_map_dir.join(format!("{}", signature));
    fs::create_dir_all(path.parent().unwrap())?;
    let tmp_path = path.with_extension(".tmp");
    //error!("{} -> {:?} - {} bytes", path.display(), locator, data.len());

    let mut bytes_written = 0;
    {
        let mut file = File::create(&tmp_path)?;
        file.write_all(&data)?;
        bytes_written += data.len();
    }
    fs::rename(tmp_path, path)?;
    Ok(bytes_written)
}

pub fn write_by_addr(
    address: &Pubkey,
    slot: Slot,
    signatures: &[Signature],
) -> Result<usize, io::Error> {
    let by_addr_dir = Path::new("h/by-addr");
    fs::create_dir_all(&by_addr_dir)?;

    let path = by_addr_dir.join(format!("{}/{}", slot_object_name(!slot), address));
    fs::create_dir_all(path.parent().unwrap())?;
    let tmp_path = path.with_extension(".tmp");
    //error!("{} - {}", path.display(), signatures.len());

    let mut bytes_written = 0;
    {
        let mut file = File::create(&tmp_path)?;
        for signature in signatures {
            let data = signature.as_ref();
            file.write_all(data)?;
            bytes_written += data.len();
        }
    }
    fs::rename(tmp_path, path)?;
    Ok(bytes_written)
}

pub fn injest_block(slot: Slot, block: &ConfirmedBlock) -> Result<(), io::Error> {
    if block_exists(slot) {
        return Ok(());
    }

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
    for (address, signatures) in by_addr.into_iter() {
        bytes_written += write_by_addr(&address, slot, &signatures)?;
    }
    bytes_written += write_block(slot, &block)?;

    error!(
        "slot {}: {} transactions, {} bytes",
        slot,
        block.transactions.len(),
        bytes_written,
    );

    Ok(())
}
