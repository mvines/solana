use crate::cluster_info::ClusterInfo;
use crate::poh_recorder::PohRecorder;
use log::*;
use solana_metrics::{datapoint_warn, inc_new_counter_info};
use solana_runtime::{bank::Bank, bank_forks::BankForks};
use solana_sdk::{clock::Slot, pubkey::Pubkey, signature::Signature};
use std::sync::Mutex;
use std::{
    collections::HashMap,
    net::{SocketAddr, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
        Arc, RwLock,
    },
    thread::{self, Builder, JoinHandle},
    time::{Duration, Instant},
};

/// Maximum size of the transaction queue
const MAX_TRANSACTION_QUEUE_SIZE: usize = 10_000; // This seems like a lot but maybe it needs to be bigger one day

pub struct SendTransactionService {
    thread: JoinHandle<()>,
}

pub struct TransactionInfo {
    pub signature: Signature,
    pub wire_transaction: Vec<u8>,
    pub last_valid_slot: Slot,
    pub retry_enabled: bool,
}

impl TransactionInfo {
    pub fn new(signature: Signature, wire_transaction: Vec<u8>, last_valid_slot: Slot) -> Self {
        Self {
            signature,
            wire_transaction,
            last_valid_slot,
            retry_enabled: true,
        }
    }
}

pub struct LeaderInfo {
    cluster_info: Arc<ClusterInfo>,
    poh_recorder: Arc<Mutex<PohRecorder>>,
    recent_peers: HashMap<Pubkey, SocketAddr>,
}

impl LeaderInfo {
    pub fn new(cluster_info: Arc<ClusterInfo>, poh_recorder: Arc<Mutex<PohRecorder>>) -> Self {
        Self {
            cluster_info,
            poh_recorder,
            recent_peers: HashMap::new(),
        }
    }

    pub fn refresh_recent_peers(&mut self) {
        self.recent_peers = self
            .cluster_info
            .tpu_peers()
            .into_iter()
            .map(|ci| (ci.id, ci.tpu))
            .collect();
    }

    pub fn get_leader_tpu(&self) -> Option<&SocketAddr> {
        self.poh_recorder
            .lock()
            .unwrap()
            .leader_after_n_slots(0)
            .and_then(|leader| self.recent_peers.get(&leader))
    }
}

#[derive(Default, Debug, PartialEq)]
struct ProcessTransactionsResult {
    rooted: u64,
    expired: u64,
    retried: u64,
    failed: u64,
    retained: u64,
}

impl SendTransactionService {
    pub fn new(
        tpu_address: SocketAddr,
        bank_forks: &Arc<RwLock<BankForks>>,
        leader_info: Option<LeaderInfo>,
        exit: &Arc<AtomicBool>,
        receiver: Receiver<TransactionInfo>,
    ) -> Self {
        let thread = Self::retry_thread(
            tpu_address,
            receiver,
            bank_forks.clone(),
            leader_info,
            exit.clone(),
        );
        Self { thread }
    }

