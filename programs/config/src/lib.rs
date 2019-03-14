//! Config program

use bincode::deserialize;
use log::*;
use solana_config_api::check_id;
use solana_config_api::config_state::ConfigState;
use solana_sdk::account::KeyedAccount;
use solana_sdk::native_program::ProgramError;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::solana_entrypoint;

fn process_instruction(
    _program_id: &Pubkey,
    keyed_accounts: &mut [KeyedAccount],
    data: &[u8],
) -> Result<(), ProgramError> {
    if !check_id(&keyed_accounts[0].account.owner) {
        error!("account[0] is not assigned to the config program");
        Err(ProgramError::IncorrectProgramId)?;
    }

    if keyed_accounts[0].signer_key().is_none() {
        error!("account[0] should sign the transaction");
        Err(ProgramError::MissingRequiredSignature)?;
    }

    let mut config_state = ConfigState::deserialize(&keyed_accounts[0].account.data)
        .map_err(|_| ProgramError::InvalidAccountData)?;

    let new_config_state = deserialize(data).map_err(|_| ProgramError::InvalidInstructionData)?;
    config_state.store(new_config_state)?;
    config_state.serialize(&mut keyed_accounts[0].account.data)?;

    Ok(())
}

solana_entrypoint!(entrypoint);
fn entrypoint(
    program_id: &Pubkey,
    keyed_accounts: &mut [KeyedAccount],
    data: &[u8],
    _tick_height: u64,
) -> Result<(), ProgramError> {
    solana_logger::setup();

    trace!("process_instruction: {:?}", data);
    trace!("keyed_accounts: {:?}", keyed_accounts);
    process_instruction(program_id, keyed_accounts, data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_config_api::config_instruction::ConfigInstruction;
    use solana_config_api::config_transaction::ConfigTransaction;
    use solana_config_api::id;
    use solana_runtime::runtime;
    use solana_sdk::account::Account;
    use solana_sdk::hash::Hash;
    use solana_sdk::signature::{Keypair, KeypairUtil};
    use solana_sdk::system_instruction::SystemInstruction;
    use solana_sdk::system_program;
    use solana_sdk::transaction::Transaction;
    use solana_sdk::transaction_builder::TransactionBuilder;

    fn create_config_account(data_len: u64, lamports: u64) -> Account {
        let space = ConfigState::max_size(data_len) as usize;
        Account::new(lamports, space, &id())
    }

    fn process_transaction(
        tx: &Transaction,
        tx_accounts: &mut Vec<Account>,
    ) -> Result<(), ProgramError> {
        runtime::process_transaction(tx, tx_accounts, process_instruction)
    }

    #[test]
    fn test_process_create_ok() {
        solana_logger::setup();
        let from_account_keypair = Keypair::new();
        let from_account = Account::new(1, 0, &system_program::id());

        let config_account_keypair = Keypair::new();
        let config_account = Account::new(0, 0, &system_program::id());

        let transaction = ConfigTransaction::new_config(
            &from_account_keypair,
            &config_account_keypair.pubkey(),
            42,
            Hash::default(),
            1,
            0,
        );
        let mut accounts = vec![from_account, config_account];
        process_transaction(&transaction, &mut accounts).unwrap();

        assert_eq!(id(), accounts[1].owner);
        assert_eq!(
            ConfigState::default(),
            ConfigState::deserialize(&accounts[1].data).unwrap()
        );
    }

    #[test]
    fn test_process_store_ok() {
        solana_logger::setup();
        let config_account_keypair = Keypair::new();
        let config_account = create_config_account(42, 1);

        let mut new_config_state = ConfigState::new(42);
        new_config_state.seq = 1;
        new_config_state.data[0] = 1;

        let transaction = ConfigTransaction::new_store(
            &config_account_keypair,
            &new_config_state,
            Hash::default(),
            0,
        );
        let mut accounts = vec![config_account];
        process_transaction(&transaction, &mut accounts).unwrap();

        assert_eq!(
            new_config_state,
            ConfigState::deserialize(&accounts[0].data).unwrap()
        );
    }

    #[test]
    fn test_process_store_fail_account0_invalid_owner() {
        solana_logger::setup();
        let config_account_keypair = Keypair::new();
        let mut config_account = create_config_account(42, 1);
        config_account.owner = Pubkey::default(); // <-- Invalid owner

        let mut new_config_state = ConfigState::new(42);
        new_config_state.seq = 1;

        let transaction = ConfigTransaction::new_store(
            &config_account_keypair,
            &new_config_state,
            Hash::default(),
            0,
        );
        let mut accounts = vec![config_account];
        process_transaction(&transaction, &mut accounts).unwrap_err();
    }

    #[test]
    fn test_process_store_fail_account0_not_signer() {
        solana_logger::setup();
        let system_account_keypair = Keypair::new();
        let system_account = Account::new(42, 0, &system_program::id());

        let config_account_keypair = Keypair::new();
        let config_account = create_config_account(42, 1);

        let mut transaction = TransactionBuilder::default()
            .push(SystemInstruction::new_move(
                &system_account_keypair.pubkey(),
                &Pubkey::default(),
                42,
            ))
            .push(ConfigInstruction::new_store(
                &config_account_keypair.pubkey(),
                &ConfigState::new(42),
            ))
            .compile();

        // Don't sign the transaction with `config_account_keypair`
        transaction.sign(&[&system_account_keypair], Hash::default());
        let mut accounts = vec![system_account, config_account];
        process_transaction(&transaction, &mut accounts).unwrap_err();
    }
}
