//! The `entry` module is a fundamental building block of Proof of History. It contains a
//! unique ID that is the hash of the Entry before it, plus the hash of the
//! transactions within it. Entries cannot be reordered, and its field `num_hashes`
//! represents an approximate amount of time since the last Entry was created.
use crate::packet::{Blob, SharedBlob, BLOB_DATA_SIZE};
use crate::poh::Poh;
use crate::result::Result;
use bincode::{deserialize, serialized_size};
use chrono::prelude::Utc;
use rayon::prelude::*;
use solana_budget_api::budget_instruction;
use solana_sdk::hash::{Hash, Hasher};
use solana_sdk::signature::{Keypair, KeypairUtil};
use solana_sdk::transaction::Transaction;
use std::borrow::Borrow;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock};

pub type EntrySender = Sender<Vec<Entry>>;
pub type EntryReceiver = Receiver<Vec<Entry>>;

/// Each Entry contains three pieces of data. The `num_hashes` field is the number
/// of hashes performed since the previous entry.  The `hash` field is the result
/// of hashing `hash` from the previous entry `num_hashes` times.  The `transactions`
/// field points to Transactions that took place shortly before `hash` was generated.
///
/// If you divide `num_hashes` by the amount of time it takes to generate a new hash, you
/// get a duration estimate since the last Entry. Since processing power increases
/// over time, one should expect the duration `num_hashes` represents to decrease proportionally.
/// An upper bound on Duration can be estimated by assuming each hash was generated by the
/// world's fastest processor at the time the entry was recorded. Or said another way, it
/// is physically not possible for a shorter duration to have occurred if one assumes the
/// hash was computed by the world's fastest processor at that time. The hash chain is both
/// a Verifiable Delay Function (VDF) and a Proof of Work (not to be confused with Proof of
/// Work consensus!)

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct Entry {
    /// The number of hashes since the previous Entry ID.
    pub num_hashes: u64,

    /// The SHA-256 hash `num_hashes` after the previous Entry ID.
    pub hash: Hash,

    /// An unordered list of transactions that were observed before the Entry ID was
    /// generated. They may have been observed before a previous Entry ID but were
    /// pushed back into this list to ensure deterministic interpretation of the ledger.
    pub transactions: Vec<Transaction>,
}

impl Entry {
    /// Creates the next Entry `num_hashes` after `start_hash`.
    pub fn new(prev_hash: &Hash, num_hashes: u64, transactions: Vec<Transaction>) -> Self {
        assert!(Self::serialized_to_blob_size(&transactions) <= BLOB_DATA_SIZE as u64);

        if num_hashes == 0 && transactions.is_empty() {
            Entry {
                num_hashes: 0,
                hash: *prev_hash,
                transactions,
            }
        } else if num_hashes == 0 {
            // If you passed in transactions, but passed in num_hashes == 0, then
            // next_hash will generate the next hash and set num_hashes == 1
            let hash = next_hash(prev_hash, 1, &transactions);
            Entry {
                num_hashes: 1,
                hash,
                transactions,
            }
        } else {
            // Otherwise, the next Entry `num_hashes` after `start_hash`.
            // If you wanted a tick for instance, then pass in num_hashes = 1
            // and transactions = empty
            let hash = next_hash(prev_hash, num_hashes, &transactions);
            Entry {
                num_hashes,
                hash,
                transactions,
            }
        }
    }

    pub fn to_shared_blob(&self) -> SharedBlob {
        let blob = self.to_blob();
        Arc::new(RwLock::new(blob))
    }

    pub fn to_blob(&self) -> Blob {
        Blob::from_serializable(&vec![&self])
    }

    /// return serialized_size of a vector with a single Entry for given TXs
    ///  since Blobs carry Vec<Entry>...
    /// calculate the total without actually constructing the full Entry (which
    ///  would require a clone() of the transactions)
    pub fn serialized_to_blob_size(transactions: &[Transaction]) -> u64 {
        let txs_size: u64 = transactions
            .iter()
            .map(|tx| serialized_size(tx).unwrap())
            .sum();

        serialized_size(&vec![Entry {
            num_hashes: 0,
            hash: Hash::default(),
            transactions: vec![],
        }])
        .unwrap()
            + txs_size
    }

    pub fn new_mut(
        start_hash: &mut Hash,
        num_hashes: &mut u64,
        transactions: Vec<Transaction>,
    ) -> Self {
        assert!(Self::serialized_to_blob_size(&transactions) <= BLOB_DATA_SIZE as u64);

        let entry = Self::new(start_hash, *num_hashes, transactions);
        *start_hash = entry.hash;
        *num_hashes = 0;

        entry
    }