    fn retry_thread(
        tpu_address: SocketAddr,
        receiver: Receiver<TransactionInfo>,
        bank_forks: Arc<RwLock<BankForks>>,
        mut leader_info: Option<LeaderInfo>,
        exit: Arc<AtomicBool>,
    ) -> JoinHandle<()> {
        let mut last_status_check = Instant::now();
        let mut transactions = HashMap::new();
        let send_socket = UdpSocket::bind("0.0.0.0:0").unwrap();

        if let Some(leader_info) = leader_info.as_mut() {
            leader_info.refresh_recent_peers();
        }

        Builder::new()
            .name("send-tx-svc".to_string())
            .spawn(move || loop {
                if exit.load(Ordering::Relaxed) {
                    break;
                }

                if let Ok(transaction_info) = receiver.recv_timeout(Duration::from_secs(1)) {
                    let address = leader_info
                        .as_ref()
                        .and_then(|leader_info| leader_info.get_leader_tpu())
                        .unwrap_or(&tpu_address);
                    Self::send_transaction(
                        &send_socket,
                        address,
                        &transaction_info.wire_transaction,
                    );
                    if transaction_info.retry_enabled {
                        if transactions.len() < MAX_TRANSACTION_QUEUE_SIZE {
                            transactions.insert(transaction_info.signature, transaction_info);
                        } else {
                            datapoint_warn!("send_transaction_service-queue-overflow");
                        }
                    }
                }

                if Instant::now().duration_since(last_status_check).as_secs() >= 5 {
                    if !transactions.is_empty() {
                        datapoint_info!(
                            "send_transaction_service-queue-size",
                            ("len", transactions.len(), i64)
                        );
                        let bank_forks = bank_forks.read().unwrap();
                        let root_bank = bank_forks.root_bank();
                        let working_bank = bank_forks.working_bank();

                        let _result = Self::process_transactions(
                            &working_bank,
                            &root_bank,
                            &send_socket,
                            &tpu_address,
                            &mut transactions,
                            &leader_info,
                        );
                    }
                    last_status_check = Instant::now();
                    if let Some(leader_info) = leader_info.as_mut() {
                        leader_info.refresh_recent_peers();
                    }
                }
            })
            .unwrap()
    }

    fn process_transactions(
        working_bank: &Arc<Bank>,
        root_bank: &Arc<Bank>,
        send_socket: &UdpSocket,
        tpu_address: &SocketAddr,
        transactions: &mut HashMap<Signature, TransactionInfo>,
        leader_info: &Option<LeaderInfo>,
    ) -> ProcessTransactionsResult {
        let mut result = ProcessTransactionsResult::default();

        transactions.retain(|signature, transaction_info| {
            if root_bank.has_signature(signature) {
                info!("Transaction is rooted: {}", signature);
                result.rooted += 1;
                inc_new_counter_info!("send_transaction_service-rooted", 1);
                false
            } else if transaction_info.last_valid_slot < root_bank.slot() {
                info!("Dropping expired transaction: {}", signature);
                result.expired += 1;
                inc_new_counter_info!("send_transaction_service-expired", 1);
                false
            } else {
                match working_bank.get_signature_status_slot(signature) {
                    None => {
                        // Transaction is unknown to the working bank, it might have been
                        // dropped or landed in another fork.  Re-send it
                        info!("Retrying transaction: {}", signature);
                        result.retried += 1;
                        inc_new_counter_info!("send_transaction_service-retry", 1);
                        Self::send_transaction(
                            &send_socket,
                            leader_info
                                .as_ref()
                                .and_then(|leader_info| leader_info.get_leader_tpu())
                                .unwrap_or(&tpu_address),
                            &transaction_info.wire_transaction,
                        );
                        true
                    }
                    Some((_slot, status)) => {
                        if status.is_err() {
                            info!("Dropping failed transaction: {}", signature);
                            result.failed += 1;
                            inc_new_counter_info!("send_transaction_service-failed", 1);
                            false
                        } else {
                            result.retained += 1;
                            true
                        }
                    }
                }
            }
        });

        result
    }

    fn send_transaction(
        send_socket: &UdpSocket,
        tpu_address: &SocketAddr,
        wire_transaction: &[u8],
    ) {
        if let Err(err) = send_socket.send_to(wire_transaction, tpu_address) {
            warn!("Failed to send transaction to {}: {:?}", tpu_address, err);
        }
    }