    #[cfg(test)]
    pub fn new_tick(num_hashes: u64, hash: &Hash) -> Self {
        Entry {
            num_hashes,
            hash: *hash,
            transactions: vec![],
        }
    }

    /// Verifies self.hash is the result of hashing a `start_hash` `self.num_hashes` times.
    /// If the transaction is not a Tick, then hash that as well.
    pub fn verify(&self, start_hash: &Hash) -> bool {
        let ref_hash = next_hash(start_hash, self.num_hashes, &self.transactions);
        if self.hash != ref_hash {
            warn!(
                "next_hash is invalid expected: {:?} actual: {:?}",
                self.hash, ref_hash
            );
            return false;
        }
        true
    }

    pub fn is_tick(&self) -> bool {
        self.transactions.is_empty()
    }
}

pub fn hash_transactions(transactions: &[Transaction]) -> Hash {
    // a hash of a slice of transactions only needs to hash the signatures
    let mut hasher = Hasher::default();
    transactions.iter().for_each(|tx| {
        if !tx.signatures.is_empty() {
            hasher.hash(&tx.signatures[0].as_ref());
        }
    });
    hasher.result()
}

/// Creates the hash `num_hashes` after `start_hash`. If the transaction contains
/// a signature, the final hash will be a hash of both the previous ID and
/// the signature.  If num_hashes is zero and there's no transaction data,
///  start_hash is returned.
fn next_hash(start_hash: &Hash, num_hashes: u64, transactions: &[Transaction]) -> Hash {
    if num_hashes == 0 && transactions.is_empty() {
        return *start_hash;
    }

    let mut poh = Poh::new(*start_hash, 0);
    poh.hash(num_hashes.saturating_sub(1));
    if transactions.is_empty() {
        poh.tick().hash
    } else {
        poh.record(hash_transactions(transactions)).hash
    }
}

pub fn reconstruct_entries_from_blobs<I>(blobs: I) -> Result<(Vec<Entry>, u64)>
where
    I: IntoIterator,
    I::Item: Borrow<Blob>,
{
    let mut entries: Vec<Entry> = vec![];
    let mut num_ticks = 0;

    for blob in blobs.into_iter() {
        let new_entries: Vec<Entry> = {
            let msg_size = blob.borrow().size();
            deserialize(&blob.borrow().data()[..msg_size])?
        };

        let num_new_ticks: u64 = new_entries.iter().map(|entry| entry.is_tick() as u64).sum();
        num_ticks += num_new_ticks;
        entries.extend(new_entries)
    }
    Ok((entries, num_ticks))
}

// an EntrySlice is a slice of Entries
pub trait EntrySlice {
    /// Verifies the hashes and counts of a slice of transactions are all consistent.
    fn verify(&self, start_hash: &Hash) -> bool;
    fn to_shared_blobs(&self) -> Vec<SharedBlob>;
    fn to_blobs(&self) -> Vec<Blob>;
    fn to_single_entry_blobs(&self) -> Vec<Blob>;
    fn to_single_entry_shared_blobs(&self) -> Vec<SharedBlob>;
}

impl EntrySlice for [Entry] {
    fn verify(&self, start_hash: &Hash) -> bool {
        let genesis = [Entry {
            num_hashes: 0,
            hash: *start_hash,
            transactions: vec![],
        }];
        let entry_pairs = genesis.par_iter().chain(self).zip(self);
        entry_pairs.all(|(x0, x1)| {
            let r = x1.verify(&x0.hash);
            if !r {
                warn!(
                    "entry invalid!: x0: {:?}, x1: {:?} num txs: {}",
                    x0.hash,
                    x1.hash,
                    x1.transactions.len()
                );
            }
            r
        })
    }

    fn to_blobs(&self) -> Vec<Blob> {
        split_serializable_chunks(
            &self,
            BLOB_DATA_SIZE as u64,
            &|s| bincode::serialized_size(&s).unwrap(),
            &mut |entries: &[Entry]| Blob::from_serializable(entries),
        )
    }

    fn to_shared_blobs(&self) -> Vec<SharedBlob> {
        self.to_blobs()
            .into_iter()
            .map(|b| Arc::new(RwLock::new(b)))
            .collect()
    }

    fn to_single_entry_shared_blobs(&self) -> Vec<SharedBlob> {
        self.to_single_entry_blobs()
            .into_iter()
            .map(|b| Arc::new(RwLock::new(b)))
            .collect()
    }

    fn to_single_entry_blobs(&self) -> Vec<Blob> {
        self.iter().map(Entry::to_blob).collect()
    }
}

pub fn next_entry_mut(start: &mut Hash, num_hashes: u64, transactions: Vec<Transaction>) -> Entry {
    let entry = Entry::new(&start, num_hashes, transactions);
    *start = entry.hash;
    entry
}

pub fn num_will_fit<T, F>(serializables: &[T], max_size: u64, serialized_size: &F) -> usize
where
    F: Fn(&[T]) -> u64,
{
    if serializables.is_empty() {
        return 0;
    }
    let mut num = serializables.len();
    let mut upper = serializables.len();
    let mut lower = 1; // if one won't fit, we have a lot of TODOs
    loop {
        let next;
        if serialized_size(&serializables[..num]) <= max_size {
            next = (upper + num) / 2;
            lower = num;
        } else {
            if num == 1 {
                // if not even one will fit, bail
                num = 0;
                break;
            }
            next = (lower + num) / 2;
            upper = num;
        }
        // same as last time
        if next == num {
            break;
        }
        num = next;
    }
    num
}

pub fn split_serializable_chunks<T, R, F1, F2>(
    serializables: &[T],
    max_size: u64,
    serialized_size: &F1,
    converter: &mut F2,
) -> Vec<R>
where
    F1: Fn(&[T]) -> u64,
    F2: FnMut(&[T]) -> R,
{
    let mut result = vec![];
    let mut chunk_start = 0;
    while chunk_start < serializables.len() {
        let chunk_end =
            chunk_start + num_will_fit(&serializables[chunk_start..], max_size, serialized_size);
        result.push(converter(&serializables[chunk_start..chunk_end]));
        chunk_start = chunk_end;
    }

    result
}

/// Creates the next entries for given transactions, outputs
/// updates start_hash to hash of last Entry, sets num_hashes to 0
fn next_entries_mut(
    start_hash: &mut Hash,
    num_hashes: &mut u64,
    transactions: Vec<Transaction>,
) -> Vec<Entry> {
    split_serializable_chunks(
        &transactions[..],
        BLOB_DATA_SIZE as u64,
        &Entry::serialized_to_blob_size,
        &mut |txs: &[Transaction]| Entry::new_mut(start_hash, num_hashes, txs.to_vec()),
    )
}

/// Creates the next Entries for given transactions
pub fn next_entries(
    start_hash: &Hash,
    num_hashes: u64,
    transactions: Vec<Transaction>,
) -> Vec<Entry> {
    let mut hash = *start_hash;
    let mut num_hashes = num_hashes;
    next_entries_mut(&mut hash, &mut num_hashes, transactions)
}

pub fn create_ticks(num_ticks: u64, mut hash: Hash) -> Vec<Entry> {
    let mut ticks = Vec::with_capacity(num_ticks as usize);
    for _ in 0..num_ticks {
        let new_tick = next_entry_mut(&mut hash, 1, vec![]);
        ticks.push(new_tick);
    }

    ticks
}

pub fn make_tiny_test_entries_from_hash(start: &Hash, num: usize) -> Vec<Entry> {
    let keypair = Keypair::new();
    let pubkey = keypair.pubkey();

    let mut hash = *start;
    let mut num_hashes = 0;
    (0..num)
        .map(|_| {
            let ix = budget_instruction::apply_timestamp(&pubkey, &pubkey, &pubkey, Utc::now());
            let tx = Transaction::new_signed_instructions(&[&keypair], vec![ix], *start);
            Entry::new_mut(&mut hash, &mut num_hashes, vec![tx])
        })
        .collect()
}

pub fn make_tiny_test_entries(num: usize) -> Vec<Entry> {
    let zero = Hash::default();
    let one = solana_sdk::hash::hash(&zero.as_ref());
    make_tiny_test_entries_from_hash(&one, num)
}

pub fn make_large_test_entries(num_entries: usize) -> Vec<Entry> {
    let zero = Hash::default();
    let one = solana_sdk::hash::hash(&zero.as_ref());
    let keypair = Keypair::new();
    let pubkey = keypair.pubkey();

    let ix = budget_instruction::apply_timestamp(&pubkey, &pubkey, &pubkey, Utc::now());
    let tx = Transaction::new_signed_instructions(&[&keypair], vec![ix], one);

    let serialized_size = serialized_size(&tx).unwrap();
    let num_txs = BLOB_DATA_SIZE / serialized_size as usize;
    let txs = vec![tx; num_txs];
    let entry = next_entries(&one, 1, txs)[0].clone();
    vec![entry; num_entries]
}