    pub fn join(self) -> thread::Result<()> {
        self.thread.join()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use solana_sdk::{
        genesis_config::create_genesis_config, pubkey::Pubkey, signature::Signer,
        system_transaction,
    };
    use std::sync::mpsc::channel;

    #[test]
    fn service_exit() {
        let tpu_address = "127.0.0.1:0".parse().unwrap();
        let bank = Bank::default();
        let bank_forks = Arc::new(RwLock::new(BankForks::new(bank)));
        let exit = Arc::new(AtomicBool::new(false));
        let (_sender, receiver) = channel();

        let send_tranaction_service =
            SendTransactionService::new(tpu_address, &bank_forks, None, &exit, receiver);

        exit.store(true, Ordering::Relaxed);
        send_tranaction_service.join().unwrap();
    }

    #[test]
    fn process_transactions() {
        solana_logger::setup();

        let (genesis_config, mint_keypair) = create_genesis_config(4);
        let bank = Bank::new(&genesis_config);
        let bank_forks = Arc::new(RwLock::new(BankForks::new(bank)));
        let send_socket = UdpSocket::bind("0.0.0.0:0").unwrap();
        let tpu_address = "127.0.0.1:0".parse().unwrap();

        let root_bank = Arc::new(Bank::new_from_parent(
            &bank_forks.read().unwrap().working_bank(),
            &Pubkey::default(),
            1,
        ));
        let rooted_signature = root_bank
            .transfer(1, &mint_keypair, &mint_keypair.pubkey())
            .unwrap();

        let working_bank = Arc::new(Bank::new_from_parent(&root_bank, &Pubkey::default(), 2));

        let non_rooted_signature = working_bank
            .transfer(2, &mint_keypair, &mint_keypair.pubkey())
            .unwrap();

        let failed_signature = {
            let blockhash = working_bank.last_blockhash();
            let transaction =
                system_transaction::transfer(&mint_keypair, &Pubkey::default(), 1, blockhash);
            let signature = transaction.signatures[0];
            working_bank.process_transaction(&transaction).unwrap_err();
            signature
        };

        let mut transactions = HashMap::new();

        info!("Expired transactions are dropped..");
        transactions.insert(
            Signature::default(),
            TransactionInfo::new(Signature::default(), vec![], root_bank.slot() - 1),
        );
        let result = SendTransactionService::process_transactions(
            &working_bank,
            &root_bank,
            &send_socket,
            &tpu_address,
            &mut transactions,
            &None,
        );
        assert!(transactions.is_empty());
        assert_eq!(
            result,
            ProcessTransactionsResult {
                expired: 1,
                ..ProcessTransactionsResult::default()
            }
        );

        info!("Rooted transactions are dropped...");
        transactions.insert(
            rooted_signature,
            TransactionInfo::new(rooted_signature, vec![], working_bank.slot()),
        );
        let result = SendTransactionService::process_transactions(
            &working_bank,
            &root_bank,
            &send_socket,
            &tpu_address,
            &mut transactions,
            &None,
        );
        assert!(transactions.is_empty());
        assert_eq!(
            result,
            ProcessTransactionsResult {
                rooted: 1,
                ..ProcessTransactionsResult::default()
            }
        );

        info!("Failed transactions are dropped...");
        transactions.insert(
            failed_signature,
            TransactionInfo::new(failed_signature, vec![], working_bank.slot()),
        );
        let result = SendTransactionService::process_transactions(
            &working_bank,
            &root_bank,
            &send_socket,
            &tpu_address,
            &mut transactions,
            &None,
        );
        assert!(transactions.is_empty());
        assert_eq!(
            result,
            ProcessTransactionsResult {
                failed: 1,
                ..ProcessTransactionsResult::default()
            }
        );

        info!("Non-rooted transactions are kept...");
        transactions.insert(
            non_rooted_signature,
            TransactionInfo::new(non_rooted_signature, vec![], working_bank.slot()),
        );
        let result = SendTransactionService::process_transactions(
            &working_bank,
            &root_bank,
            &send_socket,
            &tpu_address,
            &mut transactions,
            &None,
        );
        assert_eq!(transactions.len(), 1);
        assert_eq!(
            result,
            ProcessTransactionsResult {
                retained: 1,
                ..ProcessTransactionsResult::default()
            }
        );
        transactions.clear();

        info!("Unknown transactions are retried...");
        transactions.insert(
            Signature::default(),
            TransactionInfo::new(Signature::default(), vec![], working_bank.slot()),
        );
        let result = SendTransactionService::process_transactions(
            &working_bank,
            &root_bank,
            &send_socket,
            &tpu_address,
            &mut transactions,
            &None,
        );
        assert_eq!(transactions.len(), 1);
        assert_eq!(
            result,
            ProcessTransactionsResult {
                retried: 1,
                ..ProcessTransactionsResult::default()
            }
        );
    }
}