#[cfg(test)]
pub fn make_consecutive_blobs(
    id: &solana_sdk::pubkey::Pubkey,
    num_blobs_to_make: u64,
    start_height: u64,
    start_hash: Hash,
    addr: &std::net::SocketAddr,
) -> Vec<SharedBlob> {
    let entries = create_ticks(num_blobs_to_make, start_hash);

    let blobs = entries.to_single_entry_shared_blobs();
    let mut index = start_height;
    for blob in &blobs {
        let mut blob = blob.write().unwrap();
        blob.set_index(index);
        blob.set_id(id);
        blob.meta.set_addr(addr);
        index += 1;
    }
    blobs
}

#[cfg(test)]
/// Creates the next Tick or Transaction Entry `num_hashes` after `start_hash`.
pub fn next_entry(prev_hash: &Hash, num_hashes: u64, transactions: Vec<Transaction>) -> Entry {
    assert!(num_hashes > 0 || transactions.is_empty());
    Entry {
        num_hashes,
        hash: next_hash(prev_hash, num_hashes, &transactions),
        transactions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::Entry;
    use crate::packet::{to_blobs, BLOB_DATA_SIZE, PACKET_DATA_SIZE};
    use solana_sdk::hash::hash;
    use solana_sdk::instruction::Instruction;
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::signature::{Keypair, KeypairUtil};
    use solana_sdk::system_transaction;
    use solana_vote_api::vote_instruction;
    use solana_vote_api::vote_state::Vote;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn create_sample_payment(keypair: &Keypair, hash: Hash) -> Transaction {
        let pubkey = keypair.pubkey();
        let ixs = budget_instruction::payment(&pubkey, &pubkey, 1);
        Transaction::new_signed_instructions(&[keypair], ixs, hash)
    }

    fn create_sample_timestamp(keypair: &Keypair, hash: Hash) -> Transaction {
        let pubkey = keypair.pubkey();
        let ix = budget_instruction::apply_timestamp(&pubkey, &pubkey, &pubkey, Utc::now());
        Transaction::new_signed_instructions(&[keypair], vec![ix], hash)
    }

    fn create_sample_apply_signature(keypair: &Keypair, hash: Hash) -> Transaction {
        let pubkey = keypair.pubkey();
        let ix = budget_instruction::apply_signature(&pubkey, &pubkey, &pubkey);
        Transaction::new_signed_instructions(&[keypair], vec![ix], hash)
    }

    fn create_sample_vote(keypair: &Keypair, hash: Hash) -> Transaction {
        let pubkey = keypair.pubkey();
        let ix = vote_instruction::vote(&pubkey, vec![Vote::new(1)]);
        Transaction::new_signed_instructions(&[keypair], vec![ix], hash)
    }

    #[test]
    fn test_entry_verify() {
        let zero = Hash::default();
        let one = hash(&zero.as_ref());
        assert!(Entry::new_tick(0, &zero).verify(&zero)); // base case, never used
        assert!(!Entry::new_tick(0, &zero).verify(&one)); // base case, bad
        assert!(next_entry(&zero, 1, vec![]).verify(&zero)); // inductive step
        assert!(!next_entry(&zero, 1, vec![]).verify(&one)); // inductive step, bad
    }

    #[test]
    fn test_transaction_reorder_attack() {
        let zero = Hash::default();

        // First, verify entries
        let keypair = Keypair::new();
        let tx0 = system_transaction::create_user_account(&keypair, &keypair.pubkey(), 0, zero, 0);
        let tx1 = system_transaction::create_user_account(&keypair, &keypair.pubkey(), 1, zero, 0);
        let mut e0 = Entry::new(&zero, 0, vec![tx0.clone(), tx1.clone()]);
        assert!(e0.verify(&zero));

        // Next, swap two transactions and ensure verification fails.
        e0.transactions[0] = tx1; // <-- attack
        e0.transactions[1] = tx0;
        assert!(!e0.verify(&zero));
    }

    #[test]
    fn test_witness_reorder_attack() {
        let zero = Hash::default();

        // First, verify entries
        let keypair = Keypair::new();
        let tx0 = create_sample_timestamp(&keypair, zero);
        let tx1 = create_sample_apply_signature(&keypair, zero);
        let mut e0 = Entry::new(&zero, 0, vec![tx0.clone(), tx1.clone()]);
        assert!(e0.verify(&zero));

        // Next, swap two witness transactions and ensure verification fails.
        e0.transactions[0] = tx1; // <-- attack
        e0.transactions[1] = tx0;
        assert!(!e0.verify(&zero));
    }

    #[test]
    fn test_next_entry() {
        let zero = Hash::default();
        let tick = next_entry(&zero, 1, vec![]);
        assert_eq!(tick.num_hashes, 1);
        assert_ne!(tick.hash, zero);

        let tick = next_entry(&zero, 0, vec![]);
        assert_eq!(tick.num_hashes, 0);
        assert_eq!(tick.hash, zero);

        let keypair = Keypair::new();
        let tx0 = create_sample_timestamp(&keypair, zero);
        let entry0 = next_entry(&zero, 1, vec![tx0.clone()]);
        assert_eq!(entry0.num_hashes, 1);
        assert_eq!(entry0.hash, next_hash(&zero, 1, &vec![tx0]));
    }

    #[test]
    #[should_panic]
    fn test_next_entry_panic() {
        let zero = Hash::default();
        let keypair = Keypair::new();
        let tx = system_transaction::create_user_account(&keypair, &keypair.pubkey(), 0, zero, 0);
        next_entry(&zero, 0, vec![tx]);
    }

    #[test]
    fn test_serialized_to_blob_size() {
        let zero = Hash::default();
        let keypair = Keypair::new();
        let tx = system_transaction::create_user_account(&keypair, &keypair.pubkey(), 0, zero, 0);
        let entry = next_entry(&zero, 1, vec![tx.clone()]);
        assert_eq!(
            Entry::serialized_to_blob_size(&[tx]),
            serialized_size(&vec![entry]).unwrap() // blobs are Vec<Entry>
        );
    }

    #[test]
    fn test_verify_slice() {
        solana_logger::setup();
        let zero = Hash::default();
        let one = hash(&zero.as_ref());
        assert!(vec![][..].verify(&zero)); // base case
        assert!(vec![Entry::new_tick(0, &zero)][..].verify(&zero)); // singleton case 1
        assert!(!vec![Entry::new_tick(0, &zero)][..].verify(&one)); // singleton case 2, bad
        assert!(vec![next_entry(&zero, 0, vec![]); 2][..].verify(&zero)); // inductive step

        let mut bad_ticks = vec![next_entry(&zero, 0, vec![]); 2];
        bad_ticks[1].hash = one;
        assert!(!bad_ticks.verify(&zero)); // inductive step, bad
    }

    fn blob_sized_entries(num_entries: usize) -> Vec<Entry> {
        // rough guess
        let mut magic_len = BLOB_DATA_SIZE
            - serialized_size(&vec![Entry {
                num_hashes: 0,
                hash: Hash::default(),
                transactions: vec![],
            }])
            .unwrap() as usize;

        loop {
            let entries = vec![Entry {
                num_hashes: 0,
                hash: Hash::default(),
                transactions: vec![Transaction::new_unsigned_instructions(vec![
                    Instruction::new(Pubkey::default(), &vec![0u8; magic_len as usize], vec![]),
                ])],
            }];
            let size = serialized_size(&entries).unwrap() as usize;
            if size < BLOB_DATA_SIZE {
                magic_len += BLOB_DATA_SIZE - size;
            } else if size > BLOB_DATA_SIZE {
                magic_len -= size - BLOB_DATA_SIZE;
            } else {
                break;
            }
        }
        vec![
            Entry {
                num_hashes: 0,
                hash: Hash::default(),
                transactions: vec![Transaction::new_unsigned_instructions(vec![
                    Instruction::new(Pubkey::default(), &vec![0u8; magic_len], vec![]),
                ])],
            };
            num_entries
        ]
    }

    #[test]
    fn test_entries_to_blobs() {
        solana_logger::setup();
        let entries = blob_sized_entries(10);

        let blobs = entries.to_blobs();
        for blob in &blobs {
            assert_eq!(blob.size(), BLOB_DATA_SIZE);
        }

        assert_eq!(reconstruct_entries_from_blobs(blobs).unwrap().0, entries);
    }

    #[test]
    fn test_multiple_entries_to_blobs() {
        solana_logger::setup();
        let num_blobs = 10;
        let serialized_size =
            bincode::serialized_size(&make_tiny_test_entries_from_hash(&Hash::default(), 1))
                .unwrap();

        let num_entries = (num_blobs * BLOB_DATA_SIZE as u64) / serialized_size;
        let entries = make_tiny_test_entries_from_hash(&Hash::default(), num_entries as usize);

        let blob_q = entries.to_blobs();

        assert_eq!(blob_q.len() as u64, num_blobs);
        assert_eq!(reconstruct_entries_from_blobs(blob_q).unwrap().0, entries);
    }

    #[test]
    fn test_bad_blobs_attack() {
        solana_logger::setup();
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 8000);
        let blobs_q = to_blobs(vec![(0, addr)]).unwrap(); // <-- attack!
        assert!(reconstruct_entries_from_blobs(blobs_q).is_err());
    }

    #[test]
    fn test_next_entries() {
        solana_logger::setup();
        let hash = Hash::default();
        let next_hash = solana_sdk::hash::hash(&hash.as_ref());
        let keypair = Keypair::new();
        let vote_account = Keypair::new();
        let tx_small = create_sample_vote(&vote_account, next_hash);
        let tx_large = create_sample_payment(&keypair, next_hash);

        let tx_small_size = serialized_size(&tx_small).unwrap() as usize;
        let tx_large_size = serialized_size(&tx_large).unwrap() as usize;
        let entry_size = serialized_size(&Entry {
            num_hashes: 0,
            hash: Hash::default(),
            transactions: vec![],
        })
        .unwrap() as usize;
        assert!(tx_small_size < tx_large_size);
        assert!(tx_large_size < PACKET_DATA_SIZE);

        let threshold = (BLOB_DATA_SIZE - entry_size) / tx_small_size;

        // verify no split
        let transactions = vec![tx_small.clone(); threshold];
        let entries0 = next_entries(&hash, 0, transactions.clone());
        assert_eq!(entries0.len(), 1);
        assert!(entries0.verify(&hash));

        // verify the split with uniform transactions
        let transactions = vec![tx_small.clone(); threshold * 2];
        let entries0 = next_entries(&hash, 0, transactions.clone());
        assert_eq!(entries0.len(), 2);
        assert!(entries0.verify(&hash));

        // verify the split with small transactions followed by large
        // transactions
        let mut transactions = vec![tx_small.clone(); BLOB_DATA_SIZE / tx_small_size];
        let large_transactions = vec![tx_large.clone(); BLOB_DATA_SIZE / tx_large_size];

        transactions.extend(large_transactions);

        let entries0 = next_entries(&hash, 0, transactions.clone());
        assert!(entries0.len() >= 2);
        assert!(entries0.verify(&hash));
    }

    #[test]
    fn test_num_will_fit_empty() {
        let serializables: Vec<u32> = vec![];
        let result = num_will_fit(&serializables[..], 8, &|_| 4);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_num_will_fit() {
        let serializables_vec: Vec<u8> = (0..10).map(|_| 1).collect();
        let serializables = &serializables_vec[..];
        let sum = |i: &[u8]| (0..i.len()).into_iter().sum::<usize>() as u64;
        // sum[0] is = 0, but sum[0..1] > 0, so result contains 1 item
        let result = num_will_fit(serializables, 0, &sum);
        assert_eq!(result, 1);

        // sum[0..3] is <= 8, but sum[0..4] > 8, so result contains 3 items
        let result = num_will_fit(serializables, 8, &sum);
        assert_eq!(result, 4);

        // sum[0..1] is = 1, but sum[0..2] > 0, so result contains 2 items
        let result = num_will_fit(serializables, 1, &sum);
        assert_eq!(result, 2);

        // sum[0..9] = 45, so contains all items
        let result = num_will_fit(serializables, 45, &sum);
        assert_eq!(result, 10);

        // sum[0..8] <= 44, but sum[0..9] = 45, so contains all but last item
        let result = num_will_fit(serializables, 44, &sum);
        assert_eq!(result, 9);

        // sum[0..9] <= 46, but contains all items
        let result = num_will_fit(serializables, 46, &sum);
        assert_eq!(result, 10);

        // too small to fit a single u64
        let result = num_will_fit(&[0u64], (std::mem::size_of::<u64>() - 1) as u64, &|i| {
            (std::mem::size_of::<u64>() * i.len()) as u64
        });
        assert_eq!(result, 0);
    }
}
